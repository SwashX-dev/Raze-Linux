//! Floating windows: dialogs (toplevels with a parent, or with a fixed size) are not
//! tiled. They keep the size they ask for, open centred on the working area of the
//! active monitor, belong to the workspace that was active when they opened, and are
//! drawn above the tiled windows.

use smithay::desktop::{layer_map_for_output, Window};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::{SurfaceCachedState, ToplevelSurface};

use crate::state::Rift;

pub struct Floating {
    pub window: Window,
    pub monitor: String,
    pub workspace: u64,
    /// Top-left corner, output-local; chosen when first placed.
    pub pos: Option<(i32, i32)>,
}

/// Whether a toplevel should float instead of being tiled.
pub fn is_dialog(toplevel: &ToplevelSurface) -> bool {
    if toplevel.parent().is_some() {
        return true;
    }
    with_states(toplevel.wl_surface(), |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let s = cached.current();
        s.min_size.w > 0 && s.min_size.h > 0 && s.min_size == s.max_size
    })
}

impl Rift {
    /// The active workspace (monitor name, workspace id) of the focused monitor.
    fn active_workspace(&self) -> Option<(String, u64)> {
        let m = self.layout.monitors.get(self.layout.active_monitor)?;
        Some((m.name.clone(), m.workspaces.get(m.active)?.id))
    }

    pub fn map_floating(&mut self, window: Window) {
        let Some((monitor, workspace)) = self.active_workspace() else {
            return;
        };
        self.floating.push(Floating {
            window: window.clone(),
            monitor,
            workspace,
            pos: None,
        });
        self.focused_floating = Some(window);
    }

    pub fn is_floating(&self, window: &Window) -> bool {
        self.floating.iter().any(|f| &f.window == window)
    }

    /// The focused window: a focused dialog, else the focused tiled window.
    pub fn focused_window(&self) -> Option<&Window> {
        self.focused_floating
            .as_ref()
            .filter(|w| self.floating_visible(w))
            .or_else(|| self.layout.focused_window())
    }

    fn floating_visible(&self, window: &Window) -> bool {
        self.floating
            .iter()
            .find(|f| &f.window == window)
            .map(|f| self.workspace_active(&f.monitor, f.workspace))
            .unwrap_or(false)
    }

    fn workspace_active(&self, monitor: &str, workspace: u64) -> bool {
        self.layout
            .monitors
            .iter()
            .find(|m| m.name == monitor)
            .and_then(|m| m.workspaces.get(m.active))
            .map(|ws| ws.id == workspace)
            .unwrap_or(false)
    }

    /// Place every floating window (called from `relayout`, after the tiled windows).
    pub fn layout_floating(&mut self) {
        for i in 0..self.floating.len() {
            // A dialog's workspace may have been removed (dynamic workspaces): adopt the
            // active workspace of its monitor (or of the focused one if that is gone).
            let (monitor, workspace) =
                (self.floating[i].monitor.clone(), self.floating[i].workspace);
            let m = self
                .layout
                .monitors
                .iter()
                .find(|m| m.name == monitor)
                .or_else(|| self.layout.monitors.get(self.layout.active_monitor));
            let Some(m) = m else { continue };
            if !m.workspaces.iter().any(|ws| ws.id == workspace) || m.name != monitor {
                let (name, id) = (m.name.clone(), m.workspaces[m.active].id);
                self.floating[i].monitor = name;
                self.floating[i].workspace = id;
                self.floating[i].pos = None;
            }
            let f = &self.floating[i];
            let window = f.window.clone();
            let visible = self.workspace_active(&f.monitor, f.workspace);
            let target = self
                .outputs
                .iter()
                .find(|o| o.output.name() == f.monitor)
                .map(|o| (o.lane, o.output.clone()));
            let focused = self.focused_floating.as_ref() == Some(&window);
            if let Some(t) = window.toplevel() {
                t.with_pending_state(|s| {
                    if focused {
                        s.states.set(xdg_toplevel::State::Activated);
                    } else {
                        s.states.unset(xdg_toplevel::State::Activated);
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
            match target {
                Some((lane, output)) if visible => {
                    let pos = match self.floating[i].pos {
                        Some(p) => p,
                        None => {
                            let area = layer_map_for_output(&output).non_exclusive_zone();
                            let size = window.geometry().size;
                            let p = (
                                area.loc.x + (area.size.w - size.w).max(0) / 2,
                                area.loc.y + (area.size.h - size.h).max(0) / 2,
                            );
                            self.floating[i].pos = Some(p);
                            p
                        }
                    };
                    self.space
                        .map_element(window.clone(), (lane + pos.0, pos.1), false);
                    self.space.raise_element(&window, false);
                }
                _ => self.space.unmap_elem(&window),
            }
        }
    }

    pub fn unmap_floating(&mut self, window: &Window) -> bool {
        let before = self.floating.len();
        self.floating.retain(|f| &f.window != window);
        if self.focused_floating.as_ref() == Some(window) {
            self.focused_floating = None;
        }
        before != self.floating.len()
    }
}
