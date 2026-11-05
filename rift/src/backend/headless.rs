//! Headless backend: renders with pixman into memory. Used for tests and CI.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ImportMemWl, Offscreen};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::pixman::Image;
use smithay::utils::Transform;

use super::Backend;
use crate::state::Rift;

struct Target {
    buffer: Image<'static, 'static>,
    damage_tracker: OutputDamageTracker,
}

pub struct Headless {
    pub renderer: PixmanRenderer,
    size: (i32, i32),
    targets: HashMap<String, Target>,
    /// Frames with damage rendered so far (exposed over IPC for tests).
    pub frames: u64,
}

impl Headless {
    pub fn new(size: (i32, i32)) -> Result<Headless, String> {
        let renderer = PixmanRenderer::new().map_err(|e| format!("pixman: {e:?}"))?;
        Ok(Headless {
            renderer,
            size,
            targets: HashMap::new(),
            frames: 0,
        })
    }

    pub fn render(&mut self, state: &mut Rift, output: &Output) {
        let name = output.name();
        if !self.targets.contains_key(&name) {
            let (w, h) = self.size;
            match self.renderer.create_buffer(Fourcc::Xrgb8888, (w, h).into()) {
                Ok(buffer) => {
                    let damage_tracker = OutputDamageTracker::from_output(output);
                    self.targets.insert(
                        name.clone(),
                        Target {
                            buffer,
                            damage_tracker,
                        },
                    );
                }
                Err(e) => {
                    tracing::error!("cannot allocate headless buffer: {e:?}");
                    return;
                }
            }
        }
        let elements = crate::render::output_elements(state, &mut self.renderer, output, true);
        let clear = crate::render::background_color(state);
        let target = self.targets.get_mut(&name).unwrap();
        let rendered = match self.renderer.bind(&mut target.buffer) {
            Ok(mut fb) => match target.damage_tracker.render_output(
                &mut self.renderer,
                &mut fb,
                0,
                &elements,
                clear,
            ) {
                Ok(r) => r.damage.is_some(),
                Err(e) => {
                    tracing::warn!("headless render failed: {e:?}");
                    false
                }
            },
            Err(e) => {
                tracing::warn!("headless bind failed: {e:?}");
                false
            }
        };
        drop(elements);
        crate::screencopy::process(state, &mut self.renderer, output, rendered);
        if rendered {
            self.frames += 1;
        }
        super::post_render(state, output, rendered);
        super::throttle_idle(state, output);
    }
}

pub fn init(state: &mut Rift, count: usize) {
    let Backend::Headless(b) = &state.backend else {
        return;
    };
    let (w, h) = b.size;
    let formats: Vec<_> = b.renderer.shm_formats().collect();
    state.shm_state.update_formats(formats);
    for i in 0..count.max(1) {
        let output = Output::new(
            format!("HEADLESS-{}", i + 1),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Rift".into(),
                model: "Headless".into(),
                serial_number: "0".into(),
            },
        );
        let mode = Mode {
            size: (w, h).into(),
            refresh: 60_000,
        };
        output.create_global::<Rift>(&state.display_handle);
        output.change_current_state(Some(mode), Some(Transform::Normal), None, None);
        output.set_preferred(mode);
        state.add_output(output);
    }
}
