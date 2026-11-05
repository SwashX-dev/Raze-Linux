//! Output capture into client shm buffers, for two protocols:
//! ext-image-copy-capture-v1 (grim) and wlr-screencopy-unstable-v1 (wf-recorder and
//! other recorders).
//!
//! Captures are served when the output is repainted: its elements are rendered again
//! into an offscreen buffer, read back and copied into the client buffer. wlr
//! `copy_with_damage` requests wait for a repaint that actually changed something, so
//! recording a static screen costs nothing.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::{GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{
    Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer, Texture,
};
use smithay::output::{Output, WeakOutput};
use smithay::reexports::pixman::Image;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::Buffer;
use smithay::utils::{Physical, Rectangle, Size};
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, Frame, SessionRef,
};
use smithay::wayland::shm::with_buffer_contents_mut;

use crate::render::RiftElement;
use crate::state::Rift;

pub struct PendingCapture {
    pub session: SessionRef,
    pub frame: Frame,
    pub output: WeakOutput,
}

/// A wlr-screencopy frame: what to capture (region in buffer pixels of the output's
/// mode, i.e. untransformed).
pub struct WlrFrameData {
    output: Option<WeakOutput>,
    region: Rectangle<i32, Buffer>,
    cursor: bool,
    used: std::sync::atomic::AtomicBool,
}

/// A wlr-screencopy `copy` waiting for a repaint.
pub struct PendingWlrCopy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    with_damage: bool,
}

/// Monotonic clock, as the presentation timestamps of both protocols expect.
fn monotonic_now() -> std::time::Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for Rift {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Rift {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_manager_v1::Request;
        let (frame, cursor, wl_output, logical) = match request {
            Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor != 0, output, None),
            Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                overlay_cursor != 0,
                output,
                Some(Rectangle::<i32, smithay::utils::Logical>::new(
                    (x, y).into(),
                    (width, height).into(),
                )),
            ),
            Request::Destroy => return,
            _ => return,
        };
        let output = Output::from_resource(&wl_output);
        let region = output.as_ref().and_then(|o| {
            let mode = o.current_mode()?;
            let full = Rectangle::<i32, Buffer>::from_size((mode.size.w, mode.size.h).into());
            match logical {
                None => Some(full),
                Some(r) => {
                    let scale = o.current_scale().fractional_scale();
                    let transform = o.current_transform();
                    let logical_size = transform
                        .transform_size(mode.size)
                        .to_f64()
                        .to_logical(scale);
                    let r = r
                        .to_f64()
                        .to_buffer(scale, transform, &logical_size)
                        .to_i32_round();
                    r.intersection(full).filter(|r| !r.is_empty())
                }
            }
        });
        let data = WlrFrameData {
            output: output.as_ref().map(|o| o.downgrade()),
            region: region.unwrap_or_default(),
            cursor,
            used: Default::default(),
        };
        let frame = data_init.init(frame, data);
        match region {
            Some(r) => {
                frame.buffer(
                    wl_shm::Format::Xrgb8888,
                    r.size.w as u32,
                    r.size.h as u32,
                    r.size.w as u32 * 4,
                );
                if frame.version() >= 3 {
                    frame.buffer_done();
                }
            }
            None => frame.failed(),
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, WlrFrameData> for Rift {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &WlrFrameData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_frame_v1::Request;
        let (buffer, with_damage) = match request {
            Request::Copy { buffer } => (buffer, false),
            Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        if data.used.swap(true, std::sync::atomic::Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "frame already used",
            );
            return;
        }
        let Some(output) = data.output.as_ref().and_then(|o| o.upgrade()) else {
            frame.failed();
            return;
        };
        // Like wlroots, each client's damage starts as the whole output: its first
        // copy_with_damage is served right away, later ones wait for real changes.
        let key = (frame.client().map(|c| c.id()), output.name());
        let first = state.wlr_copy_clients.insert(key);
        let with_damage = with_damage && !first;
        state.pending_wlr_copies.push(PendingWlrCopy {
            frame: frame.clone(),
            buffer,
            with_damage,
        });
        if !with_damage {
            state.request_redraw(&output);
        } else {
            // Keep-alive: on a static screen, serve the frame after a second anyway, so
            // recorders get at least 1 frame/s and can stop cleanly.
            let frame = frame.clone();
            let _ = state.loop_handle.insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(
                    std::time::Duration::from_secs(1),
                ),
                move |_, _, state| {
                    if let Some(c) = state
                        .pending_wlr_copies
                        .iter_mut()
                        .find(|c| c.frame == frame)
                    {
                        c.with_damage = false;
                        if let Some(o) = frame
                            .data::<WlrFrameData>()
                            .and_then(|d| d.output.as_ref())
                            .and_then(|o| o.upgrade())
                        {
                            state.request_redraw(&o);
                        }
                    }
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                },
            );
        }
    }
}

