//! Scrollable-tiling layout engine.
//!
//! Each monitor has a vertical list of dynamic workspaces. A workspace is an endless
//! horizontal strip of columns; a column holds one or more windows stacked
//! vertically. Opening a window never resizes the others: the strip grows and the
//! view scrolls to keep the focused column visible.
//!
//! The engine is generic over the window handle and contains no Wayland types, so
//! all behaviour is unit-tested without a display.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && y >= self.y as f64 && x < (self.x + self.w) as f64 && y < (self.y + self.h) as f64
    }
}

#[derive(Clone, Debug)]
pub struct LayoutConfig {
    /// Gap between columns/windows and around the strip, in logical pixels.
    pub gap: i32,
    /// Column width presets as proportions of the working area.
    pub presets: Vec<f64>,
    /// Width of new columns (proportion).
    pub default_width: f64,
    /// Keep the focused column centred instead of scrolling the minimum distance.
    pub center_focused: bool,
    /// View scroll animation; 0 disables animations.
    pub animation_ms: u32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        LayoutConfig {
            gap: 10,
            presets: vec![1.0 / 3.0, 0.5, 2.0 / 3.0],
            default_width: 0.5,
            center_focused: false,
            animation_ms: 180,
        }
    }
}

/// Placement hints for a newly opened window (from window rules).
#[derive(Clone, Debug, Default)]
pub struct OpenRule {
    pub width: Option<f64>,
    /// 0-based workspace index on the active monitor.
    pub workspace: Option<usize>,
    pub maximized: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Debug)]
pub struct Column<W> {
    pub windows: Vec<W>,
    pub focused: usize,
    /// Width as a proportion of the working area.
    pub width: f64,
    pub maximized: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Copy, Debug)]
struct Anim {
    from: f64,
    to: f64,
    start: Instant,
    dur: Duration,
}

impl Anim {
    fn value(&self, now: Instant) -> f64 {
        let t = now.saturating_duration_since(self.start).as_secs_f64() / self.dur.as_secs_f64().max(1e-6);
        if t >= 1.0 {
            return self.to;
        }
        // Ease-out cubic.
        let e = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * e
    }
    fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.dur
    }
}

#[derive(Clone, Debug)]
pub struct Workspace<W> {
    pub id: u64,
    pub columns: Vec<Column<W>>,
    pub focused: usize,
    view: f64,
    anim: Option<Anim>,
}

impl<W> Workspace<W> {
    fn new(id: u64) -> Self {
        Workspace { id, columns: Vec::new(), focused: 0, view: 0.0, anim: None }
    }
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
    pub fn window_count(&self) -> usize {
        self.columns.iter().map(|c| c.windows.len()).sum()
    }
    fn view_at(&self, now: Instant) -> f64 {
        self.anim.map(|a| a.value(now)).unwrap_or(self.view)
    }
}

#[derive(Clone, Debug)]
pub struct Monitor<W> {
    pub name: String,
    /// Full monitor size (logical).
    pub size: (i32, i32),
    /// Working area (monitor minus exclusive zones of panels), monitor-relative.
    pub area: Rect,
    pub workspaces: Vec<Workspace<W>>,
    pub active: usize,
}

/// Where a window should be displayed this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement<W> {
    pub window: W,
    pub monitor: usize,
    /// Monitor-relative logical rectangle.
    pub rect: Rect,
    pub visible: bool,
    pub focused: bool,
    pub fullscreen: bool,
}

/// Summary for IPC/shell.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WorkspaceInfo {
    pub id: u64,
    pub index: usize,
    pub monitor: String,
    pub active: bool,
    pub focused: bool,
    pub windows: usize,
}

pub struct Layout<W> {
    pub monitors: Vec<Monitor<W>>,
    pub active_monitor: usize,
    pub config: LayoutConfig,
    next_id: u64,
    /// Workspaces kept while no monitor is connected.
    detached: Vec<Workspace<W>>,
}

impl<W: Clone + PartialEq> Layout<W> {
    pub fn new(config: LayoutConfig) -> Self {
        Layout { monitors: Vec::new(), active_monitor: 0, config, next_id: 1, detached: Vec::new() }
    }

    fn new_ws(&mut self) -> Workspace<W> {
        let id = self.next_id;
        self.next_id += 1;
        Workspace::new(id)
    }

