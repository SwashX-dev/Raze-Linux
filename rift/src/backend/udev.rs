//! DRM/KMS backend on a real seat (libseat + udev + libinput).
//!
//! Renders with GLES on GBM buffers when the GPU offers hardware EGL, and with pixman
//! on dumb buffers otherwise (or when `RIFT_RENDERER=pixman`), so Rift also runs on
//! virtual GPUs without 3D acceleration.

use std::collections::HashMap;
use std::path::Path;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::dumb::DumbAllocator;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::exporter::ExportFramebuffer;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType};
use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{ImportMemWl, Bind, ExportMem, ImportAll, ImportDma, ImportEgl, ImportMem, Offscreen, Renderer, Texture};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent};
use smithay::input::keyboard::LedState;
use smithay::output::{Mode as WlMode, Output, PhysicalProperties};
use smithay::reexports::calloop::{EventLoop, RegistrationToken};
use smithay::reexports::drm::control::{connector, crtc, ModeTypeFlags};
use smithay::reexports::input::{DeviceCapability, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::DeviceFd;
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};

use super::Backend;
use crate::render::RiftElement;
use crate::state::Rift;

const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

struct Surface<A: Allocator, F: ExportFramebuffer<A::Buffer>>
where
    <F as ExportFramebuffer<A::Buffer>>::Framebuffer: std::fmt::Debug + Send + Sync + 'static,
{
    output: Output,
    global: GlobalId,
    drm_output: DrmOutput<A, F, (), DrmDeviceFd>,
}

struct Gpu<A: Allocator, F: ExportFramebuffer<A::Buffer>, R>
where
    <F as ExportFramebuffer<A::Buffer>>::Framebuffer: std::fmt::Debug + Send + Sync + 'static,
{
    renderer: R,
    manager: DrmOutputManager<A, F, (), DrmDeviceFd>,
    surfaces: HashMap<crtc::Handle, Surface<A, F>>,
}

type GlesGpu = Gpu<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, GlesRenderer>;
type PixmanGpu = Gpu<DumbAllocator, DrmDeviceFd, PixmanRenderer>;

enum Kind {
    Gles(GlesGpu),
    Pixman(PixmanGpu),
}

struct Device {
    node: DrmNode,
    kind: Kind,
    scanner: DrmScanner,
    token: RegistrationToken,
}

pub struct Udev {
    session: LibSeatSession,
    libinput: Libinput,
    primary: DrmNode,
    device: Option<Device>,
    keyboards: Vec<smithay::reexports::input::Device>,
}

impl Udev {
    pub fn seat_name(&self) -> String {
        self.session.seat()
    }

    pub fn renderer_name(&self) -> &'static str {
        match self.device.as_ref().map(|d| &d.kind) {
            Some(Kind::Gles(_)) => "drm-gles",
            Some(Kind::Pixman(_)) => "drm-pixman",
            None => "drm",
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        // Single-GPU: buffers are imported when rendering.
        let _ = surface;
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self.device.as_mut().map(|d| &mut d.kind) {
            Some(Kind::Gles(g)) => g.renderer.import_dmabuf(dmabuf, None).is_ok(),
            Some(Kind::Pixman(g)) => g.renderer.import_dmabuf(dmabuf, None).is_ok(),
            None => false,
        }
    }