/// Copy `pixels` (XRGB/ARGB8888, `width` pixels per row) region `r` into an shm buffer.
fn copy_into_shm(
    buffer: &WlBuffer,
    pixels: &[u8],
    width: usize,
    r: Rectangle<i32, Buffer>,
) -> bool {
    let (w, h) = (r.size.w as usize, r.size.h as usize);
    let (x0, y0) = (r.loc.x as usize, r.loc.y as usize);
    matches!(
        with_buffer_contents_mut(buffer, |ptr, len, data| {
            if data.width as usize != w || data.height as usize != h {
                return false;
            }
            let stride = data.stride as usize;
            let offset = data.offset as usize;
            if stride < w * 4 || offset + stride * h > len {
                return false;
            }
            // SAFETY: bounds checked above; the pool mapping stays valid during the call.
            let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), stride * h) };
            for row in 0..h {
                let src = ((y0 + row) * width + x0) * 4;
                dst[row * stride..row * stride + w * 4].copy_from_slice(&pixels[src..src + w * 4]);
            }
            true
        }),
        Ok(true)
    )
}

pub fn constraints(source: &ImageCaptureSource) -> Option<BufferConstraints> {
    let output = source.user_data().get::<WeakOutput>()?.upgrade()?;
    let mode = output.current_mode()?;
    Some(BufferConstraints {
        size: (mode.size.w, mode.size.h).into(),
        shm: vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888],
        dma: None,
    })
}

pub fn queue_frame(state: &mut Rift, session: &SessionRef, frame: Frame) {
    let Some(weak) = session.source().user_data().get::<WeakOutput>().cloned() else {
        frame.fail(CaptureFailureReason::Unknown);
        return;
    };
    let Some(output) = weak.upgrade() else {
        frame.fail(CaptureFailureReason::Stopped);
        return;
    };
    state.pending_captures.push(PendingCapture {
        session: session.clone(),
        frame,
        output: weak,
    });
    state.request_redraw(&output);
}

/// Renderers that can draw into an offscreen buffer and read it back.
pub trait CaptureRenderer: Renderer + ImportAll + ImportMem + Sized
where
    Self::TextureId: Clone + Send + 'static,
{
    fn capture(
        &mut self,
        size: Size<i32, Physical>,
        output: &Output,
        elements: &[RiftElement<Self>],
        clear: [f32; 4],
    ) -> Result<Vec<u8>, String>;
}

fn capture_with<R, T>(
    renderer: &mut R,
    size: Size<i32, Physical>,
    output: &Output,
    elements: &[RiftElement<R>],
    clear: [f32; 4],
) -> Result<Vec<u8>, String>
where
    R: Renderer + ImportAll + ImportMem + ExportMem + Offscreen<T> + Bind<T>,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let mut target: T = renderer
        .create_buffer(Fourcc::Argb8888, (size.w, size.h).into())
        .map_err(|e| format!("offscreen buffer: {e:?}"))?;
    let mut fb = renderer
        .bind(&mut target)
        .map_err(|e| format!("bind: {e:?}"))?;
    let scale = output.current_scale().fractional_scale();
    let mut tracker = OutputDamageTracker::new(size, scale, output.current_transform());
    tracker
        .render_output(renderer, &mut fb, 0, elements, clear)
        .map_err(|e| format!("render: {e:?}"))?;
    let mapping = renderer
        .copy_framebuffer(
            &fb,
            Rectangle::from_size((size.w, size.h).into()),
            Fourcc::Argb8888,
        )
        .map_err(|e| format!("read back: {e:?}"))?;
    drop(fb);
    let data = renderer
        .map_texture(&mapping)
        .map_err(|e| format!("map: {e:?}"))?;
    Ok(data.to_vec())
}

