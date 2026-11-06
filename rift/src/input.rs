//! Input: key bindings, pointer movement across outputs, touch, touchpad gestures,
//! focus, actions.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent,
    GestureEndEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
};
use smithay::input::keyboard::{xkb, FilterResult, Keycode, ModifiersState};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent};
use smithay::input::touch;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraint};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::config::{parse_combo, Action, Combo};
use crate::state::Rift;

pub struct Binding {
    pub combo: Combo,
    pub action: Action,
}

/// Distance (logical pixels of touchpad movement) a swipe must travel to count.
const SWIPE_THRESHOLD: f64 = 60.0;

/// The action for a finished touchpad swipe: three or more fingers left/right scroll
/// through the columns (content follows the fingers), up/down switch workspaces.
pub fn swipe_action(fingers: u32, d: Point<f64, Logical>) -> Option<Action> {
    if fingers < 3 {
        return None;
    }
    if d.x.abs() >= d.y.abs() && d.x.abs() >= SWIPE_THRESHOLD {
        Some(if d.x < 0.0 {
            Action::FocusRight
        } else {
            Action::FocusLeft
        })
    } else if d.y.abs() > d.x.abs() && d.y.abs() >= SWIPE_THRESHOLD {
        Some(if d.y < 0.0 {
            Action::WorkspaceDown
        } else {
            Action::WorkspaceUp
        })
    } else {
        None
    }
}

/// Parse the `[binds]` table. Invalid entries were already rejected by `Config::validate`.
pub fn bindings(config: &crate::config::Config) -> Vec<Binding> {
    config
        .binds
        .iter()
        .filter_map(|(k, v)| {
            Some(Binding {
                combo: parse_combo(k).ok()?,
                action: Action::parse(v).ok()?,
            })
        })
        .collect()
}

fn matches(combo: &Combo, mods: &ModifiersState, sym: xkb::Keysym) -> bool {
    combo.mods.logo == mods.logo
        && combo.mods.ctrl == mods.ctrl
        && combo.mods.alt == mods.alt
        && combo.mods.shift == mods.shift
        && xkb::keysym_get_name(sym).eq_ignore_ascii_case(&combo.key)
}

/// Apply touchpad settings to a libinput device.
pub fn configure_device(
    config: &crate::config::Config,
    device: &mut smithay::reexports::input::Device,
) {
    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(config.input.touchpad.tap);
        if device.config_scroll_has_natural_scroll() {
            let _ = device
                .config_scroll_set_natural_scroll_enabled(config.input.touchpad.natural_scroll);
        }
        let _ = device.config_dwt_set_enabled(true);
    }
}