    pub fn update_led_state(&mut self, leds: LedState) {
        for kb in self.keyboards.iter_mut() {
            kb.led_update(leds.into());
        }
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            tracing::warn!("cannot switch to VT {vt}: {e}");
        }
    }

    pub fn new(event_loop: &EventLoop<'static, Rift>) -> Result<Udev, String> {
        let (session, notifier) = LibSeatSession::new().map_err(|e| format!("cannot open a seat session: {e}"))?;
        let seat = session.seat();
        let primary = match std::env::var("RIFT_DRM_DEVICE") {
            Ok(path) => DrmNode::from_path(path).map_err(|e| format!("RIFT_DRM_DEVICE: {e}"))?,
            Err(_) => primary_gpu(&seat)
                .ok()
                .flatten()
                .and_then(|p| DrmNode::from_path(p).ok())
                .or_else(|| all_gpus(&seat).ok()?.into_iter().find_map(|p| DrmNode::from_path(p).ok()))
                .ok_or("no GPU found")?,
        };
        let primary = primary.node_with_type(NodeType::Primary).and_then(|n| n.ok()).unwrap_or(primary);
        tracing::info!("primary GPU: {primary}");

        let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
        libinput.udev_assign_seat(&seat).map_err(|_| "libinput: cannot assign seat")?;
        let input_backend = LibinputInputBackend::new(libinput.clone());
        let handle = event_loop.handle();
        handle
            .insert_source(input_backend, |mut event, _, state| {
                if let Backend::Udev(u) = &mut state.backend {
                    match &mut event {
                        InputEvent::DeviceAdded { device } => {
                            if device.has_capability(DeviceCapability::Keyboard) {
                                if let Some(kb) = state.seat.get_keyboard() {
                                    device.led_update(kb.led_state().into());
                                }
                                u.keyboards.push(device.clone());
                            }
                            crate::input::configure_device(&state.config, device);
                        }
                        InputEvent::DeviceRemoved { device } => u.keyboards.retain(|d| d != device),
                        _ => {}
                    }
                }
                state.process_input_event(event);
            })
            .map_err(|e| format!("libinput source: {e}"))?;

        handle
            .insert_source(notifier, |event, _, state| {
                let Backend::Udev(u) = &mut state.backend else { return };
                match event {
                    SessionEvent::PauseSession => {
                        tracing::info!("session paused");
                        u.libinput.suspend();
                        if let Some(d) = &mut u.device {
                            match &mut d.kind {
                                Kind::Gles(g) => g.manager.pause(),
                                Kind::Pixman(g) => g.manager.pause(),
                            }
                        }
                    }
                    SessionEvent::ActivateSession => {
                        tracing::info!("session resumed");
                        if u.libinput.resume().is_err() {
                            tracing::warn!("cannot resume libinput");
                        }
                        if let Some(d) = &mut u.device {
                            let r = match &mut d.kind {
                                Kind::Gles(g) => g.manager.lock().activate(false),
                                Kind::Pixman(g) => g.manager.lock().activate(false),
                            };
                            if let Err(e) = r {
                                tracing::warn!("cannot reactivate DRM device: {e}");
                            }
                        }
                        for o in state.outputs.iter_mut() {
                            o.pending = false;
                            o.redraw = true;
                        }
                    }
                }
            })
            .map_err(|e| format!("session source: {e}"))?;

        Ok(Udev { session, libinput, primary, device: None, keyboards: Vec::new() })
    }

    /// Render one output.
    pub fn render(&mut self, state: &mut Rift, output: &Output) {
        let Some(device) = &mut self.device else { return };
        let Some(crtc) = output.user_data().get::<crtc::Handle>().copied() else { return };
        let queued = match &mut device.kind {
            Kind::Gles(g) => render_gpu(g, state, output, crtc),
            Kind::Pixman(g) => render_gpu(g, state, output, crtc),
        };
        match queued {
            Some(true) => {
                if let Some(o) = state.outputs.iter_mut().find(|o| &o.output == output) {
                    o.pending = true;
                }
                super::post_render(state, output, true);
            }
            Some(false) => {
                super::post_render(state, output, false);
                super::throttle_idle(state, output);
            }
            None => {}
        }
    }

    /// Load a colour-temperature gamma ramp into every CRTC (None restores linear).
    pub fn set_night_light(&mut self, kelvin: Option<u32>) -> bool {
        use smithay::reexports::drm::control::Device as _;
        let Some(device) = &mut self.device else { return false };
        let (drm, crtcs): (&DrmDevice, Vec<crtc::Handle>) = match &device.kind {
            Kind::Gles(g) => (g.manager.device(), g.surfaces.keys().copied().collect()),
            Kind::Pixman(g) => (g.manager.device(), g.surfaces.keys().copied().collect()),
        };
        let (rm, gm, bm) = kelvin.map(temperature_rgb).unwrap_or((1.0, 1.0, 1.0));
        let mut ok = true;
        for crtc in crtcs {
            let size = match drm.get_crtc(crtc) {
                Ok(info) => info.gamma_length() as usize,
                Err(_) => 0,
            };
            if size < 2 {
                ok = false;
                continue;
            }
            let ramp = |m: f64| -> Vec<u16> {
                (0..size).map(|i| ((i as f64 / (size - 1) as f64) * m * 65535.0).round().min(65535.0) as u16).collect()
            };
            if let Err(e) = drm.set_gamma(crtc, &ramp(rm), &ramp(gm), &ramp(bm)) {
                tracing::warn!("cannot set gamma: {e}");
                ok = false;
            }
        }
        ok
    }

    pub fn set_power(&mut self, state: &mut Rift, on: bool) {
        let Some(device) = &mut self.device else { return };
        for o in state.outputs.iter_mut() {
            if o.powered == on {
                continue;
            }
            o.powered = on;
            o.redraw = on;
            if !on {
                let Some(crtc) = o.output.user_data().get::<crtc::Handle>().copied() else { continue };
                let r = match &mut device.kind {
                    Kind::Gles(g) => g.surfaces.get_mut(&crtc).map(|s| s.drm_output.with_compositor(|c| c.clear())),
                    Kind::Pixman(g) => g.surfaces.get_mut(&crtc).map(|s| s.drm_output.with_compositor(|c| c.clear())),
                };
                if let Some(Err(e)) = r {
                    tracing::warn!("cannot power off {}: {e}", o.output.name());
                }
            }
        }
    }
}