    // ------------------------------------------------------------------ monitors

    pub fn add_monitor(&mut self, name: &str, size: (i32, i32)) {
        if self.monitors.iter().any(|m| m.name == name) {
            return;
        }
        let mut workspaces: Vec<Workspace<W>> = std::mem::take(&mut self.detached);
        if workspaces.is_empty() {
            workspaces.push(self.new_ws());
        }
        self.monitors.push(Monitor {
            name: name.into(),
            size,
            area: Rect::new(0, 0, size.0, size.1),
            workspaces,
            active: 0,
        });
        let i = self.monitors.len() - 1;
        self.normalize(i);
    }

    /// Remove a monitor; its windows move to another monitor (or are kept until one
    /// appears).
    pub fn remove_monitor(&mut self, name: &str) {
        let Some(i) = self.monitors.iter().position(|m| m.name == name) else { return };
        let m = self.monitors.remove(i);
        let moved: Vec<Workspace<W>> = m.workspaces.into_iter().filter(|w| !w.is_empty()).collect();
        if self.monitors.is_empty() {
            self.detached.extend(moved);
            self.active_monitor = 0;
            return;
        }
        if self.active_monitor >= self.monitors.len() || self.active_monitor == i {
            self.active_monitor = 0;
        } else if self.active_monitor > i {
            self.active_monitor -= 1;
        }
        let t = self.active_monitor;
        let target = &mut self.monitors[t];
        let insert_at = target.workspaces.len().saturating_sub(1);
        for (k, ws) in moved.into_iter().enumerate() {
            target.workspaces.insert(insert_at + k, ws);
        }
        self.normalize(t);
    }

    pub fn resize_monitor(&mut self, name: &str, size: (i32, i32)) {
        if let Some(m) = self.monitors.iter_mut().find(|m| m.name == name) {
            m.size = size;
            m.area = Rect::new(0, 0, size.0, size.1);
        }
    }

    pub fn set_working_area(&mut self, name: &str, area: Rect) {
        if let Some(i) = self.monitors.iter().position(|m| m.name == name) {
            if self.monitors[i].area != area {
                self.monitors[i].area = area;
                let now = Instant::now();
                for w in 0..self.monitors[i].workspaces.len() {
                    self.scroll_to_focus(i, w, now, false);
                }
            }
        }
    }

    pub fn monitor_index(&self, name: &str) -> Option<usize> {
        self.monitors.iter().position(|m| m.name == name)
    }

    pub fn focus_monitor(&mut self, i: usize) {
        if i < self.monitors.len() {
            self.active_monitor = i;
        }
    }

    /// Keep exactly one empty workspace at the end; drop other empty workspaces that
    /// are not active.
    fn normalize(&mut self, mi: usize) {
        let needs_tail = self.monitors[mi].workspaces.last().map(|w| !w.is_empty()).unwrap_or(true);
        if needs_tail {
            let ws = self.new_ws();
            self.monitors[mi].workspaces.push(ws);
        }
        let m = &mut self.monitors[mi];
        let active_id = m.workspaces[m.active.min(m.workspaces.len() - 1)].id;
        let last = m.workspaces.len() - 1;
        let mut k = 0;
        m.workspaces.retain(|w| {
            let keep = !w.is_empty() || w.id == active_id || k == last;
            k += 1;
            keep
        });
        // Keep only one trailing empty workspace (the active one may be empty and last).
        while m.workspaces.len() >= 2
            && m.workspaces[m.workspaces.len() - 1].is_empty()
            && m.workspaces[m.workspaces.len() - 2].is_empty()
            && m.workspaces[m.workspaces.len() - 2].id != active_id
        {
            let n = m.workspaces.len();
            m.workspaces.remove(n - 2);
        }
        while m.workspaces.len() >= 2
            && m.workspaces[m.workspaces.len() - 1].is_empty()
            && m.workspaces[m.workspaces.len() - 2].is_empty()
            && m.workspaces[m.workspaces.len() - 2].id == active_id
        {
            m.workspaces.pop();
        }
        m.active = m.workspaces.iter().position(|w| w.id == active_id).unwrap_or(0);
    }

    // ------------------------------------------------------------------ windows

