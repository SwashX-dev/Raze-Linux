//! Wayland protocol handlers.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::delegate_dispatch2;
use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, LayerSurface, PopupKind,
    PopupKeyboardGrab, PopupPointerGrab, PopupUngrabStrategy, Window, WindowSurfaceType,
};
use smithay::input::dnd::{DnDGrab, DndGrabHandler, DndTarget, GrabType, Source};
use smithay::input::keyboard::LedState;
use smithay::input::pointer::{CursorImageStatus, Focus};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{Logical, Point, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState,
};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState};
use smithay::wayland::fractional_scale::{with_fractional_scale, FractionalScaleHandler};
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler, OutputCaptureSourceState,
};
use smithay::wayland::image_copy_capture::{
    BufferConstraints, Frame, FrameRef, ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler};
use smithay::input::pointer::PointerHandle;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
};
use smithay::wayland::selection::primary_selection::{set_primary_focus, PrimarySelectionHandler, PrimarySelectionState};
use smithay::wayland::selection::wlr_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker};
use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};

use crate::state::{ClientState, Rift};

impl BufferHandler for Rift {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

fn has_buffer(surface: &WlSurface) -> bool {
    use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
    with_states(surface, |states| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .map(|d| d.lock().unwrap().buffer().is_some())
            .unwrap_or(false)
    })
}

impl CompositorHandler for Rift {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("unknown client data")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);
        self.backend.early_import(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            // A toplevel waiting for its first buffer.
            if let Some(i) = self.unmapped.iter().position(|w| {
                w.toplevel()
                    .map(|t| t.wl_surface() == &root)
                    .unwrap_or(false)
            }) {
                let window = self.unmapped[i].clone();
                window.on_commit();
                let toplevel = window.toplevel().unwrap().clone();
                if !toplevel.is_initial_configure_sent() {
                    self.initial_configure(&toplevel);
                } else if has_buffer(&root) {
                    self.unmapped.remove(i);
                    self.map_window(window);
                }
            } else if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
                if &root == surface && !has_buffer(surface) {
                    // The client unmapped the toplevel by attaching a null buffer.
                    self.unmap_window(&window);
                    if let Some(t) = window.toplevel() {
                        t.with_pending_state(|s| s.size = None);
                        self.unmapped.push(Window::new_wayland_window(t.clone()));
                    }
                } else {
                    for o in self.space.outputs_for_element(&window) {
                        self.request_redraw(&o);
                    }
                }
            }
        }

        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                let _ = popup.send_configure();
            }
        }

        // Layer surfaces.
        let layer_output = self.outputs.iter().map(|o| o.output.clone()).find(|o| {
            layer_map_for_output(o)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        });
        if let Some(output) = layer_output {
            let initial_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<LayerSurfaceData>()
                    .map(|d| d.lock().unwrap().initial_configure_sent)
                    .unwrap_or(true)
            });
            let changed = {
                let mut map = layer_map_for_output(&output);
                map.arrange()
            };
            if !initial_sent {
                if let Some(layer) = layer_map_for_output(&output)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                {
                    layer.layer_surface().send_configure();
                }
            }
            let _ = changed;
            self.relayout();
        }

        // Lock surfaces and cursor surfaces just need a redraw.
        if self.locked {
            self.request_redraw_all();
        }
        if matches!(&self.cursor_status, CursorImageStatus::Surface(s) if s == surface) {
            self.request_redraw_all();
        }
    }
}

impl Rift {
    fn initial_configure(&mut self, toplevel: &ToplevelSurface) {
        // Dialogs float and choose their own size.
        let dialog = crate::floating::is_dialog(toplevel);
        let size = if dialog {
            None
        } else {
            self.layout.initial_size()
        };
        toplevel.with_pending_state(|s| {
            s.size = size.map(|(w, h)| (w, h).into());
            if dialog {
                return;
            }
            for st in [
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledLeft,
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledRight,
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledTop,
                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledBottom,
            ] {
                s.states.set(st);
            }
        });
        if let Some(o) = self.active_output() {
            let scale = o.current_scale();
            let transform = o.current_transform();
            with_states(toplevel.wl_surface(), |states| {
                smithay::wayland::compositor::send_surface_state(
                    toplevel.wl_surface(),
                    states,
                    scale.integer_scale(),
                    transform,
                );
                with_fractional_scale(states, |fs| {
                    fs.set_preferred_scale(scale.fractional_scale())
                });
            });
        }
        toplevel.send_configure();
    }

    pub fn active_output(&self) -> Option<Output> {
        let name = self
            .layout
            .monitors
            .get(self.layout.active_monitor)
            .map(|m| m.name.clone());
        self.outputs
            .iter()
            .find(|o| Some(o.output.name()) == name)
            .or(self.outputs.first())
            .map(|o| o.output.clone())
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        // Popups of toplevels: keep inside the toplevel's output.
        let (origin, output) = if let Some(window) = self.window_for_surface(&root) {
            let Some(loc) = self.space.element_location(&window) else {
                return;
            };
            let Some(output) = self.space.outputs_for_element(&window).into_iter().next() else {
                return;
            };
            (loc + window.geometry().loc, output)
        } else {
            // Popups of layer surfaces (panel menus).
            let found = self.outputs.iter().find_map(|o| {
                let map = layer_map_for_output(&o.output);
                let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
                let geo = map.layer_geometry(layer)?;
                Some((geo.loc + Point::from((o.lane, 0)), o.output.clone()))
            });
            let Some(f) = found else { return };
            f
        };
        let Some(out_geo) = self.space.output_geometry(&output) else {
            return;
        };
        let mut target = out_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= origin;
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

impl XdgShellHandler for Rift {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.unmapped.push(Window::new_wayland_window(surface));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.unmapped.retain(|w| w.toplevel() != Some(&surface));
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.unmap_window(&window);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(e) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!("cannot track popup: {e}");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        let Some(seat) = Seat::<Rift>::from_resource(&seat) else {
            return;
        };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else {
            return;
        };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn move_request(&mut self, _surface: ToplevelSurface, _seat: WlSeat, _serial: Serial) {
        // Tiled windows are positioned by the layout; interactive moves are keyboard-driven.
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: WlSeat,
        _serial: Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.layout.set_fullscreen(&window, true);
            self.relayout();
        } else {
            surface.with_pending_state(|s| {
                s.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen)
            });
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.layout.set_fullscreen(&window, false);
            self.relayout();
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.update_toplevel_info(&window);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.update_toplevel_info(&window);
        }
    }
}

