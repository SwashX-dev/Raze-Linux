//! ext-image-copy-capture-v1: output capture into client shm buffers.
//!
//! Captures are served on the next repaint of the output: the output's elements are
//! rendered again into an offscreen buffer, read back and copied into the client buffer.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::{GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer, Texture};
use smithay::output::{Output, WeakOutput};
use smithay::reexports::pixman::Image;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{Physical, Rectangle, Size};
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::image_copy_capture::{BufferConstraints, CaptureFailureReason, Frame, SessionRef};
use smithay::wayland::shm::with_buffer_contents_mut;

use crate::render::RiftElement;
use crate::state::Rift;

pub struct PendingCapture {
    pub session: SessionRef,
    pub frame: Frame,
    pub output: WeakOutput,
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
    state.pending_captures.push(PendingCapture { session: session.clone(), frame, output: weak });
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
    let mut fb = renderer.bind(&mut target).map_err(|e| format!("bind: {e:?}"))?;
    let scale = output.current_scale().fractional_scale();
    let mut tracker = OutputDamageTracker::new(size, scale, output.current_transform());
    tracker
        .render_output(renderer, &mut fb, 0, elements, clear)
        .map_err(|e| format!("render: {e:?}"))?;
    let mapping = renderer
        .copy_framebuffer(&fb, Rectangle::from_size((size.w, size.h).into()), Fourcc::Argb8888)
        .map_err(|e| format!("read back: {e:?}"))?;
    drop(fb);
    let data = renderer.map_texture(&mapping).map_err(|e| format!("map: {e:?}"))?;
    Ok(data.to_vec())
}

impl CaptureRenderer for GlesRenderer {
    fn capture(&mut self, size: Size<i32, Physical>, output: &Output, elements: &[RiftElement<Self>], clear: [f32; 4]) -> Result<Vec<u8>, String> {
        capture_with::<_, GlesRenderbuffer>(self, size, output, elements, clear)
    }
}

impl CaptureRenderer for PixmanRenderer {
    fn capture(&mut self, size: Size<i32, Physical>, output: &Output, elements: &[RiftElement<Self>], clear: [f32; 4]) -> Result<Vec<u8>, String> {
        capture_with::<_, Image<'static, 'static>>(self, size, output, elements, clear)
    }
}

/// Serve the pending captures of `output` (called by backends after drawing it).
pub fn process<R>(state: &mut Rift, renderer: &mut R, output: &Output)
where
    R: CaptureRenderer,
    R::TextureId: Clone + Send + 'static,
{
    if state.pending_captures.is_empty() {
        return;
    }
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending_captures)
        .into_iter()
        .partition(|c| c.output.upgrade().as_ref() == Some(output));
    state.pending_captures = rest;
    let Some(mode) = output.current_mode() else { return };
    let clear = crate::render::background_color(state);
    for capture in mine {
        let elements = crate::render::output_elements(state, renderer, output, capture.session.draw_cursor());
        let pixels = match renderer.capture(mode.size, output, &elements, clear) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("screencopy failed: {e}");
                capture.frame.fail(CaptureFailureReason::Unknown);
                continue;
            }
        };
        let buffer = capture.frame.buffer();
        let (w, h) = (mode.size.w as usize, mode.size.h as usize);
        let copied = with_buffer_contents_mut(&buffer, |ptr, len, data| {
            if data.width as usize != w || data.height as usize != h {
                return false;
            }
            let stride = data.stride as usize;
            let offset = data.offset as usize;
            if offset + stride * h > len {
                return false;
            }
            // SAFETY: bounds checked above; the pool mapping stays valid during the call.
            let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), stride * h) };
            for row in 0..h {
                dst[row * stride..row * stride + w * 4].copy_from_slice(&pixels[row * w * 4..(row + 1) * w * 4]);
            }
            true
        });
        match copied {
            Ok(true) => {
                let now = state.start_time.elapsed();
                capture.frame.success(output.current_transform(), None, now);
            }
            _ => capture.frame.fail(CaptureFailureReason::BufferConstraints),
        }
    }
}