    pub fn find(&self, w: &W) -> Option<(usize, usize, usize, usize)> {
        for (mi, m) in self.monitors.iter().enumerate() {
            for (wi, ws) in m.workspaces.iter().enumerate() {
                for (ci, c) in ws.columns.iter().enumerate() {
                    if let Some(k) = c.windows.iter().position(|x| x == w) {
                        return Some((mi, wi, ci, k));
                    }
                }
            }
        }
        None
    }

    pub fn windows(&self) -> impl Iterator<Item = &W> {
        self.monitors
            .iter()
            .flat_map(|m| m.workspaces.iter())
            .chain(self.detached.iter())
            .flat_map(|ws| ws.columns.iter())
            .flat_map(|c| c.windows.iter())
    }

    fn active_ws_idx(&self) -> Option<(usize, usize)> {
        let m = self.monitors.get(self.active_monitor)?;
        Some((self.active_monitor, m.active))
    }

    pub fn focused_window(&self) -> Option<&W> {
        let (mi, wi) = self.active_ws_idx()?;
        let ws = &self.monitors[mi].workspaces[wi];
        let c = ws.columns.get(ws.focused)?;
        c.windows.get(c.focused)
    }

    pub fn add_window(&mut self, w: W, rule: &OpenRule) {
        if self.monitors.is_empty() {
            let mut ws = match self.detached.pop() {
                Some(ws) => ws,
                None => self.new_ws(),
            };
            ws.columns.push(Column { windows: vec![w], focused: 0, width: self.config.default_width, maximized: false, fullscreen: false });
            self.detached.push(ws);
            return;
        }
        let mi = self.active_monitor.min(self.monitors.len() - 1);
        let wi = match rule.workspace {
            Some(n) => {
                while self.monitors[mi].workspaces.len() <= n {
                    let ws = self.new_ws();
                    self.monitors[mi].workspaces.push(ws);
                }
                n
            }
            None => self.monitors[mi].active,
        };
        let width = rule.width.unwrap_or(self.config.default_width).clamp(0.05, 1.0);
        let ws = &mut self.monitors[mi].workspaces[wi];
        let at = if ws.columns.is_empty() { 0 } else { ws.focused + 1 };
        ws.columns.insert(
            at,
            Column { windows: vec![w], focused: 0, width, maximized: rule.maximized, fullscreen: rule.fullscreen },
        );
        ws.focused = at;
        if rule.workspace.is_none() || rule.workspace == Some(self.monitors[mi].active) {
            self.scroll_to_focus(mi, wi, Instant::now(), true);
        }
        self.normalize(mi);
    }

    pub fn remove_window(&mut self, w: &W) -> bool {
        for ws in self.detached.iter_mut() {
            for c in ws.columns.iter_mut() {
                c.windows.retain(|x| x != w);
            }
            ws.columns.retain(|c| !c.windows.is_empty());
        }
        self.detached.retain(|ws| !ws.is_empty());
        let Some((mi, wi, ci, k)) = self.find(w) else { return false };
        let ws = &mut self.monitors[mi].workspaces[wi];
        let col = &mut ws.columns[ci];
        col.windows.remove(k);
        if col.windows.is_empty() {
            ws.columns.remove(ci);
            if ws.focused >= ci && ws.focused > 0 {
                ws.focused -= 1;
            }
        } else if col.focused >= col.windows.len() {
            col.focused = col.windows.len() - 1;
        }
        ws.focused = ws.focused.min(ws.columns.len().saturating_sub(1));
        self.scroll_to_focus(mi, wi, Instant::now(), true);
        self.normalize(mi);
        true
    }

    /// Focus a window (e.g. on click or activation request), switching workspace and
    /// monitor as needed.
    pub fn focus_window(&mut self, w: &W) {
        if let Some((mi, wi, ci, k)) = self.find(w) {
            self.active_monitor = mi;
            self.monitors[mi].active = wi;
            let ws = &mut self.monitors[mi].workspaces[wi];
            ws.focused = ci;
            ws.columns[ci].focused = k;
            self.scroll_to_focus(mi, wi, Instant::now(), true);
            self.normalize(mi);
        }
    }

    // ------------------------------------------------------------------ geometry

    fn column_width(&self, area: &Rect, c: &Column<W>) -> i32 {
        let g = self.config.gap;
        if c.maximized {
            return (area.w - 2 * g).max(1);
        }
        (((area.w - g) as f64 * c.width).round() as i32 - g).max(1)
    }