impl XdgDecorationHandler for Rift {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        // Rift draws no title bars; ask clients to drop theirs too.
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl WlrLayerShellHandler for Rift {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.active_output());
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            tracing::warn!("cannot map layer surface: {e}");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for o in &self.outputs {
            let mut map = layer_map_for_output(&o.output);
            let layer = map
                .layers()
                .find(|l| l.layer_surface() == &surface)
                .cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
            }
        }
        self.relayout();
    }
}

impl ShmHandler for Rift {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for Rift {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self.backend.import_dmabuf(&dmabuf) {
            let _ = notifier.successful::<Rift>();
        } else {
            notifier.failed();
        }
    }
}

impl SeatHandler for Rift {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Rift> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, target: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = target.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        self.request_redraw_all();
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, led_state: LedState) {
        self.backend.update_led_state(led_state);
    }
}

impl smithay::input::tablet::TabletSeatHandler for Rift {
    type ToolFocus = WlSurface;
}

impl SelectionHandler for Rift {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Rift {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl WaylandDndGrabHandler for Rift {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        self.dnd_icon = icon;
        if let GrabType::Pointer = type_ {
            let pointer = seat.get_pointer().unwrap();
            if let Some(start) = pointer.grab_start_data() {
                pointer.set_grab(
                    self,
                    DnDGrab::new_pointer(&self.display_handle, start, source, seat),
                    serial,
                    Focus::Keep,
                );
            }
        }
    }
}

impl DndGrabHandler for Rift {
    fn dropped(
        &mut self,
        _target: Option<DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        self.dnd_icon = None;
    }
}

impl PrimarySelectionHandler for Rift {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl DataControlHandler for Rift {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

impl OutputHandler for Rift {}

impl FractionalScaleHandler for Rift {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self
            .active_output()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0);
        with_states(&surface, |states| {
            with_fractional_scale(states, |fs| fs.set_preferred_scale(scale))
        });
    }
}

impl XdgActivationHandler for Rift {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        // Tokens from the focused client (or without a serial, e.g. from launchers) are valid.
        match data.serial {
            Some((serial, seat)) => {
                let keyboard = self.seat.get_keyboard().unwrap();
                Seat::from_resource(&seat) == Some(self.seat.clone())
                    && keyboard
                        .last_enter()
                        .map(|e| serial.is_no_older_than(&e))
                        .unwrap_or(false)
            }
            None => true,
        }
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if data.timestamp.elapsed().as_secs() < 10 {
            if let Some(window) = self.window_for_surface(&surface) {
                self.focus_window(window);
            }
        }
    }
}

impl SessionLockHandler for Rift {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        self.locked = true;
        for o in self.outputs.iter_mut() {
            o.locked_frame = false;
        }
        // Confirm once every output has rendered a locked frame (see `render`).
        self.pending_lock = Some(confirmation);
        self.update_keyboard_focus();
        self.request_redraw_all();
    }

    fn unlock(&mut self) {
        self.locked = false;
        self.pending_lock = None;
        for o in self.outputs.iter_mut() {
            o.lock_surface = None;
        }
        self.update_keyboard_focus();
        self.request_redraw_all();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            return;
        };
        let size = self.output_logical_size(&output);
        surface.with_pending_state(|s| s.size = Some((size.w as u32, size.h as u32).into()));
        surface.send_configure();
        if let Some(o) = self.outputs.iter_mut().find(|o| o.output == output) {
            o.lock_surface = Some(surface);
        }
        self.update_keyboard_focus();
        self.request_redraw(&output);
    }
}

impl IdleNotifierHandler for Rift {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}

impl IdleInhibitHandler for Rift {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.push(surface);
        self.idle_notifier_state.set_is_inhibited(true);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.retain(|s| s != &surface);
        self.idle_notifier_state
            .set_is_inhibited(!self.idle_inhibitors.is_empty());
    }
}

impl ForeignToplevelListHandler for Rift {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}

impl PointerConstraintsHandler for Rift {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    c.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: Point<f64, Logical>,
    ) {
    }
}

impl ImageCaptureSourceHandler for Rift {
    fn source_destroyed(&mut self, _source: ImageCaptureSource) {}
}

impl OutputCaptureSourceHandler for Rift {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for Rift {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        crate::screencopy::constraints(source)
    }

    fn new_session(&mut self, session: Session) {
        self.capture_sessions.push(session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        crate::screencopy::queue_frame(self, session, frame);
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        self.pending_captures.retain(|c| *c.frame != frame);
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.capture_sessions.retain(|s| *s != session);
        self.pending_captures.retain(|c| c.session != session);
    }
}

delegate_dispatch2!(Rift);