/// RGB multipliers for a black-body colour temperature (Tanner Helland's fit), with
/// 6500 K as neutral.
fn temperature_rgb(kelvin: u32) -> (f64, f64, f64) {
    fn channel(k: f64) -> (f64, f64, f64) {
        let t = k / 100.0;
        let r = if t <= 66.0 { 255.0 } else { 329.698727446 * (t - 60.0).powf(-0.1332047592) };
        let g = if t <= 66.0 { 99.4708025861 * t.ln() - 161.1195681661 } else { 288.1221695283 * (t - 60.0).powf(-0.0755148492) };
        let b = if t >= 66.0 { 255.0 } else if t <= 19.0 { 0.0 } else { 138.5177312231 * (t - 10.0).ln() - 305.0447927307 };
        (r.clamp(0.0, 255.0) / 255.0, g.clamp(0.0, 255.0) / 255.0, b.clamp(0.0, 255.0) / 255.0)
    }
    let (r, g, b) = channel(kelvin as f64);
    let (nr, ng, nb) = channel(6500.0);
    ((r / nr).min(1.0), (g / ng).min(1.0), (b / nb).min(1.0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn night_light_is_warm_and_neutral_at_6500k() {
        let (r, g, b) = super::temperature_rgb(6500);
        assert!((r - 1.0).abs() < 1e-9 && (g - 1.0).abs() < 1e-9 && (b - 1.0).abs() < 1e-9);
        let (r, g, b) = super::temperature_rgb(3500);
        assert!(r > g && g > b && b < 0.75, "{r} {g} {b}");
    }
}

/// Render `output` on `gpu`. `Some(true)` if a frame was queued, `Some(false)` if
/// nothing changed, `None` on error.
fn render_gpu<A, F, R>(gpu: &mut Gpu<A, F, R>, state: &mut Rift, output: &Output, crtc: crtc::Handle) -> Option<bool>
where
    A: Allocator + Clone + std::fmt::Debug + 'static,
    A::Buffer: smithay::backend::allocator::dmabuf::AsDmabuf,
    A::Error: Send + Sync + 'static,
    <A::Buffer as smithay::backend::allocator::dmabuf::AsDmabuf>::Error: Send + Sync + 'static + std::fmt::Debug,
    F: ExportFramebuffer<A::Buffer> + Clone,
    <F as ExportFramebuffer<A::Buffer>>::Framebuffer: std::fmt::Debug + Send + Sync + 'static,
    <F as ExportFramebuffer<A::Buffer>>::Error: Send + Sync + 'static,
    R: Renderer + ImportAll + ImportMem + Bind<Dmabuf> + ExportMem,
    R: crate::screencopy::CaptureRenderer,
    R::TextureId: Texture + Clone + Send + 'static,
    R::Error: Send + Sync + 'static,
{
    let surface = gpu.surfaces.get_mut(&crtc)?;
    let powered = state.output_state(output).map(|o| o.powered).unwrap_or(true);
    if !powered {
        return None;
    }
    let elements = crate::render::output_elements(state, &mut gpu.renderer, output, true);
    let clear = crate::render::background_color(state);
    let result = surface.drm_output.render_frame(&mut gpu.renderer, &elements, clear, FrameFlags::DEFAULT);
    let queued = match result {
        Ok(r) if !r.is_empty => match surface.drm_output.queue_frame(()) {
            Ok(()) => Some(true),
            Err(e) => {
                tracing::warn!("queue_frame on {}: {e:?}", output.name());
                None
            }
        },
        Ok(_) => Some(false),
        Err(e) => {
            tracing::warn!("render_frame on {}: {e:?}", output.name());
            None
        }
    };
    drop(elements);
    crate::screencopy::process(state, &mut gpu.renderer, output);
    queued
}

fn on_vblank(state: &mut Rift, crtc: crtc::Handle) {
    let Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = &mut u.device else { return };
    let r = match &mut device.kind {
        Kind::Gles(g) => g.surfaces.get_mut(&crtc).map(|s| s.drm_output.frame_submitted().map(|_| ()).map_err(|e| format!("{e:?}"))),
        Kind::Pixman(g) => g.surfaces.get_mut(&crtc).map(|s| s.drm_output.frame_submitted().map(|_| ()).map_err(|e| format!("{e:?}"))),
    };
    if let Some(Err(e)) = r {
        tracing::debug!("frame_submitted: {e}");
    }
    if let Some(o) = state.outputs.iter_mut().find(|o| o.output.user_data().get::<crtc::Handle>() == Some(&crtc)) {
        o.pending = false;
    }
}

/// Open the primary GPU and light up its connectors.
pub fn init(state: &mut Rift) -> Result<(), String> {
    let Backend::Udev(u) = &mut state.backend else { return Ok(()) };
    let seat = u.session.seat();
    let udev = UdevBackend::new(&seat).map_err(|e| format!("udev: {e}"))?;
    let primary = u.primary;
    let path = udev
        .device_list()
        .find(|(id, _)| *id == primary.dev_id())
        .map(|(_, p)| p.to_path_buf())
        .ok_or_else(|| format!("primary GPU {primary} not in the udev device list"))?;
    add_device(state, primary, &path)?;

    state
        .loop_handle
        .clone()
        .insert_source(udev, move |event, _, state| match event {
            UdevEvent::Changed { device_id } if device_id == primary.dev_id() => scan_connectors(state),
            UdevEvent::Added { .. } | UdevEvent::Removed { .. } | UdevEvent::Changed { .. } => {}
        })
        .map_err(|e| format!("udev source: {e}"))?;
    Ok(())
}

fn add_device(state: &mut Rift, node: DrmNode, path: &Path) -> Result<(), String> {
    let Backend::Udev(u) = &mut state.backend else { return Ok(()) };
    let fd = u
        .session
        .open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, notifier) = DrmDevice::new(fd.clone(), true).map_err(|e| format!("DRM device: {e}"))?;
    let token = state
        .loop_handle
        .insert_source(notifier, |event, _, state| match event {
            DrmEvent::VBlank(crtc) => on_vblank(state, crtc),
            DrmEvent::Error(e) => tracing::error!("DRM error: {e}"),
        })
        .map_err(|e| format!("DRM source: {e}"))?;

    let force_pixman = std::env::var("RIFT_RENDERER").map(|v| v == "pixman").unwrap_or(false);
    let gbm = GbmDevice::new(fd.clone()).map_err(|e| format!("GBM: {e}"))?;
    let gles = if force_pixman {
        Err("RIFT_RENDERER=pixman".to_string())
    } else {
        try_gles(&gbm)
    };
    let kind = match gles {
        Ok(mut renderer) => {
            tracing::info!("renderer: GLES (hardware)");
            if let Err(e) = renderer.bind_wl_display(&state.display_handle) {
                tracing::debug!("EGL wl_display binding unavailable: {e}");
            }
            let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
            let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
            let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(node).into());
            let manager = DrmOutputManager::new(drm, allocator, exporter, Some(gbm), COLOR_FORMATS, render_formats);
            let dmabuf_formats = renderer.dmabuf_formats();
            state.shm_state.update_formats(renderer.shm_formats().collect::<Vec<_>>());
            state.dmabuf_state.create_global::<Rift>(&state.display_handle, dmabuf_formats);
            Kind::Gles(Gpu { renderer, manager, surfaces: HashMap::new() })
        }
        Err(why) => {
            tracing::info!("renderer: pixman (software) — {why}");
            let renderer = PixmanRenderer::new().map_err(|e| format!("pixman: {e:?}"))?;
            let render_formats = Bind::<Dmabuf>::supported_formats(&renderer).unwrap_or_default();
            let allocator = DumbAllocator::new(fd.clone());
            let manager = DrmOutputManager::new(drm, allocator, fd.clone(), None::<GbmDevice<DrmDeviceFd>>, COLOR_FORMATS, render_formats);
            state.shm_state.update_formats(renderer.shm_formats().collect::<Vec<_>>());
            Kind::Pixman(Gpu { renderer, manager, surfaces: HashMap::new() })
        }
    };
    let Backend::Udev(u) = &mut state.backend else { return Ok(()) };
    u.device = Some(Device { node, kind, scanner: DrmScanner::new(), token });
    scan_connectors(state);
    Ok(())
}