    /// Size a new window would get on the active monitor (sent with its first configure).
    pub fn initial_size(&self) -> Option<(i32, i32)> {
        let m = self.monitors.get(self.active_monitor)?;
        let g = self.config.gap;
        let c: Column<W> = Column { windows: vec![], focused: 0, width: self.config.default_width, maximized: false, fullscreen: false };
        Some((self.column_width(&m.area, &c), (m.area.h - 2 * g).max(1)))
    }

    /// Strip x position (relative to the strip start) and width of each column.
    fn column_spans(&self, area: &Rect, ws: &Workspace<W>) -> Vec<(i32, i32)> {
        let g = self.config.gap;
        let mut x = 0;
        ws.columns
            .iter()
            .map(|c| {
                let w = self.column_width(area, c);
                let span = (x, w);
                x += w + g;
                span
            })
            .collect()
    }

    fn scroll_to_focus(&mut self, mi: usize, wi: usize, now: Instant, animate: bool) {
        let area = self.monitors[mi].area;
        let g = self.config.gap;
        let spans = self.column_spans(&area, &self.monitors[mi].workspaces[wi]);
        let center = self.config.center_focused;
        let anim_ms = self.config.animation_ms;
        let ws = &mut self.monitors[mi].workspaces[wi];
        let Some(&(x, w)) = spans.get(ws.focused) else {
            ws.view = 0.0;
            ws.anim = None;
            return;
        };
        let viewport = (area.w - 2 * g).max(1) as f64;
        let (x, w) = (x as f64, w as f64);
        let current = ws.view_at(now);
        let mut target = ws.view;
        if center || w >= viewport {
            target = if w >= viewport { x } else { x + w / 2.0 - viewport / 2.0 };
        } else if x < target {
            target = x;
        } else if x + w > target + viewport {
            target = x + w - viewport;
        }
        // Never scroll past the start of the strip.
        target = target.max(0.0);
        if (target - ws.view).abs() < 0.5 && ws.anim.is_none() {
            return;
        }
        ws.view = target;
        ws.anim = if animate && anim_ms > 0 && (current - target).abs() >= 1.0 {
            Some(Anim { from: current, to: target, start: now, dur: Duration::from_millis(anim_ms as u64) })
        } else {
            None
        };
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.monitors
            .iter()
            .flat_map(|m| m.workspaces.iter())
            .any(|ws| ws.anim.map(|a| !a.done(now)).unwrap_or(false))
    }

    /// Compute every window's placement at time `now`.
    pub fn arrange(&mut self, now: Instant) -> Vec<Placement<W>> {
        for m in self.monitors.iter_mut() {
            for ws in m.workspaces.iter_mut() {
                if ws.anim.map(|a| a.done(now)).unwrap_or(false) {
                    ws.anim = None;
                }
            }
        }
        let g = self.config.gap;
        let focused = self.focused_window().cloned();
        let mut out = Vec::new();
        for (mi, m) in self.monitors.iter().enumerate() {
            let area = m.area;
            for (wi, ws) in m.workspaces.iter().enumerate() {
                let active = wi == m.active;
                let spans = self.column_spans(&area, ws);
                let view = ws.view_at(now);
                let fullscreen_col = ws.columns.get(ws.focused).filter(|c| c.fullscreen).map(|_| ws.focused);
                for (ci, c) in ws.columns.iter().enumerate() {
                    let (sx, cw) = spans[ci];
                    let n = c.windows.len() as i32;
                    let x = area.x + g + sx - view.round() as i32;
                    let avail = area.h - g * (n + 1);
                    let each = (avail / n.max(1)).max(1);
                    for (k, w) in c.windows.iter().enumerate() {
                        let is_full = fullscreen_col == Some(ci) && k == c.focused;
                        let rect = if is_full {
                            Rect::new(0, 0, m.size.0, m.size.1)
                        } else {
                            let y = area.y + g + k as i32 * (each + g);
                            let h = if k as i32 == n - 1 { area.y + area.h - g - y } else { each };
                            Rect::new(x, y, cw, h.max(1))
                        };
                        let on_screen = rect.intersects(&Rect::new(0, 0, m.size.0, m.size.1));
                        let hidden_by_fullscreen = fullscreen_col.is_some() && !is_full;
                        out.push(Placement {
                            window: w.clone(),
                            monitor: mi,
                            rect,
                            visible: active && on_screen && !hidden_by_fullscreen,
                            focused: Some(w) == focused.as_ref() && mi == self.active_monitor,
                            fullscreen: is_full,
                        });
                    }
                }
            }
        }
        out
    }

