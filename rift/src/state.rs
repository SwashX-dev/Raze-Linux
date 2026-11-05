//! Compositor state: protocol globals, windows, outputs, focus.

use std::collections::HashMap;
use std::ffi::OsString;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::desktop::{
    layer_map_for_output, LayerSurface, PopupManager, Space, Window, WindowSurfaceType,
};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER};
use smithay::wayland::compositor::{send_surface_state, with_states, CompositorClientState, CompositorState};
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::DmabufState;
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState};
use smithay::wayland::fractional_scale::{with_fractional_scale, FractionalScaleManagerState};
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::idle_notify::IdleNotifierState;
use smithay::wayland::image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState};
use smithay::wayland::image_copy_capture::ImageCopyCaptureState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::session_lock::{LockSurface, SessionLockManagerState, SessionLocker};
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::{XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::XdgActivationState;

use crate::backend::Backend;
use crate::config::Config;
use crate::layout::{Layout, LayoutConfig, Placement};

/// Each output is rendered from its own horizontal lane of the shared `Space`, so a
/// column scrolled partly off one monitor never shows up on its neighbour.
pub const LANE: i32 = 1 << 16;

pub struct OutputState {
    pub output: Output,
    pub lane: i32,
    pub redraw: bool,
    /// Rendering requested while a frame was in flight.
    pub pending: bool,
    pub lock_surface: Option<LockSurface>,
    /// Drawn at least once since the session was locked.
    pub locked_frame: bool,
    pub powered: bool,
}

pub struct Rift {
    pub start_time: Instant,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, Rift>,
    pub loop_signal: LoopSignal,
    pub socket_name: OsString,
    pub config: Config,
    pub config_error: Option<String>,

    pub layout: Layout<Window>,
    pub space: Space<Window>,
    pub popups: PopupManager,
    pub outputs: Vec<OutputState>,
    pub backend: Backend,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Rift>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub viewporter_state: ViewporterState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub xdg_activation_state: XdgActivationState,
    pub session_lock_state: SessionLockManagerState,
    pub idle_notifier_state: IdleNotifierState<Rift>,
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub foreign_toplevel_state: ForeignToplevelListState,
    pub presentation_state: PresentationState,
    pub image_capture_source_state: ImageCaptureSourceState,
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    pub single_pixel_buffer_state: SinglePixelBufferState,

    pub seat: Seat<Rift>,
    /// Pointer position in Space coordinates (inside an output lane).
    pub pointer_location: Point<f64, Logical>,
    pub cursor_status: CursorImageStatus,
    pub cursor_visible: bool,

    pub toplevel_handles: HashMap<WlSurface, ForeignToplevelHandle>,
    pub idle_inhibitors: Vec<WlSurface>,
    pub locked: bool,
    pub pending_lock: Option<SessionLocker>,
    pub children: Vec<Child>,
    pub xwayland_display: Option<String>,
    pub ipc: crate::ipc::Ipc,
    pub pending_captures: Vec<crate::screencopy::PendingCapture>,
    pub capture_sessions: Vec<smithay::wayland::image_copy_capture::Session>,
    pub last_state_hash: u64,
    /// Toplevels that have not committed a buffer yet.
    pub unmapped: Vec<Window>,
    pub dnd_icon: Option<WlSurface>,
    pub bindings: Vec<crate::input::Binding>,
    pub suppressed_keys: std::collections::HashSet<smithay::input::keyboard::Keycode>,
    pub cursors: crate::render::Cursors,
    pub decorations: crate::render::Decorations,
    /// Night-light colour temperature in kelvin (None: neutral gamma).
    pub night_light: Option<u32>,
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

pub fn layout_config(c: &Config) -> LayoutConfig {
    LayoutConfig {
        gap: c.layout.gap.max(0),
        presets: c.layout.presets.clone(),
        default_width: c.layout.default_width,
        center_focused: c.layout.center_focused,
        animation_ms: if c.animations.enabled { c.animations.duration_ms } else { 0 },
    }
}

impl Rift {
    pub fn new(
        display: Display<Rift>,
        loop_handle: LoopHandle<'static, Rift>,
        loop_signal: LoopSignal,
        backend: Backend,
        config: Config,
        config_error: Option<String>,
    ) -> Rift {
        let dh = display.handle();

        let source = ListeningSocketSource::new_auto().expect("cannot create a Wayland socket");
        let socket_name = source.socket_name().to_os_string();
        loop_handle
            .insert_source(source, |stream, _, state: &mut Rift| {
                if let Err(e) = state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                    tracing::warn!("cannot add client: {e}");
                }
            })
            .expect("cannot listen on the Wayland socket");
        loop_handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // SAFETY: the display is never dropped while the source exists.
                unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                }
                Ok(PostAction::Continue)
            })
            .expect("cannot watch the Wayland display");

        let compositor_state = CompositorState::new::<Rift>(&dh);
        let xdg_shell_state = XdgShellState::new::<Rift>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Rift>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Rift>(&dh);
        let shm_state = ShmState::new::<Rift>(&dh, vec![]);
        let dmabuf_state = DmabufState::new();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Rift>(&dh);
        let data_device_state = DataDeviceState::new::<Rift>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Rift>(&dh);
        let data_control_state = DataControlState::new::<Rift, _>(&dh, Some(&primary_selection_state), |_| true);
        let viewporter_state = ViewporterState::new::<Rift>(&dh);
        let fractional_scale_state = FractionalScaleManagerState::new::<Rift>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Rift>(&dh);
        let session_lock_state = SessionLockManagerState::new::<Rift, _>(&dh, |_| true);
        let idle_notifier_state = IdleNotifierState::<Rift>::new(&dh, loop_handle.clone());
        let idle_inhibit_state = IdleInhibitManagerState::new::<Rift>(&dh);
        let foreign_toplevel_state = ForeignToplevelListState::new::<Rift>(&dh);
        let presentation_state = PresentationState::new::<Rift>(&dh, libc::CLOCK_MONOTONIC as u32);
        let image_capture_source_state = ImageCaptureSourceState::new();
        let output_capture_source_state = OutputCaptureSourceState::new::<Rift>(&dh);
        let image_copy_capture_state = ImageCopyCaptureState::new::<Rift>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Rift>(&dh);
        CursorShapeManagerState::new::<Rift>(&dh);
        RelativePointerManagerState::new::<Rift>(&dh);
        PointerConstraintsState::new::<Rift>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, backend.seat_name());
        let kb = &config.input.keyboard;
        let xkb = XkbConfig {
            layout: &kb.layout,
            variant: &kb.variant,
            options: if kb.options.is_empty() { None } else { Some(kb.options.clone()) },
            model: &kb.model,
            ..Default::default()
        };
        if let Err(e) = seat.add_keyboard(xkb, kb.repeat_delay, kb.repeat_rate) {
            tracing::warn!("keyboard layout rejected ({e:?}); using defaults");
            seat.add_keyboard(XkbConfig::default(), kb.repeat_delay, kb.repeat_rate).expect("default keymap");
        }
        seat.add_pointer();

        let ipc = crate::ipc::Ipc::new(&loop_handle, &socket_name);
        let bindings = crate::input::bindings(&config);
        let cursors = crate::render::Cursors::new(&config.cursor.theme, config.cursor.size);

        Rift {
            start_time: Instant::now(),
            display_handle: dh,
            loop_handle,
            loop_signal,
            socket_name,
            layout: Layout::new(layout_config(&config)),
            config,
            config_error,
            space: Space::default(),
            popups: PopupManager::default(),
            outputs: Vec::new(),
            backend,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            shm_state,
            dmabuf_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            viewporter_state,
            fractional_scale_state,
            xdg_activation_state,
            session_lock_state,
            idle_notifier_state,
            idle_inhibit_state,
            foreign_toplevel_state,
            presentation_state,
            image_capture_source_state,
            output_capture_source_state,
            image_copy_capture_state,
            single_pixel_buffer_state,
            seat,
            pointer_location: (0.0, 0.0).into(),
            cursor_status: CursorImageStatus::default_named(),
            cursor_visible: true,
            toplevel_handles: HashMap::new(),
            idle_inhibitors: Vec::new(),
            locked: false,
            pending_lock: None,
            children: Vec::new(),
            xwayland_display: None,
            ipc,
            pending_captures: Vec::new(),
            capture_sessions: Vec::new(),
            last_state_hash: 0,
            unmapped: Vec::new(),
            dnd_icon: None,
            bindings,
            suppressed_keys: Default::default(),
            cursors,
            decorations: Default::default(),
            night_light: None,
        }
    }

    // ------------------------------------------------------------------ outputs

    /// Register an output (called by backends). Applies scale/position from config.
    pub fn add_output(&mut self, output: Output) {
        let name = output.name();
        let cfg = self.config.outputs.get(&name).cloned().unwrap_or_default();
        if let Some(s) = cfg.scale {
            output.change_current_state(None, None, Some(smithay::output::Scale::Fractional(s)), None);
        }
        let lane = (0..)
            .map(|i| i * LANE)
            .find(|l| !self.outputs.iter().any(|o| o.lane == *l))
            .unwrap();
        output.change_current_state(None, None, None, Some((lane, 0).into()));
        self.space.map_output(&output, (lane, 0));
        let size = self.output_logical_size(&output);
        self.layout.add_monitor(&name, (size.w, size.h));
        if self.outputs.is_empty() {
            self.pointer_location = (lane as f64 + size.w as f64 / 2.0, size.h as f64 / 2.0).into();
        }
        self.outputs.push(OutputState { output, lane, redraw: true, pending: false, lock_surface: None, locked_frame: false, powered: true });
        self.relayout();
    }

    pub fn remove_output(&mut self, output: &Output) {
        layer_map_for_output(output).cleanup();
        self.space.unmap_output(output);
        self.layout.remove_monitor(&output.name());
        self.outputs.retain(|o| &o.output != output);
        self.relayout();
    }

    /// Re-read an output's mode/scale (mode change, resize of a nested window).
    pub fn output_changed(&mut self, output: &Output) {
        let size = self.output_logical_size(output);
        self.layout.resize_monitor(&output.name(), (size.w, size.h));
        layer_map_for_output(output).arrange();
        self.relayout();
    }

    pub fn output_logical_size(&self, output: &Output) -> Size<i32, Logical> {
        let mode = output.current_mode().map(|m| m.size).unwrap_or((1280, 800).into());
        let scale = output.current_scale().fractional_scale();
        output.current_transform().transform_size(mode).to_f64().to_logical(scale).to_i32_round()
    }

    pub fn output_state(&self, output: &Output) -> Option<&OutputState> {
        self.outputs.iter().find(|o| &o.output == output)
    }

    /// The output whose lane contains `pos`.
    pub fn output_at(&self, pos: Point<f64, Logical>) -> Option<Output> {
        self.outputs
            .iter()
            .find(|o| {
                let size = self.output_logical_size(&o.output);
                Rectangle::new((o.lane, 0).into(), size).to_f64().contains(pos)
            })
            .map(|o| o.output.clone())
    }

    pub fn request_redraw_all(&mut self) {
        for o in self.outputs.iter_mut() {
            o.redraw = true;
        }
    }

    pub fn request_redraw(&mut self, output: &Output) {
        if let Some(o) = self.outputs.iter_mut().find(|o| &o.output == output) {
            o.redraw = true;
        }
    }

    // ------------------------------------------------------------------ windows

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.layout
            .windows()
            .find(|w| w.toplevel().map(|t| t.wl_surface() == surface).unwrap_or(false))
            .cloned()
    }

    pub fn app_id_title(window: &Window) -> (String, String) {
        let Some(toplevel) = window.toplevel() else { return Default::default() };
        with_states(toplevel.wl_surface(), |states| {
            let data = states.data_map.get::<XdgToplevelSurfaceData>().map(|d| d.lock().unwrap());
            match data {
                Some(d) => (d.app_id.clone().unwrap_or_default(), d.title.clone().unwrap_or_default()),
                None => Default::default(),
            }
        })
    }

    /// Add a newly mapped toplevel to the layout (after its first commit).
    pub fn map_window(&mut self, window: Window) {
        let (app_id, title) = Self::app_id_title(&window);
        let rule = self.config.rule_for(&app_id, &title);
        if let Some(t) = window.toplevel() {
            let handle = self.foreign_toplevel_state.new_toplevel::<Rift>(title.clone(), app_id.clone());
            self.toplevel_handles.insert(t.wl_surface().clone(), handle);
        }
        self.layout.add_window(window, &rule);
        self.relayout();
    }

    pub fn unmap_window(&mut self, window: &Window) {
        if let Some(t) = window.toplevel() {
            if let Some(h) = self.toplevel_handles.remove(t.wl_surface()) {
                self.foreign_toplevel_state.remove_toplevel(&h);
            }
        }
        self.space.unmap_elem(window);
        self.layout.remove_window(window);
        self.relayout();
    }

    pub fn update_toplevel_info(&mut self, window: &Window) {
        let (app_id, title) = Self::app_id_title(window);
        if let Some(t) = window.toplevel() {
            if let Some(h) = self.toplevel_handles.get(t.wl_surface()) {
                h.send_title(&title);
                h.send_app_id(&app_id);
                h.send_done();
            }
        }
        self.ipc_state_changed();
    }

    fn lane_of_monitor(&self, monitor: usize) -> Option<(i32, Output)> {
        let name = self.layout.monitors.get(monitor)?.name.clone();
        self.outputs.iter().find(|o| o.output.name() == name).map(|o| (o.lane, o.output.clone()))
    }

    /// Apply the layout: position/size every window, update focus and working areas.
    pub fn relayout(&mut self) {
        // Working areas from layer-shell exclusive zones.
        for o in &self.outputs {
            let zone = layer_map_for_output(&o.output).non_exclusive_zone();
            self.layout.set_working_area(
                &o.output.name(),
                crate::layout::Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h),
            );
        }
        let placements: Vec<Placement<Window>> = self.layout.arrange(Instant::now());
        for p in &placements {
            let Some(toplevel) = p.window.toplevel() else { continue };
            toplevel.with_pending_state(|s| {
                s.size = Some((p.rect.w, p.rect.h).into());
                if p.focused {
                    s.states.set(xdg_toplevel::State::Activated);
                } else {
                    s.states.unset(xdg_toplevel::State::Activated);
                }
                if p.fullscreen {
                    s.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    s.states.unset(xdg_toplevel::State::Fullscreen);
                }
                for st in [
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    s.states.set(st);
                }
            });
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
            match self.lane_of_monitor(p.monitor) {
                Some((lane, output)) if p.visible => {
                    let loc = (lane + p.rect.x, p.rect.y);
                    self.space.map_element(p.window.clone(), loc, false);
                    let scale = output.current_scale();
                    let transform = output.current_transform();
                    p.window.with_surfaces(|surface, states| {
                        send_surface_state(surface, states, scale.integer_scale(), transform);
                        with_fractional_scale(states, |fs| fs.set_preferred_scale(scale.fractional_scale()));
                    });
                }
                _ => self.space.unmap_elem(&p.window),
            }
        }
        for o in &self.outputs {
            let scale = o.output.current_scale();
            let transform = o.output.current_transform();
            for layer in layer_map_for_output(&o.output).layers() {
                layer.with_surfaces(|surface, states| {
                    send_surface_state(surface, states, scale.integer_scale(), transform);
                    with_fractional_scale(states, |fs| fs.set_preferred_scale(scale.fractional_scale()));
                });
            }
        }
        self.update_keyboard_focus();
        self.request_redraw_all();
        self.ipc_state_changed();
    }

    /// Placements for rendering decorations (focus ring) on one output.
    pub fn focused_rect_on(&self, output: &Output) -> Option<(Rectangle<i32, Logical>, bool)> {
        let focused = self.layout.focused_window()?;
        let loc = self.space.element_location(focused)?;
        let lane = self.output_state(output)?.lane;
        let geo = focused.geometry();
        let rect = Rectangle::new((loc.x - lane, loc.y).into(), geo.size);
        let fullscreen = self.layout.is_fullscreen(focused);
        Some((rect, fullscreen))
    }

    /// Keyboard focus: lock surface > exclusive layer surface > focused window.
    pub fn update_keyboard_focus(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let target: Option<WlSurface> = if self.locked {
            let active = self.layout.monitors.get(self.layout.active_monitor).map(|m| m.name.clone());
            self.outputs
                .iter()
                .find(|o| Some(o.output.name()) == active)
                .or(self.outputs.first())
                .and_then(|o| o.lock_surface.as_ref().map(|l| l.wl_surface().clone()))
        } else if let Some(layer) = self.exclusive_layer() {
            Some(layer.wl_surface().clone())
        } else {
            self.layout.focused_window().and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()))
        };
        if keyboard.current_focus() != target {
            keyboard.set_focus(self, target, serial);
        }
    }

    /// A layer surface (on the top or overlay layer) asking for exclusive keyboard focus.
    pub fn exclusive_layer(&self) -> Option<LayerSurface> {
        use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};
        for o in &self.outputs {
            let map = layer_map_for_output(&o.output);
            for layer in map.layers() {
                let exclusive = layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive;
                if exclusive && matches!(layer.layer(), Layer::Top | Layer::Overlay) {
                    return Some(layer.clone());
                }
            }
        }
        None
    }

    /// Topmost surface under the pointer, with its origin in Space coordinates.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        use smithay::wayland::shell::wlr_layer::Layer;
        let output = self.output_at(pos)?;
        let lane = self.output_state(&output)?.lane as f64;
        let local = pos - Point::from((lane, 0.0));
        if self.locked {
            let lock = self.output_state(&output)?.lock_surface.as_ref()?;
            return Some((lock.wl_surface().clone(), (lane, 0.0).into()));
        }
        let map = layer_map_for_output(&output);
        let layer_hit = |layer: Layer| {
            map.layer_under(layer, local).and_then(|l| {
                let geo = map.layer_geometry(l)?;
                l.surface_under(local - geo.loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + geo.loc).to_f64() + Point::from((lane, 0.0))))
            })
        };
        if let Some(hit) = layer_hit(Layer::Overlay).or_else(|| layer_hit(Layer::Top)) {
            return Some(hit);
        }
        if let Some((window, loc)) = self.space.element_under(pos) {
            if let Some((s, p)) = window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL) {
                return Some((s, (p + loc).to_f64()));
            }
        }
        layer_hit(Layer::Bottom).or_else(|| layer_hit(Layer::Background))
    }

    // ------------------------------------------------------------------ processes

    pub fn child_env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            ("WAYLAND_DISPLAY".to_string(), self.socket_name.to_string_lossy().into_owned()),
            ("XDG_CURRENT_DESKTOP".to_string(), "rift".to_string()),
            ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
            ("RIFT_SOCKET".to_string(), self.ipc.path.display().to_string()),
        ];
        if let Some(d) = &self.xwayland_display {
            env.push(("DISPLAY".to_string(), d.clone()));
        }
        env.extend(self.config.environment.iter().map(|(k, v)| (k.clone(), v.clone())));
        env
    }

    pub fn spawn(&mut self, command: &str) {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command).stdin(Stdio::null());
        for (k, v) in self.child_env() {
            cmd.env(k, v);
        }
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(child) => self.children.push(child),
            Err(e) => tracing::warn!("cannot spawn {command:?}: {e}"),
        }
    }

    pub fn reap_children(&mut self) {
        self.children.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
    }

    /// Start xwayland-satellite on the first free X display.
    pub fn start_xwayland(&mut self) {
        if !self.config.xwayland {
            return;
        }
        let Some(n) = (0..32).find(|n| {
            !std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
                && !std::path::Path::new(&format!("/tmp/.X{n}-lock")).exists()
        }) else {
            return;
        };
        let x_display = format!(":{n}");
        let mut cmd = Command::new("xwayland-satellite");
        cmd.arg(&x_display).stdin(Stdio::null());
        for (k, v) in self.child_env() {
            cmd.env(k, v);
        }
        match cmd.spawn() {
            Ok(child) => {
                tracing::info!("xwayland-satellite on DISPLAY={x_display}");
                self.children.push(child);
                self.xwayland_display = Some(x_display);
            }
            Err(e) => tracing::info!("XWayland disabled: xwayland-satellite not available ({e})"),
        }
    }

    pub fn reload_config(&mut self) {
        let (cfg, err) = Config::load();
        if let Some(e) = &err {
            tracing::warn!("config error, keeping the current configuration: {e}");
            self.config_error = err;
            return;
        }
        self.layout.config = layout_config(&cfg);
        self.bindings = crate::input::bindings(&cfg);
        self.cursors = crate::render::Cursors::new(&cfg.cursor.theme, cfg.cursor.size);
        self.config = cfg;
        self.config_error = None;
        self.relayout();
    }

    /// Apply `night_light` to every output's gamma ramp. Returns whether the backend
    /// supports it (only DRM outputs have gamma tables).
    pub fn apply_night_light(&mut self) -> bool {
        match &mut self.backend {
            crate::backend::Backend::Udev(u) => u.set_night_light(self.night_light),
            _ => false,
        }
    }

    // ------------------------------------------------------------------ frames

    /// Send frame callbacks for everything on `output` (after it was presented).
    pub fn send_frames(&self, output: &Output) {
        let time = self.start_time.elapsed();
        let throttle = Some(Duration::from_millis(995));
        for window in self.space.elements() {
            if self.space.outputs_for_element(window).contains(output) {
                window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
            }
        }
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
        if let Some(lock) = self.output_state(output).and_then(|o| o.lock_surface.as_ref()) {
            smithay::desktop::utils::send_frames_surface_tree(lock.wl_surface(), output, time, throttle, |_, _| {
                Some(output.clone())
            });
        }
        if let CursorImageStatus::Surface(s) = &self.cursor_status {
            smithay::desktop::utils::send_frames_surface_tree(s, output, time, throttle, |_, _| Some(output.clone()));
        }
    }

    /// Housekeeping after each event-loop iteration.
    pub fn refresh(&mut self) {
        self.space.refresh();
        self.popups.cleanup();
        for o in &self.outputs {
            layer_map_for_output(&o.output).cleanup();
        }
        if self.layout.is_animating(Instant::now()) {
            self.relayout();
        }
        self.reap_children();
        self.ipc_flush();
        let _ = self.display_handle.flush_clients();
    }
}
