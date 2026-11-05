//! Output backends: DRM/KMS (udev), nested (winit) and headless (pixman, for tests).

pub mod headless;
pub mod udev;
pub mod winit;

use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::input::keyboard::LedState;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::state::Rift;

pub enum Backend {
    /// Placeholder while a backend is temporarily taken out of the state for rendering.
    Empty,
    Headless(Box<headless::Headless>),
    Winit(Box<winit::Winit>),
    Udev(Box<udev::Udev>),
}

impl Backend {
    pub fn seat_name(&self) -> String {
        match self {
            Backend::Udev(u) => u.seat_name(),
            _ => "seat0".to_string(),
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Backend::Udev(u) = self {
            u.early_import(surface);
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Backend::Udev(u) => u.import_dmabuf(dmabuf),
            Backend::Winit(w) => w.import_dmabuf(dmabuf),
            _ => false,
        }
    }

    pub fn update_led_state(&mut self, leds: LedState) {
        if let Backend::Udev(u) = self {
            u.update_led_state(leds);
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Backend::Empty => "none",
            Backend::Headless(_) => "headless",
            Backend::Winit(_) => "winit",
            Backend::Udev(u) => u.renderer_name(),
        }
    }
}

/// Render every output that asked for a redraw and is not waiting for a vblank.
pub fn render_pending(state: &mut Rift) {
    let outputs: Vec<Output> = state
        .outputs
        .iter()
        .filter(|o| (o.redraw || !state.pending_captures.is_empty()) && !o.pending)
        .map(|o| o.output.clone())
        .collect();
    if outputs.is_empty() {
        return;
    }
    let mut backend = std::mem::replace(&mut state.backend, Backend::Empty);
    for output in outputs {
        if let Some(o) = state.outputs.iter_mut().find(|o| o.output == output) {
            o.redraw = false;
        }
        match &mut backend {
            Backend::Headless(b) => b.render(state, &output),
            Backend::Winit(b) => b.render(state, &output),
            Backend::Udev(b) => b.render(state, &output),
            Backend::Empty => {}
        }
    }
    state.backend = backend;
}

/// Common work after an output was drawn (or found undamaged).
pub fn post_render(state: &mut Rift, output: &Output, presented: bool) {
    state.send_frames(output);
    if state.locked {
        if let Some(o) = state.outputs.iter_mut().find(|o| &o.output == output) {
            o.locked_frame = true;
        }
        if state.outputs.iter().all(|o| o.locked_frame || !o.powered) {
            if let Some(locker) = state.pending_lock.take() {
                locker.lock();
            }
        }
    }
    let _ = presented;
}

/// When nothing was drawn there is no vblank to wait for; pace frame callbacks with a timer.
pub fn throttle_idle(state: &mut Rift, output: &Output) {
    let refresh = output
        .current_mode()
        .map(|m| m.refresh)
        .filter(|r| *r > 0)
        .unwrap_or(60_000);
    let frame = Duration::from_micros(1_000_000_000 / refresh as u64);
    if let Some(o) = state.outputs.iter_mut().find(|o| &o.output == output) {
        o.pending = true;
    }
    let output = output.clone();
    let _ = state.loop_handle.insert_source(
        smithay::reexports::calloop::timer::Timer::from_duration(frame),
        move |_, _, state| {
            if let Some(o) = state.outputs.iter_mut().find(|o| o.output == output) {
                o.pending = false;
            }
            state.send_frames(&output);
            smithay::reexports::calloop::timer::TimeoutAction::Drop
        },
    );
}
