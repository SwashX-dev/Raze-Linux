//! Nested backend: runs Rift in a window of another compositor (development).

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportMemWl, ImportDma, ImportMem};
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::utils::Transform;

use super::Backend;
use crate::state::Rift;

pub struct Winit {
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: Option<OutputDamageTracker>,
    output: Option<Output>,
}

impl Winit {
    pub fn new(event_loop: &EventLoop<'static, Rift>) -> Result<Winit, String> {
        let (backend, winit) = winit::init::<GlesRenderer>().map_err(|e| format!("winit: {e}"))?;
        event_loop
            .handle()
            .insert_source(winit, |event, _, state| match event {
                WinitEvent::Resized { size, .. } => {
                    if let Some(output) = state.outputs.first().map(|o| o.output.clone()) {
                        let mode = Mode { size, refresh: 60_000 };
                        output.change_current_state(Some(mode), None, None, None);
                        output.set_preferred(mode);
                        if let Backend::Winit(w) = &mut state.backend {
                            w.damage_tracker = None;
                        }
                        state.output_changed(&output);
                    }
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => state.request_redraw_all(),
                WinitEvent::CloseRequested => state.loop_signal.stop(),
                WinitEvent::Focus(_) => {}
            })
            .map_err(|e| format!("winit event source: {e}"))?;
        Ok(Winit { backend, damage_tracker: None, output: None })
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.backend.renderer().import_dmabuf(dmabuf, None).is_ok()
    }

    pub fn render(&mut self, state: &mut Rift, output: &Output) {
        let age = if self.damage_tracker.is_none() { 0 } else { self.backend.buffer_age().unwrap_or(0) };
        let tracker = self.damage_tracker.get_or_insert_with(|| OutputDamageTracker::from_output(output));
        let clear = crate::render::background_color(state);
        let result = match self.backend.bind() {
            Ok((renderer, mut fb)) => {
                let elements = crate::render::output_elements(state, renderer, output, true);
                tracker.render_output(renderer, &mut fb, age, &elements, clear).map(|r| r.damage.cloned()).map_err(|e| format!("{e:?}"))
            }
            Err(e) => Err(format!("{e:?}")),
        };
        let rendered = match result {
            Ok(Some(damage)) => {
                if let Err(e) = self.backend.submit(Some(&damage)) {
                    tracing::warn!("winit submit failed: {e}");
                }
                true
            }
            Ok(None) => false,
            Err(e) => {
                tracing::warn!("winit render failed: {e}");
                false
            }
        };
        crate::screencopy::process(state, self.backend.renderer(), output, rendered);
        self.backend.window().set_cursor_visible(false);
        super::post_render(state, output, rendered);
        super::throttle_idle(state, output);
    }
}

pub fn init(state: &mut Rift) {
    let Backend::Winit(w) = &mut state.backend else { return };
    let size = w.backend.window_size();
    let formats: Vec<_> = w.backend.renderer().shm_formats().collect();
    let dmabuf_formats = w.backend.renderer().dmabuf_formats();
    let mode = Mode { size, refresh: 60_000 };
    let output = Output::new(
        "WINIT-1".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Rift".into(),
            model: "Nested".into(),
            serial_number: "0".into(),
        },
    );
    output.create_global::<Rift>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, None);
    output.set_preferred(mode);
    w.output = Some(output.clone());
    state.shm_state.update_formats(formats);
    state.dmabuf_state.create_global::<Rift>(&state.display_handle, dmabuf_formats);
    state.add_output(output);
}