fn try_gles(gbm: &GbmDevice<DrmDeviceFd>) -> Result<GlesRenderer, String> {
    // SAFETY: the GBM device outlives the EGL display (both live in the backend).
    let display = unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| format!("EGL display: {e}"))?;
    let device = EGLDevice::device_for_display(&display).map_err(|e| format!("EGL device: {e}"))?;
    if device.is_software() && std::env::var("RIFT_RENDERER").map(|v| v != "gles").unwrap_or(true) {
        return Err("EGL is software-only (llvmpipe)".into());
    }
    let context = EGLContext::new(&display).map_err(|e| format!("EGL context: {e}"))?;
    // SAFETY: the context is current only on this thread.
    unsafe { GlesRenderer::new(context) }.map_err(|e| format!("GLES: {e}"))
}

fn scan_connectors(state: &mut Rift) {
    let Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = &mut u.device else { return };
    let drm = match &device.kind {
        Kind::Gles(g) => g.manager.device(),
        Kind::Pixman(g) => g.manager.device(),
    };
    let events = match device.scanner.scan_connectors(drm) {
        Ok(r) => r.into_iter().collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!("connector scan failed: {e}");
            return;
        }
    };
    for event in events {
        match event {
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } => connector_connected(state, connector, crtc),
            DrmScanEvent::Disconnected { crtc: Some(crtc), .. } => connector_disconnected(state, crtc),
            _ => {}
        }
    }
}