    pub fn workspaces(&self) -> Vec<WorkspaceInfo> {
        let mut out = Vec::new();
        for (mi, m) in self.monitors.iter().enumerate() {
            for (wi, ws) in m.workspaces.iter().enumerate() {
                out.push(WorkspaceInfo {
                    id: ws.id,
                    index: wi,
                    monitor: m.name.clone(),
                    active: wi == m.active,
                    focused: wi == m.active && mi == self.active_monitor,
                    windows: ws.window_count(),
                });
            }
        }
        out
    }

    /// Index of the workspace containing `w` (for IPC).
    pub fn workspace_of(&self, w: &W) -> Option<u64> {
        let (mi, wi, _, _) = self.find(w)?;
        Some(self.monitors[mi].workspaces[wi].id)
    }

    // ------------------------------------------------------------------ actions

    fn with_active<R>(&mut self, f: impl FnOnce(&mut Workspace<W>) -> R) -> Option<R> {
        let (mi, wi) = self.active_ws_idx()?;
        let r = f(&mut self.monitors[mi].workspaces[wi]);
        self.scroll_to_focus(mi, wi, Instant::now(), true);
        self.normalize(mi);
        Some(r)
    }

    pub fn focus_left(&mut self) {
        self.with_active(|ws| ws.focused = ws.focused.saturating_sub(1));
    }

    pub fn focus_right(&mut self) {
        self.with_active(|ws| {
            if ws.focused + 1 < ws.columns.len() {
                ws.focused += 1;
            }
        });
    }

