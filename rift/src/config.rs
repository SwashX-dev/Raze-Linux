//! Rift configuration (TOML).
//!
//! Lookup: `$XDG_CONFIG_HOME/rift/config.toml`, then `/etc/rift/config.toml`, then
//! the built-in defaults (`default-config.toml`, also shipped as the system file).
//! Unknown keys are rejected so typos are reported instead of silently ignored.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_CONFIG: &str = include_str!("../default-config.toml");

/// User files are merged over the built-in defaults table by table (`binds` key by
/// key, so a user file only lists what it changes; bind a key to "none" to remove it).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub input: Input,
    pub layout: LayoutSection,
    pub animations: Animations,
    pub cursor: Cursor,
    #[serde(default)]
    pub outputs: BTreeMap<String, OutputConfig>,
    pub binds: BTreeMap<String, String>,
    #[serde(default)]
    pub window_rules: Vec<WindowRule>,
    /// Shell commands run once the compositor is ready.
    pub spawn_at_startup: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Run xwayland-satellite for X11 applications.
    pub xwayland: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub keyboard: Keyboard,
    pub touchpad: Touchpad,
    pub focus_follows_mouse: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub model: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
    pub numlock: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Touchpad {
    pub tap: bool,
    pub natural_scroll: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutSection {
    pub gap: i32,
    pub presets: Vec<f64>,
    pub default_width: f64,
    pub center_focused: bool,
    pub focus_ring_width: i32,
    pub active_color: String,
    pub inactive_color: String,
    pub background: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Animations {
    pub enabled: bool,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cursor {
    pub theme: String,
    pub size: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    /// Fractional scales are supported (e.g. 1.25).
    pub scale: Option<f64>,
    /// "WIDTHxHEIGHT" or "WIDTHxHEIGHT@HZ".
    pub mode: Option<String>,
    pub position: Option<(i32, i32)>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowRule {
    /// Exact app id, or a prefix when ending in `*`.
    pub app_id: Option<String>,
    /// Substring of the window title.
    pub title: Option<String>,
    pub width: Option<f64>,
    /// 1-based workspace number.
    pub workspace: Option<usize>,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config::parse("").expect("built-in default config is valid")
    }
}

impl Default for Input {
    fn default() -> Self {
        Input { keyboard: Keyboard::default(), touchpad: Touchpad::default(), focus_follows_mouse: false }
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Keyboard {
            layout: String::new(),
            variant: String::new(),
            options: String::new(),
            model: String::new(),
            repeat_rate: 25,
            repeat_delay: 600,
            numlock: false,
        }
    }
}

impl Default for Touchpad {
    fn default() -> Self {
        Touchpad { tap: true, natural_scroll: true }
    }
}

impl Default for LayoutSection {
    fn default() -> Self {
        LayoutSection {
            gap: 10,
            presets: vec![1.0 / 3.0, 0.5, 2.0 / 3.0],
            default_width: 0.5,
            center_focused: false,
            focus_ring_width: 2,
            active_color: "#dc2626".into(),
            inactive_color: "#3f3f46".into(),
            background: "#0f0f10".into(),
        }
    }
}

impl Default for Animations {
    fn default() -> Self {
        Animations { enabled: true, duration_ms: 180 }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor { theme: "Adwaita".into(), size: 24 }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let user = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok()
            .map(|d| d.join("rift/config.toml"));
        user.into_iter().chain([PathBuf::from("/etc/rift/config.toml")]).find(|p| p.exists())
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let mut table: toml::Table = DEFAULT_CONFIG.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let user: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        merge(&mut table, user);
        if let Some(toml::Value::Table(binds)) = table.get_mut("binds") {
            binds.retain(|_, v| v.as_str() != Some("none"));
        }
        let cfg: Config = toml::Value::Table(table).try_into().map_err(|e: toml::de::Error| e.to_string())?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Load the configuration; an invalid file is reported and the defaults are used
    /// so that a typo never leaves the user without a desktop.
    pub fn load() -> (Config, Option<String>) {
        let Some(path) = Self::path() else { return (Config::default(), None) };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Self::parse(&t)) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{}: {e}", path.display()))),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (combo, action) in &self.binds {
            parse_combo(combo)?;
            Action::parse(action).map_err(|e| format!("bind {combo:?}: {e}"))?;
        }
        for c in [&self.layout.active_color, &self.layout.inactive_color, &self.layout.background] {
            parse_color(c).ok_or_else(|| format!("invalid color {c:?}"))?;
        }
        if !(0.05..=1.0).contains(&self.layout.default_width) {
            return Err("layout.default_width must be between 0.05 and 1.0".into());
        }
        for (name, o) in &self.outputs {
            if let Some(s) = o.scale {
                if !(0.25..=8.0).contains(&s) {
                    return Err(format!("output {name}: scale must be between 0.25 and 8"));
                }
            }
            if let Some(m) = &o.mode {
                parse_mode(m).ok_or_else(|| format!("output {name}: invalid mode {m:?}"))?;
            }
        }
        Ok(())
    }

    pub fn rule_for(&self, app_id: &str, title: &str) -> crate::layout::OpenRule {
        let mut rule = crate::layout::OpenRule::default();
        for r in &self.window_rules {
            let app_ok = match &r.app_id {
                None => true,
                Some(p) if p.ends_with('*') => app_id.starts_with(&p[..p.len() - 1]),
                Some(p) => p == app_id,
            };
            let title_ok = r.title.as_ref().map(|t| title.contains(t.as_str())).unwrap_or(true);
            if app_ok && title_ok {
                rule.width = r.width.or(rule.width);
                rule.workspace = r.workspace.map(|w| w.saturating_sub(1)).or(rule.workspace);
                rule.maximized |= r.maximized;
                rule.fullscreen |= r.fullscreen;
            }
        }
        rule
    }
}

/// Recursively merge `over` into `base`; arrays and scalars are replaced.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// `#rrggbb` or `#rrggbbaa` to linear-ish RGBA floats (sRGB values, as the renderer expects).
pub fn parse_color(s: &str) -> Option<[f32; 4]> {
    let h = s.strip_prefix('#')?;
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|b| b as f32 / 255.0);
    match h.len() {
        6 => Some([v(0)?, v(2)?, v(4)?, 1.0]),
        8 => Some([v(0)?, v(2)?, v(4)?, v(6)?]),
        _ => None,
    }
}

pub fn parse_mode(s: &str) -> Option<(i32, i32, Option<u32>)> {
    let (res, hz) = match s.split_once('@') {
        Some((r, h)) => (r, Some((h.parse::<f64>().ok()? * 1000.0).round() as u32)),
        None => (s, None),
    };
    let (w, h) = res.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, hz))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key combination: modifiers plus a lower-cased keysym name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    pub mods: Mods,
    pub key: String,
}