fn pick_mode(state: &Rift, name: &str, connector: &connector::Info) -> Option<smithay::reexports::drm::control::Mode> {
    let modes = connector.modes();
    let wanted = state.config.outputs.get(name).and_then(|c| c.mode.as_deref()).and_then(crate::config::parse_mode);
    if let Some((w, h, hz)) = wanted {
        let found = modes
            .iter()
            .filter(|m| m.size() == (w as u16, h as u16))
            .filter(|m| hz.map(|hz| (m.vrefresh() as i64 - hz as i64).abs() <= 1).unwrap_or(true))
            .max_by_key(|m| m.vrefresh());
        if found.is_some() {
            return found.copied();
        }
        tracing::warn!("{name}: mode {w}x{h} not available, using the preferred mode");
    }
    modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or(modes.first())
        .copied()
}

fn connector_connected(state: &mut Rift, connector: connector::Info, crtc: crtc::Handle) {
    let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
    if state.config.outputs.get(&name).map(|c| c.disabled).unwrap_or(false) {
        tracing::info!("{name}: disabled in the configuration");
        return;
    }
    let Some(mode) = pick_mode(state, &name, &connector) else {
        tracing::warn!("{name}: no modes");
        return;
    };
    let (pw, ph) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        name.clone(),
        PhysicalProperties {
            size: (pw as i32, ph as i32).into(),
            subpixel: connector.subpixel().into(),
            make: "Unknown".into(),
            model: "Unknown".into(),
            serial_number: "Unknown".into(),
        },
    );
    let wl_mode = WlMode::from(mode);
    output.set_preferred(wl_mode);
    output.change_current_state(Some(wl_mode), None, None, None);
    output.user_data().insert_if_missing(|| crtc);
    let global = output.create_global::<Rift>(&state.display_handle);

    let Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = &mut u.device else { return };
    let ok = match &mut device.kind {
        Kind::Gles(g) => init_surface(g, &output, global.clone(), crtc, mode, connector.handle()),
        Kind::Pixman(g) => init_surface(g, &output, global.clone(), crtc, mode, connector.handle()),
    };
    if ok {
        tracing::info!("{name}: {}x{}@{}", mode.size().0, mode.size().1, mode.vrefresh());
        state.add_output(output);
    } else {
        state.display_handle.remove_global::<Rift>(global);
    }
}