impl CaptureRenderer for GlesRenderer {
    fn capture(
        &mut self,
        size: Size<i32, Physical>,
        output: &Output,
        elements: &[RiftElement<Self>],
        clear: [f32; 4],
    ) -> Result<Vec<u8>, String> {
        capture_with::<_, GlesRenderbuffer>(self, size, output, elements, clear)
    }
}

impl CaptureRenderer for PixmanRenderer {
    fn capture(
        &mut self,
        size: Size<i32, Physical>,
        output: &Output,
        elements: &[RiftElement<Self>],
        clear: [f32; 4],
    ) -> Result<Vec<u8>, String> {
        capture_with::<_, Image<'static, 'static>>(self, size, output, elements, clear)
    }
}

/// Serve the pending captures of `output` (called by backends after drawing it).
/// `damaged`: whether this repaint changed the output.
pub fn process<R>(state: &mut Rift, renderer: &mut R, output: &Output, damaged: bool)
where
    R: CaptureRenderer,
    R::TextureId: Clone + Send + 'static,
{
    if state.pending_captures.is_empty() && state.pending_wlr_copies.is_empty() {
        return;
    }
    let (ext, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending_captures)
        .into_iter()
        .partition(|c| c.output.upgrade().as_ref() == Some(output));
    state.pending_captures = rest;
    let (wlr, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending_wlr_copies)
        .into_iter()
        .partition(|c| {
            let data = c.frame.data::<WlrFrameData>();
            let mine = data
                .and_then(|d| d.output.as_ref())
                .and_then(|o| o.upgrade())
                .as_ref()
                == Some(output);
            mine && (damaged || !c.with_damage)
        });
    state.pending_wlr_copies = rest;
    if ext.is_empty() && wlr.is_empty() {
        return;
    }
    let Some(mode) = output.current_mode() else {
        return;
    };
    let clear = crate::render::background_color(state);
    let width = mode.size.w as usize;
    let full = Rectangle::<i32, Buffer>::from_size((mode.size.w, mode.size.h).into());
    // Render at most twice (with and without the cursor).
    let mut images: [Option<Result<Vec<u8>, String>>; 2] = [None, None];
    let mut image = |state: &Rift, renderer: &mut R, cursor: bool| -> Result<Vec<u8>, String> {
        let slot = &mut images[cursor as usize];
        if slot.is_none() {
            let elements = crate::render::output_elements(state, renderer, output, cursor);
            *slot = Some(renderer.capture(mode.size, output, &elements, clear));
        }
        slot.clone().unwrap()
    };
    let now = monotonic_now();

    for capture in ext {
        match image(state, renderer, capture.session.draw_cursor()) {
            Ok(pixels) if copy_into_shm(&capture.frame.buffer(), &pixels, width, full) => {
                capture.frame.success(output.current_transform(), None, now);
            }
            Ok(_) => capture.frame.fail(CaptureFailureReason::BufferConstraints),
            Err(e) => {
                tracing::warn!("screencopy failed: {e}");
                capture.frame.fail(CaptureFailureReason::Unknown);
            }
        }
    }

    for copy in wlr {
        let Some(data) = copy.frame.data::<WlrFrameData>() else {
            continue;
        };
        let pixels = match image(state, renderer, data.cursor) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("screencopy failed: {e}");
                copy.frame.failed();
                continue;
            }
        };
        if !copy_into_shm(&copy.buffer, &pixels, width, data.region) {
            copy.frame.failed();
            continue;
        }
        copy.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
        if copy.with_damage {
            copy.frame
                .damage(0, 0, data.region.size.w as u32, data.region.size.h as u32);
        }
        let secs = now.as_secs();
        copy.frame
            .ready((secs >> 32) as u32, secs as u32, now.subsec_nanos());
    }
}