impl Rift {
    pub fn process_input_event<B: InputBackend>(&mut self, event: InputEvent<B>) {
        self.idle_notifier_state.notify_activity(&self.seat);
        if self.outputs.iter().any(|o| !o.powered)
            && !matches!(
                event,
                InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }
            )
        {
            self.set_monitors_power(true);
        }
        match event {
            InputEvent::Keyboard { event } => self.on_key::<B>(event),
            InputEvent::PointerMotion { event } => self.on_pointer_motion::<B>(event),
            InputEvent::PointerMotionAbsolute { event } => self.on_pointer_absolute::<B>(event),
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            InputEvent::TouchDown { event } => {
                let pos = self.touch_position(&event);
                self.on_touch_down(event.slot(), pos, event.time());
            }
            InputEvent::TouchMotion { event } => {
                let pos = self.touch_position(&event);
                self.on_touch_motion(event.slot(), pos, event.time());
            }
            InputEvent::TouchUp { event } => self.on_touch_up(event.slot(), event.time()),
            InputEvent::TouchFrame { .. } => {
                if let Some(t) = self.seat.get_touch() {
                    t.frame(self);
                }
            }
            InputEvent::TouchCancel { .. } => {
                self.touch_points.clear();
                if let Some(t) = self.seat.get_touch() {
                    t.cancel(self);
                }
            }
            InputEvent::GestureSwipeBegin { event } => {
                self.swipe = Some((event.fingers(), Point::from((0.0, 0.0))));
            }
            InputEvent::GestureSwipeUpdate { event } => {
                if let Some((_, d)) = &mut self.swipe {
                    *d += event.delta();
                }
            }
            InputEvent::GestureSwipeEnd { event } => {
                if let Some((fingers, d)) = self.swipe.take() {
                    if !event.cancelled() && !self.locked {
                        if let Some(a) = swipe_action(fingers, d) {
                            self.do_action(a);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_key<B: InputBackend>(&mut self, event: B::KeyboardKeyEvent) {
        let keycode: Keycode = event.key_code();
        let state = event.state();
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time(&event);
        let keyboard = self.seat.get_keyboard().unwrap();
        let locked = self.locked;
        let action = keyboard.input(self, keycode, state, serial, time, |rift, mods, handle| {
            if state == KeyState::Released {
                if rift.suppressed_keys.remove(&keycode) {
                    return FilterResult::Intercept(None);
                }
                return FilterResult::Forward;
            }
            let Some(sym) = handle.raw_latin_sym_or_raw_current_sym() else {
                return FilterResult::Forward;
            };
            // VT switching keysyms (XF86Switch_VT_n) come from Ctrl+Alt+Fn in the keymap.
            let raw = sym.raw();
            if (xkb::keysyms::KEY_XF86Switch_VT_1..=xkb::keysyms::KEY_XF86Switch_VT_12)
                .contains(&raw)
            {
                rift.suppressed_keys.insert(keycode);
                return FilterResult::Intercept(Some(Action::SwitchVt(
                    (raw - xkb::keysyms::KEY_XF86Switch_VT_1 + 1) as i32,
                )));
            }
            if locked {
                return FilterResult::Forward;
            }
            let found = rift
                .bindings
                .iter()
                .find(|b| matches(&b.combo, mods, sym))
                .map(|b| b.action.clone());
            match found {
                Some(action) => {
                    rift.suppressed_keys.insert(keycode);
                    FilterResult::Intercept(Some(action))
                }
                None => FilterResult::Forward,
            }
        });
        if let Some(Some(action)) = action {
            self.do_action(action);
        }
    }

    /// Ordered list of (lane, width, height) for pointer travel between outputs.
    fn output_strip(&self) -> Vec<(i32, f64, f64)> {
        let mut v: Vec<(i32, f64, f64, i32)> = self
            .outputs
            .iter()
            .map(|o| {
                let s = self.output_logical_size(&o.output);
                let pos = self
                    .config
                    .outputs
                    .get(&o.output.name())
                    .and_then(|c| c.position)
                    .map(|p| p.0)
                    .unwrap_or(o.lane);
                (o.lane, s.w as f64, s.h as f64, pos)
            })
            .collect();
        v.sort_by_key(|o| o.3);
        v.into_iter().map(|(l, w, h, _)| (l, w, h)).collect()
    }

    /// Move the pointer by `delta`, crossing to neighbouring outputs at the edges.
    fn move_pointer(
        &self,
        from: Point<f64, Logical>,
        delta: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let strip = self.output_strip();
        let Some(mut i) = strip
            .iter()
            .position(|(lane, w, _)| from.x >= *lane as f64 && from.x < *lane as f64 + w)
        else {
            return strip
                .first()
                .map(|(l, _, _)| Point::from((*l as f64, 0.0)))
                .unwrap_or(from);
        };
        let mut x = from.x - strip[i].0 as f64 + delta.x;
        let mut y = from.y + delta.y;
        while x >= strip[i].1 && i + 1 < strip.len() {
            x -= strip[i].1;
            i += 1;
        }
        while x < 0.0 && i > 0 {
            i -= 1;
            x += strip[i].1;
        }
        let (lane, w, h) = strip[i];
        x = x.clamp(0.0, w - 1.0);
        y = y.clamp(0.0, h - 1.0);
        Point::from((lane as f64 + x, y))
    }

    fn on_pointer_motion<B: InputBackend>(&mut self, event: B::PointerMotionEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.seat.get_pointer().unwrap();
        let under = self.surface_under(self.pointer_location);

        let mut locked = false;
        if let Some((surface, origin)) = &under {
            with_pointer_constraint(surface, &pointer, |c| {
                if let Some(c) = c {
                    if c.is_active() {
                        let inside = c.region().is_none_or(|r| {
                            r.contains((self.pointer_location - *origin).to_i32_round())
                        });
                        if inside && matches!(&*c, PointerConstraint::Locked(_)) {
                            locked = true;
                        }
                    }
                }
            });
        }
        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta: event.delta(),
                delta_unaccel: event.delta_unaccel(),
                time: event.time(),
            },
        );
        if locked {
            pointer.frame(self);
            return;
        }
        let new = self.move_pointer(self.pointer_location, event.delta());
        self.set_pointer(new, serial, event.time());
    }

    fn on_pointer_absolute<B: InputBackend>(&mut self, event: B::PointerMotionAbsoluteEvent) {
        let Some(o) = self.outputs.first() else {
            return;
        };
        let size = self.output_logical_size(&o.output);
        let pos = event.position_transformed(size) + Point::from((o.lane as f64, 0.0));
        self.set_pointer(pos, SERIAL_COUNTER.next_serial(), event.time());
    }

    fn set_pointer(
        &mut self,
        pos: Point<f64, Logical>,
        serial: smithay::utils::Serial,
        time: smithay::backend::input::InputTime,
    ) {
        let old_output = self.output_at(self.pointer_location);
        self.pointer_location = pos;
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(
            self,
            under.clone(),
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);

        let new_output = self.output_at(pos);
        if new_output != old_output {
            if let Some(o) = &new_output {
                if let Some(i) = self.layout.monitor_index(&o.name()) {
                    self.layout.focus_monitor(i);
                    self.relayout();
                }
            }
        }
        if self.config.input.focus_follows_mouse && !self.locked {
            if let Some((surface, _)) = &under {
                if let Some(window) = self.window_for_surface(surface) {
                    self.focus_window(window);
                }
            }
        }
        // The cursor is drawn by Rift, so every move damages the output(s) it touches.
        if let Some(o) = old_output {
            self.request_redraw(&o);
        }
        if let Some(o) = new_output {
            self.request_redraw(&o);
        }
    }

    /// Touchscreens map to the first output (the built-in panel on laptops).
    fn touch_position<E: AbsolutePositionEvent<B>, B: InputBackend>(
        &self,
        event: &E,
    ) -> Point<f64, Logical> {
        match self.outputs.first() {
            Some(o) => {
                let size = self.output_logical_size(&o.output);
                event.position_transformed(size) + Point::from((o.lane as f64, 0.0))
            }
            None => Point::from((0.0, 0.0)),
        }
    }

    fn on_touch_down(
        &mut self,
        slot: smithay::backend::input::TouchSlot,
        pos: Point<f64, Logical>,
        time: smithay::backend::input::InputTime,
    ) {
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.surface_under(pos);
        // Touching a window focuses it, like a click.
        if let (Some((surface, _)), false) = (&under, self.locked) {
            let root = self.root_surface(surface);
            if let Some(window) = self.window_for_surface(&root) {
                self.focus_window(window);
            } else if self.is_interactive_layer(&root) {
                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.set_focus(self, Some(root), serial);
            }
        }
        self.touch_points.retain(|(s, _)| *s != slot);
        self.touch_points.push((slot, under.clone()));
        if let Some(t) = self.seat.get_touch() {
            t.down(
                self,
                under,
                &touch::DownEvent {
                    slot,
                    location: pos,
                    serial,
                    time,
                },
            );
        }
    }

    fn on_touch_motion(
        &mut self,
        slot: smithay::backend::input::TouchSlot,
        pos: Point<f64, Logical>,
        time: smithay::backend::input::InputTime,
    ) {
        // A touch point stays with the surface it started on.
        let focus = self
            .touch_points
            .iter()
            .find(|(s, _)| *s == slot)
            .and_then(|(_, f)| f.clone());
        if let Some(t) = self.seat.get_touch() {
            t.motion(
                self,
                focus,
                &touch::MotionEvent {
                    slot,
                    location: pos,
                    time,
                },
            );
        }
    }

    fn on_touch_up(
        &mut self,
        slot: smithay::backend::input::TouchSlot,
        time: smithay::backend::input::InputTime,
    ) {
        self.touch_points.retain(|(s, _)| *s != slot);
        if let Some(t) = self.seat.get_touch() {
            t.up(
                self,
                &touch::UpEvent {
                    slot,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
        }
    }

    fn on_pointer_button<B: InputBackend>(&mut self, event: B::PointerButtonEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let state = event.state();
        if state == ButtonState::Pressed && !self.locked {
            // Click to focus: windows and layer surfaces with keyboard interactivity.
            if let Some((surface, _)) = self.surface_under(self.pointer_location) {
                let root = self.root_surface(&surface);
                if let Some(window) = self.window_for_surface(&root) {
                    self.focus_window(window);
                } else if self.is_interactive_layer(&root) {
                    let keyboard = self.seat.get_keyboard().unwrap();
                    keyboard.set_focus(self, Some(root), serial);
                }
            }
        }
        let pointer = self.seat.get_pointer().unwrap();
        pointer.button(
            self,
            &ButtonEvent {
                button: event.button_code(),
                state,
                serial,
                time: event.time(),
            },
        );
        pointer.frame(self);
    }

    fn is_interactive_layer(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> bool {
        use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
        self.outputs.iter().any(|o| {
            let map = smithay::desktop::layer_map_for_output(&o.output);
            map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                .map(|l| l.cached_state().keyboard_interactivity != KeyboardInteractivity::None)
                .unwrap_or(false)
        })
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, event: B::PointerAxisEvent) {
        let source = event.source();
        let h = event
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0);
        let v = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0);
        let mut frame = AxisFrame::new(event.time()).source(source);
        if h != 0.0 {
            frame = frame
                .relative_direction(Axis::Horizontal, event.relative_direction(Axis::Horizontal))
                .value(Axis::Horizontal, h);
            if let Some(d) = event.amount_v120(Axis::Horizontal) {
                frame = frame.v120(Axis::Horizontal, d as i32);
            }
        }
        if v != 0.0 {
            frame = frame
                .relative_direction(Axis::Vertical, event.relative_direction(Axis::Vertical))
                .value(Axis::Vertical, v);
            if let Some(d) = event.amount_v120(Axis::Vertical) {
                frame = frame.v120(Axis::Vertical, d as i32);
            }
        }
        if source == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }
        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    pub fn set_monitors_power(&mut self, on: bool) {
        let mut backend = std::mem::replace(&mut self.backend, crate::backend::Backend::Empty);
        match &mut backend {
            crate::backend::Backend::Udev(u) => u.set_power(self, on),
            _ => {
                for o in self.outputs.iter_mut() {
                    o.powered = on;
                    o.redraw = true;
                }
            }
        }
        self.backend = backend;
    }

    pub fn close_focused(&mut self) {
        if let Some(t) = self.focused_window().and_then(|w| w.toplevel().cloned()) {
            t.send_close();
        }
    }

    /// Focus a window (tiled or floating) if it is not focused already.
    pub fn focus_window(&mut self, window: smithay::desktop::Window) {
        if self.focused_window() == Some(&window) {
            return;
        }
        if self.is_floating(&window) {
            self.focused_floating = Some(window);
        } else {
            self.focused_floating = None;
            self.layout.focus_window(&window);
        }
        self.relayout();
    }

    pub fn do_action(&mut self, action: Action) {
        // Layout actions act on the tiled windows: a focused dialog gives up focus.
        if !matches!(
            action,
            Action::Spawn(_)
                | Action::Close
                | Action::Quit
                | Action::ReloadConfig
                | Action::PowerOffMonitors
                | Action::SwitchVt(_)
                | Action::ToggleAnimations
        ) {
            self.focused_floating = None;
        }
        let l = &mut self.layout;
        match action {
            Action::Spawn(cmd) => return self.spawn(&cmd),
            Action::Close => return self.close_focused(),
            Action::Quit => return self.loop_signal.stop(),
            Action::FocusLeft => l.focus_left(),
            Action::FocusRight => l.focus_right(),
            Action::FocusUp => l.focus_up(),
            Action::FocusDown => l.focus_down(),
            Action::FocusFirst => l.focus_first(),
            Action::FocusLast => l.focus_last(),
            Action::MoveLeft => l.move_column_left(),
            Action::MoveRight => l.move_column_right(),
            Action::MoveUp => l.move_window_up(),
            Action::MoveDown => l.move_window_down(),
            Action::Consume => l.consume_into_column(),
            Action::Expel => l.expel_from_column(),
            Action::SwitchWidth => l.switch_preset_width(),
            Action::AdjustWidth(d) => l.adjust_width(d),
            Action::Maximize => l.toggle_maximize(),
            Action::Fullscreen => l.toggle_fullscreen(),
            Action::Center => l.center_column(),
            Action::Workspace(n) => l.focus_workspace(n),
            Action::WorkspaceUp => l.focus_workspace_up(),
            Action::WorkspaceDown => l.focus_workspace_down(),
            Action::MoveToWorkspace(n) => l.move_column_to_workspace(n),
            Action::MoveToWorkspaceUp => l.move_column_to_workspace_up(),
            Action::MoveToWorkspaceDown => l.move_column_to_workspace_down(),
            Action::FocusMonitorNext => {
                l.focus_monitor_next();
                self.warp_pointer_to_active_monitor();
            }
            Action::MoveToMonitorNext => {
                l.move_column_to_monitor_next();
                self.warp_pointer_to_active_monitor();
            }
            Action::ToggleAnimations => {
                self.config.animations.enabled = !self.config.animations.enabled;
                self.layout.config = crate::state::layout_config(&self.config);
            }
            Action::ReloadConfig => return self.reload_config(),
            Action::PowerOffMonitors => return self.set_monitors_power(false),
            Action::SwitchVt(vt) => {
                if let crate::backend::Backend::Udev(u) = &mut self.backend {
                    u.change_vt(vt);
                }
                return;
            }
        }
        self.relayout();
    }

    fn warp_pointer_to_active_monitor(&mut self) {
        let Some(output) = self.active_output() else {
            return;
        };
        if self.output_at(self.pointer_location).as_ref() == Some(&output) {
            return;
        }
        let Some(lane) = self.output_state(&output).map(|o| o.lane) else {
            return;
        };
        let size = self.output_logical_size(&output);
        self.pointer_location = (lane as f64 + size.w as f64 / 2.0, size.h as f64 / 2.0).into();
        self.request_redraw_all();
    }
}

impl Rift {
    /// The toplevel or layer surface a (sub)surface or popup belongs to.
    fn root_surface(&self, surface: &WlSurface) -> WlSurface {
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(popup) = self.popups.find_popup(&root) {
            if let Ok(r) = smithay::desktop::find_popup_root_surface(&popup) {
                return r;
            }
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touchpad_swipes() {
        let p = |x: f64, y: f64| Point::<f64, Logical>::from((x, y));
        assert!(swipe_action(2, p(-200.0, 0.0)).is_none());
        assert!(swipe_action(3, p(-20.0, 10.0)).is_none());
        assert!(matches!(
            swipe_action(3, p(-200.0, 30.0)),
            Some(Action::FocusRight)
        ));
        assert!(matches!(
            swipe_action(4, p(150.0, -20.0)),
            Some(Action::FocusLeft)
        ));
        assert!(matches!(
            swipe_action(3, p(10.0, -120.0)),
            Some(Action::WorkspaceDown)
        ));
        assert!(matches!(
            swipe_action(3, p(10.0, 120.0)),
            Some(Action::WorkspaceUp)
        ));
    }
}