fn init_surface<A, F, R>(
    gpu: &mut Gpu<A, F, R>,
    output: &Output,
    global: GlobalId,
    crtc: crtc::Handle,
    mode: smithay::reexports::drm::control::Mode,
    connector: connector::Handle,
) -> bool
where
    A: Allocator + Clone + std::fmt::Debug + 'static,
    A::Buffer: smithay::backend::allocator::dmabuf::AsDmabuf,
    A::Error: Send + Sync + 'static,
    <A::Buffer as smithay::backend::allocator::dmabuf::AsDmabuf>::Error: Send + Sync + 'static,
    F: ExportFramebuffer<A::Buffer> + Clone,
    <F as ExportFramebuffer<A::Buffer>>::Framebuffer: std::fmt::Debug + Send + Sync + 'static,
    <F as ExportFramebuffer<A::Buffer>>::Error: Send + Sync + 'static,
    R: Renderer + ImportAll + ImportMem + Bind<Dmabuf>,
    R::TextureId: Texture + Clone + Send + 'static,
    R::Error: Send + Sync + 'static,
{
    let result = gpu.manager.lock().initialize_output::<_, RiftElement<R>>(
        crtc,
        mode,
        &[connector],
        output,
        None,
        &mut gpu.renderer,
        &DrmOutputRenderElements::default(),
    );
    match result {
        Ok(drm_output) => {
            gpu.surfaces.insert(crtc, Surface { output: output.clone(), global, drm_output });
            true
        }
        Err(e) => {
            tracing::warn!("cannot initialise {}: {e:?}", output.name());
            false
        }
    }
}

fn connector_disconnected(state: &mut Rift, crtc: crtc::Handle) {
    let Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = &mut u.device else { return };
    let removed = match &mut device.kind {
        Kind::Gles(g) => g.surfaces.remove(&crtc).map(|s| (s.output, s.global)),
        Kind::Pixman(g) => g.surfaces.remove(&crtc).map(|s| (s.output, s.global)),
    };
    if let Some((output, global)) = removed {
        tracing::info!("{}: disconnected", output.name());
        state.remove_output(&output);
        state.display_handle.remove_global::<Rift>(global);
    }
}

#[allow(dead_code)]
fn unused(d: &Device) -> (DrmNode, &RegistrationToken) {
    (d.node, &d.token)
}