    pub fn focus_up(&mut self) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                c.focused = c.focused.saturating_sub(1);
            }
        });
    }

    pub fn focus_down(&mut self) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                if c.focused + 1 < c.windows.len() {
                    c.focused += 1;
                }
            }
        });
    }

    pub fn focus_first(&mut self) {
        self.with_active(|ws| ws.focused = 0);
    }

    pub fn focus_last(&mut self) {
        self.with_active(|ws| ws.focused = ws.columns.len().saturating_sub(1));
    }

    pub fn move_column_left(&mut self) {
        self.with_active(|ws| {
            if ws.focused > 0 && ws.focused < ws.columns.len() {
                ws.columns.swap(ws.focused, ws.focused - 1);
                ws.focused -= 1;
            }
        });
    }

    pub fn move_column_right(&mut self) {
        self.with_active(|ws| {
            if ws.focused + 1 < ws.columns.len() {
                ws.columns.swap(ws.focused, ws.focused + 1);
                ws.focused += 1;
            }
        });
    }

    pub fn move_window_up(&mut self) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                if c.focused > 0 {
                    c.windows.swap(c.focused, c.focused - 1);
                    c.focused -= 1;
                }
            }
        });
    }

    pub fn move_window_down(&mut self) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                if c.focused + 1 < c.windows.len() {
                    c.windows.swap(c.focused, c.focused + 1);
                    c.focused += 1;
                }
            }
        });
    }

    /// Pull the first window of the column on the right into the focused column.
    pub fn consume_into_column(&mut self) {
        self.with_active(|ws| {
            let i = ws.focused;
            if i + 1 >= ws.columns.len() {
                return;
            }
            let w = ws.columns[i + 1].windows.remove(0);
            if ws.columns[i + 1].windows.is_empty() {
                ws.columns.remove(i + 1);
            } else {
                let c = &mut ws.columns[i + 1];
                c.focused = c.focused.saturating_sub(1).min(c.windows.len() - 1);
            }
            ws.columns[i].windows.push(w);
        });
    }

    /// Move the focused window out of its column into a new column on the right.
    pub fn expel_from_column(&mut self) {
        let default_width = self.config.default_width;
        self.with_active(|ws| {
            let i = ws.focused;
            let Some(c) = ws.columns.get_mut(i) else { return };
            if c.windows.len() < 2 {
                return;
            }
            let w = c.windows.remove(c.focused);
            c.focused = c.focused.min(c.windows.len() - 1);
            let width = c.width.max(default_width.min(c.width));
            ws.columns.insert(i + 1, Column { windows: vec![w], focused: 0, width, maximized: false, fullscreen: false });
            ws.focused = i + 1;
        });
    }

    pub fn switch_preset_width(&mut self) {
        let presets = self.config.presets.clone();
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                c.maximized = false;
                let next = presets
                    .iter()
                    .copied()
                    .find(|p| *p > c.width + 1e-3)
                    .or_else(|| presets.first().copied())
                    .unwrap_or(0.5);
                c.width = next;
            }
        });
    }

    pub fn adjust_width(&mut self, delta: f64) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                c.maximized = false;
                c.width = (c.width + delta).clamp(0.1, 1.0);
            }
        });
    }

    pub fn toggle_maximize(&mut self) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                c.maximized = !c.maximized;
            }
        });
    }

    pub fn toggle_fullscreen(&mut self) {
        self.with_active(|ws| {
            if let Some(c) = ws.columns.get_mut(ws.focused) {
                c.fullscreen = !c.fullscreen;
            }
        });
    }

    pub fn set_fullscreen(&mut self, w: &W, on: bool) {
        if let Some((mi, wi, ci, k)) = self.find(w) {
            let ws = &mut self.monitors[mi].workspaces[wi];
            ws.columns[ci].fullscreen = on;
            ws.columns[ci].focused = k;
            if on {
                ws.focused = ci;
            }
        }
    }

    pub fn is_fullscreen(&self, w: &W) -> bool {
        self.find(w).map(|(mi, wi, ci, _)| self.monitors[mi].workspaces[wi].columns[ci].fullscreen).unwrap_or(false)
    }

    pub fn center_column(&mut self) {
        let Some((mi, wi)) = self.active_ws_idx() else { return };
        let center = self.config.center_focused;
        self.config.center_focused = true;
        self.scroll_to_focus(mi, wi, Instant::now(), true);
        self.config.center_focused = center;
    }

    pub fn focus_workspace(&mut self, n: usize) {
        let Some(mi) = self.monitors.get(self.active_monitor).map(|_| self.active_monitor) else { return };
        let len = self.monitors[mi].workspaces.len();
        self.monitors[mi].active = n.min(len - 1);
        self.normalize(mi);
    }

    pub fn focus_workspace_up(&mut self) {
        if let Some(m) = self.monitors.get(self.active_monitor) {
            let n = m.active.saturating_sub(1);
            self.focus_workspace(n);
        }
    }

    pub fn focus_workspace_down(&mut self) {
        if let Some(m) = self.monitors.get(self.active_monitor) {
            let n = m.active + 1;
            self.focus_workspace(n);
        }
    }

    /// Move the focused column to workspace `n` of the active monitor.
    pub fn move_column_to_workspace(&mut self, n: usize) {
        let Some((mi, wi)) = self.active_ws_idx() else { return };
        if n == wi {
            return;
        }
        let ws = &mut self.monitors[mi].workspaces[wi];
        if ws.columns.is_empty() {
            return;
        }
        let col = ws.columns.remove(ws.focused);
        ws.focused = ws.focused.min(ws.columns.len().saturating_sub(1));
        while self.monitors[mi].workspaces.len() <= n {
            let w = self.new_ws();
            self.monitors[mi].workspaces.push(w);
        }
        let target = &mut self.monitors[mi].workspaces[n];
        let at = if target.columns.is_empty() { 0 } else { target.focused + 1 };
        target.columns.insert(at, col);
        target.focused = at;
        self.scroll_to_focus(mi, wi, Instant::now(), true);
        self.scroll_to_focus(mi, n, Instant::now(), false);
        self.normalize(mi);
    }

    pub fn move_column_to_workspace_up(&mut self) {
        if let Some(m) = self.monitors.get(self.active_monitor) {
            if m.active > 0 {
                let n = m.active - 1;
                self.move_column_to_workspace(n);
            }
        }
    }

    pub fn move_column_to_workspace_down(&mut self) {
        if let Some(m) = self.monitors.get(self.active_monitor) {
            let n = m.active + 1;
            self.move_column_to_workspace(n);
        }
    }

    pub fn focus_monitor_next(&mut self) {
        if !self.monitors.is_empty() {
            self.active_monitor = (self.active_monitor + 1) % self.monitors.len();
        }
    }

    pub fn move_column_to_monitor_next(&mut self) {
        if self.monitors.len() < 2 {
            return;
        }
        let Some((mi, wi)) = self.active_ws_idx() else { return };
        let ws = &mut self.monitors[mi].workspaces[wi];
        if ws.columns.is_empty() {
            return;
        }
        let col = ws.columns.remove(ws.focused);
        ws.focused = ws.focused.min(ws.columns.len().saturating_sub(1));
        self.scroll_to_focus(mi, wi, Instant::now(), true);
        self.normalize(mi);
        let t = (mi + 1) % self.monitors.len();
        let ti = self.monitors[t].active;
        let target = &mut self.monitors[t].workspaces[ti];
        let at = if target.columns.is_empty() { 0 } else { target.focused + 1 };
        target.columns.insert(at, col);
        target.focused = at;
        self.active_monitor = t;
        self.scroll_to_focus(t, ti, Instant::now(), true);
        self.normalize(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout<u32> {
        let mut l = Layout::new(LayoutConfig { animation_ms: 0, ..Default::default() });
        l.add_monitor("A", (1000, 600));
        l
    }

    fn visible(l: &mut Layout<u32>) -> Vec<(u32, Rect)> {
        l.arrange(Instant::now()).into_iter().filter(|p| p.visible).map(|p| (p.window, p.rect)).collect()
    }

    #[test]
    fn opening_windows_adds_columns_without_resizing_others() {
        let mut l = layout();
        l.add_window(1, &OpenRule::default());
        l.add_window(2, &OpenRule::default());
        let v = visible(&mut l);
        // Two half-width columns fill the screen exactly: (1000-10)*0.5-10 = 485.
        assert_eq!(v, vec![(1, Rect::new(10, 10, 485, 580)), (2, Rect::new(505, 10, 485, 580))]);
        assert_eq!(l.focused_window(), Some(&2));
        // A third window scrolls the view; window 1 goes off-screen, sizes stay.
        l.add_window(3, &OpenRule::default());
        let v = visible(&mut l);
        assert_eq!(v.iter().map(|(w, _)| *w).collect::<Vec<_>>(), vec![2, 3]);
        assert!(v.iter().all(|(_, r)| r.w == 485));
        assert_eq!(v[1].1.x + v[1].1.w, 990);
    }

    #[test]
    fn focus_moves_scroll_the_view_minimally() {
        let mut l = layout();
        for w in 1..=4 {
            l.add_window(w, &OpenRule::default());
        }
        l.focus_first();
        let v = visible(&mut l);
        assert_eq!(v[0], (1, Rect::new(10, 10, 485, 580)));
        l.focus_right();
        // Window 2 was already visible: no scroll.
        assert_eq!(visible(&mut l)[0].0, 1);
        l.focus_right();
        let v = visible(&mut l);
        assert_eq!(v.iter().map(|(w, _)| *w).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn columns_stack_windows_and_split_height() {
        let mut l = layout();
        l.add_window(1, &OpenRule::default());
        l.add_window(2, &OpenRule::default());
        l.focus_left();
        l.consume_into_column();
        let v = visible(&mut l);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].1, Rect::new(10, 10, 485, 285));
        assert_eq!(v[1].1, Rect::new(10, 305, 485, 285));
        l.focus_down();
        assert_eq!(l.focused_window(), Some(&2));
        l.expel_from_column();
        let v = visible(&mut l);
        assert_eq!(v[1], (2, Rect::new(505, 10, 485, 580)));
    }

    #[test]
    fn presets_maximize_and_fullscreen() {
        let mut l = layout();
        l.add_window(1, &OpenRule::default());
        l.switch_preset_width();
        assert_eq!(visible(&mut l)[0].1.w, ((990.0 * (2.0 / 3.0)) as f64).round() as i32 - 10);
        l.switch_preset_width();
        assert_eq!(visible(&mut l)[0].1.w, (990.0f64 / 3.0).round() as i32 - 10);
        l.toggle_maximize();
        assert_eq!(visible(&mut l)[0].1, Rect::new(10, 10, 980, 580));
        l.add_window(2, &OpenRule::default());
        l.toggle_fullscreen();
        let v = visible(&mut l);
        assert_eq!(v, vec![(2, Rect::new(0, 0, 1000, 600))]);
    }

    #[test]
    fn workspaces_are_dynamic() {
        let mut l = layout();
        assert_eq!(l.workspaces().len(), 1);
        l.add_window(1, &OpenRule::default());
        assert_eq!(l.workspaces().len(), 2, "an empty workspace follows the last used one");
        l.move_column_to_workspace_down();
        let ws = l.workspaces();
        // You stay on the (now empty) workspace until you leave it.
        assert_eq!(ws.iter().map(|w| w.windows).collect::<Vec<_>>(), vec![0, 1, 0]);
        assert!(ws[0].active);
        l.focus_workspace_down();
        let ws = l.workspaces();
        assert_eq!(ws.iter().map(|w| w.windows).collect::<Vec<_>>(), vec![1, 0], "the empty workspace is dropped once left");
        assert!(ws[0].active);
        l.add_window(2, &OpenRule::default());
        l.focus_workspace_down();
        assert_eq!(l.focused_window(), None);
        l.add_window(3, &OpenRule::default());
        assert_eq!(l.workspaces().iter().map(|w| w.windows).collect::<Vec<_>>(), vec![2, 1, 0]);
        l.remove_window(&3);
        assert_eq!(l.workspaces().iter().map(|w| w.windows).collect::<Vec<_>>(), vec![2, 0]);
        // Hidden workspaces' windows are not visible.
        l.focus_workspace(0);
        assert_eq!(visible(&mut l).len(), 2);
    }

    #[test]
    fn rules_place_windows() {
        let mut l = layout();
        l.add_window(1, &OpenRule { width: Some(1.0), ..Default::default() });
        assert_eq!(visible(&mut l)[0].1.w, 980);
        l.add_window(2, &OpenRule { workspace: Some(1), ..Default::default() });
        assert_eq!(l.workspaces()[1].windows, 1);
        assert_eq!(l.focused_window(), Some(&1));
    }

    #[test]
    fn working_area_respects_panels() {
        let mut l = layout();
        l.set_working_area("A", Rect::new(0, 30, 1000, 570));
        l.add_window(1, &OpenRule::default());
        assert_eq!(visible(&mut l)[0].1, Rect::new(10, 40, 485, 550));
    }

    #[test]
    fn monitors_hotplug_keeps_windows() {
        let mut l = layout();
        l.add_window(1, &OpenRule::default());
        l.add_monitor("B", (800, 600));
        l.focus_monitor_next();
        l.add_window(2, &OpenRule::default());
        assert_eq!(l.find(&2).unwrap().0, 1);
        l.remove_monitor("B");
        assert!(l.find(&2).is_some());
        assert_eq!(l.windows().count(), 2);
        l.remove_monitor("A");
        assert_eq!(l.windows().count(), 2, "windows survive with no monitor connected");
        l.add_monitor("C", (1000, 600));
        assert_eq!(l.windows().count(), 2);
        assert!(!visible(&mut l).is_empty());
    }

    #[test]
    fn animation_interpolates_and_finishes() {
        let mut l = Layout::new(LayoutConfig { animation_ms: 100, ..Default::default() });
        l.add_monitor("A", (1000, 600));
        for w in 1..=3 {
            l.add_window(w, &OpenRule::default());
        }
        let t0 = Instant::now();
        assert!(l.is_animating(t0));
        let mid = l.arrange(t0 + Duration::from_millis(20));
        let end = l.arrange(t0 + Duration::from_millis(500));
        let x = |ps: &[Placement<u32>], w| ps.iter().find(|p| p.window == w).unwrap().rect.x;
        assert!(x(&mid, 3) > x(&end, 3), "column 3 slides in from the right");
        assert!(!l.is_animating(t0 + Duration::from_millis(500)));
    }

    #[test]
    fn remove_window_fixes_focus() {
        let mut l = layout();
        for w in 1..=3 {
            l.add_window(w, &OpenRule::default());
        }
        l.remove_window(&3);
        assert_eq!(l.focused_window(), Some(&2));
        l.focus_first();
        l.remove_window(&1);
        assert_eq!(l.focused_window(), Some(&2));
        l.remove_window(&2);
        assert_eq!(l.focused_window(), None);
        assert_eq!(l.workspaces().len(), 1);
    }
}