pub fn parse_combo(s: &str) -> Result<Combo, String> {
    let mut mods = Mods::default();
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (key, modifiers) = parts.split_last().ok_or("empty key binding")?;
    for m in modifiers {
        match m.to_ascii_lowercase().as_str() {
            "super" | "logo" | "mod" | "mod4" | "win" => mods.logo = true,
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "mod1" => mods.alt = true,
            "shift" => mods.shift = true,
            other => return Err(format!("unknown modifier {other:?} in {s:?}")),
        }
    }
    if key.is_empty() {
        return Err(format!("missing key in {s:?}"));
    }
    Ok(Combo { mods, key: key.to_ascii_lowercase() })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Spawn(String),
    Close,
    Quit,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    FocusFirst,
    FocusLast,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Consume,
    Expel,
    SwitchWidth,
    AdjustWidth(f64),
    Maximize,
    Fullscreen,
    Center,
    Workspace(usize),
    WorkspaceUp,
    WorkspaceDown,
    MoveToWorkspace(usize),
    MoveToWorkspaceUp,
    MoveToWorkspaceDown,
    FocusMonitorNext,
    MoveToMonitorNext,
    ToggleAnimations,
    ReloadConfig,
    PowerOffMonitors,
    SwitchVt(i32),
}

impl Action {
    pub fn parse(s: &str) -> Result<Action, String> {
        let s = s.trim();
        let (name, arg) = match s.split_once(char::is_whitespace) {
            Some((n, a)) => (n, a.trim()),
            None => (s, ""),
        };
        let num = |a: &str| a.parse::<usize>().map_err(|_| format!("{name}: expected a number, got {a:?}"));
        Ok(match name {
            "spawn" if !arg.is_empty() => Action::Spawn(arg.to_string()),
            "spawn" => return Err("spawn needs a command".into()),
            "close" | "close-window" => Action::Close,
            "quit" | "exit" => Action::Quit,
            "focus-left" | "focus-column-left" => Action::FocusLeft,
            "focus-right" | "focus-column-right" => Action::FocusRight,
            "focus-up" | "focus-window-up" => Action::FocusUp,
            "focus-down" | "focus-window-down" => Action::FocusDown,
            "focus-first" | "focus-column-first" => Action::FocusFirst,
            "focus-last" | "focus-column-last" => Action::FocusLast,
            "move-left" | "move-column-left" => Action::MoveLeft,
            "move-right" | "move-column-right" => Action::MoveRight,
            "move-up" | "move-window-up" => Action::MoveUp,
            "move-down" | "move-window-down" => Action::MoveDown,
            "consume" | "consume-window-into-column" => Action::Consume,
            "expel" | "expel-window-from-column" => Action::Expel,
            "switch-width" | "switch-preset-column-width" => Action::SwitchWidth,
            "width" | "set-column-width" => {
                let v: f64 = arg.trim_end_matches('%').parse().map_err(|_| format!("width: invalid amount {arg:?}"))?;
                Action::AdjustWidth(v / 100.0)
            }
            "maximize" | "maximize-column" => Action::Maximize,
            "fullscreen" | "fullscreen-window" => Action::Fullscreen,
            "center" | "center-column" => Action::Center,
            "workspace" | "focus-workspace" => Action::Workspace(num(arg)?.max(1) - 1),
            "workspace-up" | "focus-workspace-up" => Action::WorkspaceUp,
            "workspace-down" | "focus-workspace-down" => Action::WorkspaceDown,
            "move-to-workspace" | "move-column-to-workspace" => Action::MoveToWorkspace(num(arg)?.max(1) - 1),
            "move-to-workspace-up" => Action::MoveToWorkspaceUp,
            "move-to-workspace-down" => Action::MoveToWorkspaceDown,
            "focus-monitor-next" => Action::FocusMonitorNext,
            "move-to-monitor-next" => Action::MoveToMonitorNext,
            "toggle-animations" => Action::ToggleAnimations,
            "reload-config" => Action::ReloadConfig,
            "power-off-monitors" => Action::PowerOffMonitors,
            "vt" | "switch-vt" => Action::SwitchVt(num(arg)? as i32),
            other => return Err(format!("unknown action {other:?}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let c = Config::default();
        c.validate().unwrap();
        assert!(c.binds.len() > 30);
        assert!(c.binds.values().any(|a| a.starts_with("spawn")));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        assert!(Config::parse("bogus = 1").is_err());
        assert!(Config::parse("[layout]\nactive_color = \"red\"").is_err());
        assert!(Config::parse("[binds]\n\"Super+Q\" = \"explode\"").is_err());
        assert!(Config::parse("[binds]\n\"Hyper+Q\" = \"close\"").is_err());
        assert!(Config::parse("[outputs.HDMI-A-1]\nscale = 20.0").is_err());
        assert!(Config::parse("[outputs.HDMI-A-1]\nmode = \"big\"").is_err());
        let c = Config::parse("[outputs.\"eDP-1\"]\nscale = 1.25\nmode = \"2560x1600@120\"").unwrap();
        assert_eq!(c.outputs["eDP-1"].scale, Some(1.25));
    }

    #[test]
    fn combos_actions_colors_modes() {
        let c = parse_combo("Super+Shift+Return").unwrap();
        assert!(c.mods.logo && c.mods.shift && !c.mods.ctrl);
        assert_eq!(c.key, "return");
        assert_eq!(Action::parse("spawn foot -e htop").unwrap(), Action::Spawn("foot -e htop".into()));
        assert_eq!(Action::parse("workspace 3").unwrap(), Action::Workspace(2));
        assert_eq!(Action::parse("width -10%").unwrap(), Action::AdjustWidth(-0.1));
        assert!(Action::parse("workspace x").is_err());
        assert_eq!(parse_color("#ff000080"), Some([1.0, 0.0, 0.0, 128.0 / 255.0]));
        assert_eq!(parse_mode("1920x1080@59.94"), Some((1920, 1080, Some(59940))));
    }

    #[test]
    fn window_rules() {
        let c = Config::parse(
            "[[window_rules]]\napp_id = \"firefox\"\nwidth = 0.75\n\n[[window_rules]]\napp_id = \"org.gnome.*\"\nworkspace = 2\n\n[[window_rules]]\ntitle = \"Picture-in-Picture\"\nfullscreen = true",
        )
        .unwrap();
        assert_eq!(c.rule_for("firefox", "x").width, Some(0.75));
        assert_eq!(c.rule_for("org.gnome.Nautilus", "x").workspace, Some(1));
        assert!(c.rule_for("firefox", "Picture-in-Picture").fullscreen);
        assert_eq!(c.rule_for("foot", "x").width, None);
    }
}
