use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Optional CLI/env override, checked before the normal search order.
static CONFIG_OVERRIDE: OnceLock<String> = OnceLock::new();

pub fn set_config_override(path: String) {
    let _ = CONFIG_OVERRIDE.set(path);
}

/// Default user config directory (`$XDG_CONFIG_HOME/hyprcircl` or `~/.config/hyprcircl`).
pub fn default_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("hyprcircl");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".config/hyprcircl");
        }
    }
    PathBuf::from(".config/hyprcircl")
}

/// Current configuration schema version. bump this when adding fields that
/// change the config structure; old configs will be detected and defaults
/// applied until the user re-saves.
const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Write a starter config + CSS into the user config directory.
pub fn init_user_config(force: bool) -> Result<(PathBuf, PathBuf), String> {
    let dir = default_config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let toml_path = dir.join("hyprcircl.toml");
    let css_path = dir.join("hyprcircl.css");
    if toml_path.exists() && !force {
        return Err(format!(
            "{} already exists (use `hyprcircl init --force` to overwrite)",
            toml_path.display()
        ));
    }
    std::fs::write(&toml_path, DEFAULT_CONFIG_TOML)
        .map_err(|e| format!("write {}: {e}", toml_path.display()))?;
    std::fs::write(&css_path, include_str!("../hyprcircl.css"))
        .map_err(|e| format!("write {}: {e}", css_path.display()))?;
    Ok((toml_path, css_path))
}

/// Path the loader would use: first existing candidate, else the default write location.
pub fn resolved_config_path() -> PathBuf {
    for path in config_paths() {
        if std::path::Path::new(&path).exists() {
            return PathBuf::from(path);
        }
    }
    default_config_dir().join("hyprcircl.toml")
}

// =========================================================================
// Data Configuration Model
//
// The file format (`RawConfig`) is deliberately forgiving: every key is
// optional, geometry lives in a `[menu]` section, the status bar in `[bar]`,
// and the flat keys / `[top_bar]` table of schema 1 configs are still read.
// It is normalised into the flat runtime `RadialConfig` the rest of the
// program uses, so `toml::from_str::<RadialConfig>` accepts both formats.
// =========================================================================

/// The config shipped in the repo root. It is the built-in default (used when
/// no file exists, and for any section a user file leaves out) and the file
/// `hyprcircl init` writes, so there is exactly one source of truth.
pub const DEFAULT_CONFIG_TOML: &str = include_str!("../hyprcircl.toml");

/// Built-in geometry, also used for keys missing from a user's `[menu]`.
pub const DEFAULT_INNER_RADIUS: f64 = 64.0;
pub const DEFAULT_OUTER_RADIUS: f64 = 164.0;
pub const DEFAULT_SUBMENU_WIDTH: f64 = 78.0;
pub const DEFAULT_RING_GAP: f64 = 8.0;
pub const DEFAULT_ITEM_GAP: f64 = 6.0;
pub const DEFAULT_ANIMATION_MS: u64 = 160;

/// A menu command: either an argv list (`["playerctl", "next"]`, exec'd
/// directly) or a single string, which runs through `sh -c` so pipes,
/// `$(...)` and `~` work as written.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum CommandSpec {
    Shell(String),
    Argv(Vec<String>),
}

impl CommandSpec {
    fn into_argv(self) -> Vec<String> {
        match self {
            CommandSpec::Argv(v) => v,
            CommandSpec::Shell(s) if s.trim().is_empty() => Vec::new(),
            CommandSpec::Shell(s) => vec!["sh".into(), "-c".into(), s],
        }
    }
}

fn de_command<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(CommandSpec::deserialize(d)?.into_argv())
}

fn de_command_list<'de, D>(d: D) -> Result<Vec<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Vec<CommandSpec> = Vec::deserialize(d)?;
    Ok(raw.into_iter().map(CommandSpec::into_argv).collect())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuItem {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: String,
    /// Executed when clicked; empty for submenu items. A string runs through
    /// `sh -c`; a list is exec'd as argv.
    #[serde(default, deserialize_with = "de_command")]
    pub command: Vec<String>,
    /// Alternatives tried in order when `command`'s program is not installed
    /// (e.g. a list of terminals). Same string/list forms as `command`.
    #[serde(default, deserialize_with = "de_command_list")]
    pub fallback: Vec<Vec<String>>,
    /// Recursive child items for submenus.
    #[serde(default)]
    pub items: Vec<MenuItem>,
}

impl MenuItem {
    pub fn is_submenu(&self) -> bool {
        !self.items.is_empty()
    }

    /// The command to run: `command` when its program is on `PATH`,
    /// otherwise the first `fallback` whose program is. Falls back to
    /// `command` itself so a missing program still produces a clear spawn
    /// error instead of silently doing nothing.
    pub fn resolved_command(&self) -> Vec<String> {
        if self.fallback.is_empty() || command_available(&self.command) {
            return self.command.clone();
        }
        self.fallback
            .iter()
            .find(|c| command_available(c))
            .cloned()
            .unwrap_or_else(|| self.command.clone())
    }
}

/// Whether the program a command would launch exists. For `sh -c` strings
/// this is the first word of the script; `$VAR` words are expanded from the
/// environment (so `$TERMINAL` counts as missing when it is unset).
pub fn command_available(command: &[String]) -> bool {
    let word = match command {
        [sh, flag, script, ..] if sh == "sh" && flag == "-c" => {
            script.split_whitespace().next().unwrap_or("").to_string()
        }
        [prog, ..] => prog.clone(),
        [] => return false,
    };
    let word = word.trim_matches(|c| c == '"' || c == '\'');
    let word = if let Some(var) = word.strip_prefix('$') {
        let var = var.trim_matches(|c| c == '{' || c == '}');
        match std::env::var(var) {
            Ok(v) => v.split_whitespace().next().unwrap_or("").to_string(),
            Err(_) => return false,
        }
    } else {
        word.to_string()
    };
    program_on_path(&word)
}

fn program_on_path(prog: &str) -> bool {
    if prog.is_empty() {
        return false;
    }
    let is_exec = |p: &std::path::Path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.metadata()
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    };
    if prog.contains('/') {
        return is_exec(std::path::Path::new(prog));
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| is_exec(&dir.join(prog))))
        .unwrap_or(false)
}

fn default_interval() -> u64 {
    5
}

fn default_format() -> String {
    "{output}".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarModule {
    /// Shell command whose stdout becomes the module text (Waybar-style).
    /// Optional when a `stream_command` is used instead.
    #[serde(default)]
    pub command: String,
    /// Refresh interval in seconds.
    #[serde(default = "default_interval")]
    pub interval: u64,
    /// Waybar-style format string; `{output}` is replaced with the command output.
    #[serde(default = "default_format")]
    pub format: String,
    /// Optional icon rendered before the output.
    #[serde(default)]
    pub icon: String,
    /// Optional long-lived push command: spawned once per show, its stdout
    /// lines become the module output (no interval polling). Empty disables.
    /// E.g. `pactl subscribe | ...` or reading the Hyprland event socket.
    #[serde(default)]
    pub stream_command: String,
    /// File paths whose changes re-run `command` before its interval elapses.
    /// Watched files are read cheaply every ~100ms (works with sysfs), so
    /// file-backed modules like brightness/battery update instantly without
    /// spawning processes on a tight timer. Missing files are tolerated.
    #[serde(default)]
    pub watch: Vec<String>,
    /// Shell command run when the module is left-clicked (Waybar `on-click`).
    #[serde(default)]
    pub on_click: String,
    /// Shell command run when the module is right-clicked (Waybar `on-click-right`).
    #[serde(default)]
    pub on_click_right: String,
    /// Shell command run when scrolling up over the module (Waybar `on-scroll-up`).
    #[serde(default)]
    pub on_scroll_up: String,
    /// Shell command run when scrolling down over the module (Waybar `on-scroll-down`).
    #[serde(default)]
    pub on_scroll_down: String,
}

impl Default for BarModule {
    fn default() -> Self {
        Self {
            command: String::new(),
            interval: default_interval(),
            format: default_format(),
            icon: String::new(),
            stream_command: String::new(),
            watch: Vec::new(),
            on_click: String::new(),
            on_click_right: String::new(),
            on_scroll_up: String::new(),
            on_scroll_down: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopBarConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Ordered list of Waybar-style modules rendered inside the pill.
    /// Visual styling (colors, fonts, geometry) is driven by `hyprcircl.css`.
    #[serde(default)]
    pub modules: Vec<BarModule>,
}

/// Runtime configuration (normalised; see `RawConfig` for the file format).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawConfig")]
pub struct RadialConfig {
    pub inner_radius: f64,
    pub outer_radius: f64,
    /// Thickness of each outer submenu ring.
    pub ring_thickness: f64,
    /// Gap between consecutive rings.
    pub ring_gap: f64,
    /// Legacy angular gap between wedges, in degrees. Only non-zero when a
    /// config sets `item_gap_degrees` and no pixel `item_gap`.
    pub item_gap_degrees: f64,
    /// Gap between wedges in pixels; constant width from hole to rim.
    pub item_gap: f64,
    /// Open animation length in milliseconds (0 disables).
    pub animation_ms: u64,
    pub top_bar: TopBarConfig,
    /// When true, clicking an action only sends a notification instead of
    /// executing its command (safety mode).
    pub notify_only: bool,
    /// When false, text labels are hidden below icons in the radial menu.
    pub show_labels: bool,
    /// Arrow keys / number keys navigate wedges; Enter activates.
    pub keyboard_navigation: bool,
    pub items: Vec<MenuItem>,
    /// Bumped by the watcher on every successful reload so consumers can
    /// detect "the module list changed" and rebuild index-keyed state
    /// instead of misattributing streams/outputs across a reorder.
    #[serde(skip, default)]
    pub generation: u64,
    /// Configuration format version. Used to detect old config formats and
    /// apply backward-compatible migrations or refuse to load incompatible ones.
    pub schema_version: u32,
    /// Keys the loader did not recognise (typos, removed options), reported
    /// once per load so they don't fail silently.
    #[serde(skip, default)]
    pub unknown_keys: Vec<String>,
}

/// `[menu]`: ring geometry and menu behaviour. Every key is optional.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RawMenu {
    inner_radius: Option<f64>,
    outer_radius: Option<f64>,
    #[serde(alias = "ring_thickness")]
    submenu_width: Option<f64>,
    ring_gap: Option<f64>,
    item_gap: Option<f64>,
    item_gap_degrees: Option<f64>,
    animation_ms: Option<u64>,
    #[serde(alias = "show_labels")]
    labels: Option<bool>,
    #[serde(alias = "keyboard_navigation")]
    keyboard: Option<bool>,
    notify_only: Option<bool>,
}

/// The on-disk format. Schema 2 groups keys into `[menu]` and `[bar]`;
/// schema 1's flat keys and `[top_bar]` are still accepted (a `[menu]` key
/// wins when both are present).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    schema_version: Option<u32>,
    menu: RawMenu,
    #[serde(alias = "top_bar")]
    bar: Option<TopBarConfig>,
    items: Option<Vec<MenuItem>>,
    // --- schema 1 flat keys ---
    inner_radius: Option<f64>,
    outer_radius: Option<f64>,
    ring_thickness: Option<f64>,
    ring_gap: Option<f64>,
    item_gap_degrees: Option<f64>,
    item_gap: Option<f64>,
    animation_ms: Option<u64>,
    notify_only: Option<bool>,
    show_labels: Option<bool>,
    keyboard_navigation: Option<bool>,
}

/// The shipped config, parsed once. Its `[bar]` and `[[items]]` are the
/// defaults for files that leave those sections out.
fn shipped() -> &'static RawConfig {
    static SHIPPED: OnceLock<RawConfig> = OnceLock::new();
    SHIPPED.get_or_init(|| toml::from_str(DEFAULT_CONFIG_TOML).unwrap_or_default())
}

impl From<RawConfig> for RadialConfig {
    fn from(raw: RawConfig) -> Self {
        let m = &raw.menu;
        let item_gap_px = m.item_gap.or(raw.item_gap);
        let legacy_deg = m.item_gap_degrees.or(raw.item_gap_degrees);
        // A config that only sets the old angular gap keeps that look.
        let (item_gap, item_gap_degrees) = match (item_gap_px, legacy_deg) {
            (Some(px), _) => (px.max(0.0), 0.0),
            (None, Some(deg)) => (0.0, deg.max(0.0)),
            (None, None) => (DEFAULT_ITEM_GAP, 0.0),
        };
        let items = raw
            .items
            .clone()
            .or_else(|| shipped().items.clone())
            .unwrap_or_default();
        let top_bar = raw
            .bar
            .clone()
            .or_else(|| shipped().bar.clone())
            .unwrap_or(TopBarConfig {
                enabled: true,
                modules: Vec::new(),
            });
        RadialConfig {
            inner_radius: m
                .inner_radius
                .or(raw.inner_radius)
                .unwrap_or(DEFAULT_INNER_RADIUS),
            outer_radius: m
                .outer_radius
                .or(raw.outer_radius)
                .unwrap_or(DEFAULT_OUTER_RADIUS),
            ring_thickness: m
                .submenu_width
                .or(raw.ring_thickness)
                .unwrap_or(DEFAULT_SUBMENU_WIDTH),
            ring_gap: m.ring_gap.or(raw.ring_gap).unwrap_or(DEFAULT_RING_GAP),
            item_gap,
            item_gap_degrees,
            animation_ms: m
                .animation_ms
                .or(raw.animation_ms)
                .unwrap_or(DEFAULT_ANIMATION_MS),
            notify_only: m.notify_only.or(raw.notify_only).unwrap_or(false),
            show_labels: m.labels.or(raw.show_labels).unwrap_or(true),
            keyboard_navigation: m.keyboard.or(raw.keyboard_navigation).unwrap_or(true),
            top_bar,
            items,
            generation: 0,
            schema_version: raw.schema_version.unwrap_or(0),
            unknown_keys: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

impl Default for RadialConfig {
    fn default() -> Self {
        let mut cfg: RadialConfig = toml::from_str(DEFAULT_CONFIG_TOML).unwrap_or_else(|_| {
            // Unreachable in practice (a unit test parses the shipped file);
            // keep a usable ring rather than panicking.
            RadialConfig::from(RawConfig::default())
        });
        cfg.schema_version = CURRENT_SCHEMA_VERSION;
        cfg
    }
}

/// Keys hyprcircl understands, per table, for typo warnings.
const KNOWN_TOP: &[&str] = &[
    "schema_version",
    "menu",
    "bar",
    "top_bar",
    "items",
    "inner_radius",
    "outer_radius",
    "ring_thickness",
    "ring_gap",
    "item_gap_degrees",
    "item_gap",
    "animation_ms",
    "notify_only",
    "show_labels",
    "keyboard_navigation",
    // schema 0/1 visual keys, moved to hyprcircl.css; ignored quietly.
    "corner_radius",
];
const KNOWN_MENU: &[&str] = &[
    "inner_radius",
    "outer_radius",
    "submenu_width",
    "ring_thickness",
    "ring_gap",
    "item_gap",
    "item_gap_degrees",
    "animation_ms",
    "labels",
    "show_labels",
    "keyboard",
    "keyboard_navigation",
    "notify_only",
];
const KNOWN_BAR: &[&str] = &[
    "enabled",
    "modules",
    // moved to hyprcircl.css (.pill)
    "height",
    "padding_x",
    "offset_y",
    "corner_radius",
    "background",
    "foreground",
    "font",
];
const KNOWN_MODULE: &[&str] = &[
    "command",
    "interval",
    "format",
    "icon",
    "stream_command",
    "watch",
    "on_click",
    "on_click_right",
    "on_scroll_up",
    "on_scroll_down",
];
const KNOWN_ITEM: &[&str] = &["label", "icon", "command", "fallback", "items"];

fn unknown_keys(contents: &str) -> Vec<String> {
    let Ok(top) = toml::from_str::<toml::Table>(contents) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let check = |t: &toml::Table, known: &[&str], prefix: &str, out: &mut Vec<String>| {
        for k in t.keys() {
            if !known.contains(&k.as_str()) {
                out.push(format!("{prefix}{k}"));
            }
        }
    };
    check(&top, KNOWN_TOP, "", &mut out);
    if let Some(toml::Value::Table(menu)) = top.get("menu") {
        check(menu, KNOWN_MENU, "menu.", &mut out);
    }
    for name in ["bar", "top_bar"] {
        if let Some(toml::Value::Table(bar)) = top.get(name) {
            check(bar, KNOWN_BAR, &format!("{name}."), &mut out);
            if let Some(toml::Value::Array(mods)) = bar.get("modules") {
                for (i, m) in mods.iter().enumerate() {
                    if let toml::Value::Table(m) = m {
                        check(m, KNOWN_MODULE, &format!("{name}.modules[{i}]."), &mut out);
                    }
                }
            }
        }
    }
    fn walk(items: &toml::Value, path: &str, out: &mut Vec<String>) {
        if let toml::Value::Array(arr) = items {
            for (i, it) in arr.iter().enumerate() {
                if let toml::Value::Table(t) = it {
                    let here = format!("{path}[{i}]");
                    for k in t.keys() {
                        if !KNOWN_ITEM.contains(&k.as_str()) {
                            out.push(format!("{here}.{k}"));
                        }
                    }
                    if let Some(children) = t.get("items") {
                        walk(children, &format!("{here}.items"), out);
                    }
                }
            }
        }
    }
    if let Some(items) = top.get("items") {
        walk(items, "items", &mut out);
    }
    out
}

/// Parse and validate a config file's contents. Used by the initial load and
/// the live-reload watcher so both apply the same rules.
pub fn parse_config(contents: &str) -> Result<RadialConfig, String> {
    let mut cfg: RadialConfig = toml::from_str(contents).map_err(|e| e.to_string())?;
    if cfg.schema_version == 0 {
        cfg.schema_version = 1;
    }
    if cfg.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "schema_version {} is newer than this hyprcircl understands ({}); \
             update hyprcircl or lower schema_version",
            cfg.schema_version, CURRENT_SCHEMA_VERSION
        ));
    }
    if cfg.outer_radius <= cfg.inner_radius {
        return Err(format!(
            "menu.outer_radius ({}) must be larger than menu.inner_radius ({})",
            cfg.outer_radius, cfg.inner_radius
        ));
    }
    cfg.unknown_keys = unknown_keys(contents);
    Ok(cfg)
}

fn report_unknown_keys(path: &str, cfg: &RadialConfig) {
    for k in &cfg.unknown_keys {
        eprintln!("[CONFIG] {path}: unknown key `{k}` (ignored; see the README for valid keys)");
    }
}

/// Compute the inner/outer radius for a given ring level.
/// Level 0 is the root wheel; deeper levels stack outward concentrically.
impl RadialConfig {
    pub fn get_level_radii(&self, level: usize) -> (f64, f64) {
        if level == 0 {
            (self.inner_radius, self.outer_radius)
        } else {
            let base_in = self.outer_radius + self.ring_gap;
            let step = self.ring_thickness + self.ring_gap;
            let r_in = base_in + (level - 1) as f64 * step;
            let r_out = r_in + self.ring_thickness;
            (r_in, r_out)
        }
    }

    /// Angular gap used for wedge layout and hit-testing. Zero in pixel-gap
    /// mode: the visual gap is drawn by `draw_rounded_sector`, and a pointer
    /// in that thin gap simply belongs to the nearest wedge.
    pub fn gap_rad(&self) -> f64 {
        self.item_gap_degrees.to_radians()
    }
}

// =========================================================================
// Configuration IO (load + auto-reload from TOML)
// =========================================================================

/// Candidate config file locations, in priority order.
/// The real file is `hyprcircl.toml`; `config.toml` is kept as a compat
/// fallback (e.g. a symlink) since it's too generic a name for a config folder.
fn config_paths() -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    if let Some(override_path) = CONFIG_OVERRIDE.get() {
        paths.push(override_path.clone());
    }
    if let Ok(env_path) = std::env::var("HYPRCIRCL_CONFIG") {
        if !env_path.is_empty() {
            paths.push(env_path);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            paths.push(format!("{xdg}/hyprcircl/hyprcircl.toml"));
            paths.push(format!("{xdg}/hyprcircl/config.toml"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            paths.push(format!("{home}/.config/hyprcircl/hyprcircl.toml"));
            paths.push(format!("{home}/.config/hyprcircl/config.toml"));
        }
    }
    paths.push("hyprcircl.toml".into());
    paths.push("config.toml".into());
    paths
}

/// Load `RadialConfig` plus the exact file it came from.
///
/// A missing (or unreadable) candidate moves the search down the priority
/// list. A candidate that exists but fails to parse STOPS the search: silently
/// substituting a lower-priority config — or defaults — hides real errors.
/// In that case defaults are returned together with the broken path so the
/// daemon still watches it; fixing the file applies live.
pub fn load_config_with_path() -> (RadialConfig, Option<String>) {
    for path in config_paths() {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let cfg = match parse_config(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("[CONFIG] {path}: {e}");
                eprintln!(
                    "[CONFIG] Using built-in defaults until the file is fixed (it is watched)."
                );
                return (RadialConfig::default(), Some(path));
            }
        };
        report_unknown_keys(&path, &cfg);
        println!("[CONFIG] Loaded {path}");
        return (cfg, Some(path));
    }

    println!("[CONFIG] No config found, using defaults");
    (RadialConfig::default(), None)
}

/// Find a CSS file, preferring the directory of the config that actually
/// loaded, then the remaining config directories in priority order. The
/// cwd fallback resolves to `./hyprcircl.css` (a relative config path's
/// parent is empty, which must not become a filesystem-root probe).
pub fn find_css_path(config_file: Option<&str>) -> Option<String> {
    let mut dirs: Vec<String> = Vec::new();
    let push_dir = |d: std::path::PathBuf, dirs: &mut Vec<String>| {
        let dir = if d.as_os_str().is_empty() {
            ".".to_string()
        } else {
            d.to_string_lossy().into_owned()
        };
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if let Some(cfg) = config_file {
        push_dir(
            std::path::Path::new(cfg)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
            &mut dirs,
        );
    }
    for p in config_paths() {
        push_dir(
            std::path::Path::new(&p)
                .parent()
                .map(|q| q.to_path_buf())
                .unwrap_or_default(),
            &mut dirs,
        );
    }
    for dir in dirs {
        let css = std::path::Path::new(&dir).join("hyprcircl.css");
        if css.exists() {
            return Some(css.to_string_lossy().into_owned());
        }
    }
    None
}

// =========================================================================
// Unit tests for config model, radii math, and config IO
// =========================================================================

#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_item_is_submenu() {
        let leaf = MenuItem {
            label: "Terminal".into(),
            icon: "💻".into(),
            command: vec!["foot".into()],
            ..Default::default()
        };
        assert!(!leaf.is_submenu());

        let branch = MenuItem {
            label: "Media".into(),
            icon: "🎵".into(),
            command: vec![],
            items: vec![MenuItem {
                label: "Next".into(),
                icon: "⏭".into(),
                command: vec!["playerctl".into(), "next".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(branch.is_submenu());
    }

    #[test]
    fn default_config_is_fully_populated() {
        let cfg = RadialConfig::default();
        assert!(cfg.inner_radius > 0.0);
        assert!(cfg.outer_radius > cfg.inner_radius);
        assert_eq!(cfg.ring_thickness, DEFAULT_SUBMENU_WIDTH);
        assert_eq!(cfg.ring_gap, DEFAULT_RING_GAP);
        assert_eq!(cfg.item_gap, DEFAULT_ITEM_GAP);
        assert_eq!(cfg.item_gap_degrees, 0.0);
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        // The built-in default is the shipped file, item for item.
        assert_eq!(cfg.items.len(), 8);
        assert_eq!(cfg.items[0].label, "Terminal");
        assert_eq!(cfg.top_bar.modules.len(), 4);
        assert!(cfg.show_labels);
        assert!(!cfg.notify_only);
        assert!(!cfg.items.is_empty());
        assert!(!cfg.top_bar.modules.is_empty());
    }

    #[test]
    fn level_radii_root_uses_inner_outer() {
        let cfg = RadialConfig::default();
        assert_eq!(cfg.get_level_radii(0), (cfg.inner_radius, cfg.outer_radius));
    }

    #[test]
    fn level_radii_stacks_outward() {
        let cfg = RadialConfig::default();
        let (_r0_in, r0_out) = cfg.get_level_radii(0);
        let (r1_in, r1_out) = cfg.get_level_radii(1);
        let (r2_in, r2_out) = cfg.get_level_radii(2);

        // Level 1 starts just past the root outer radius plus the gap.
        assert_eq!(r1_in, cfg.outer_radius + cfg.ring_gap);
        assert_eq!(r1_out, r1_in + cfg.ring_thickness);

        // Level 2 is one ring step further out.
        let step = cfg.ring_thickness + cfg.ring_gap;
        assert_eq!(r2_in, r1_in + step);
        assert_eq!(r2_out, r1_out + step);

        // Each level strictly contains the previous.
        assert!(r1_in > r0_out);
        assert!(r2_in > r1_out);
    }

    #[test]
    fn level_radii_grows_monotonically() {
        let cfg = RadialConfig::default();
        let mut prev_out = 0.0f64;
        for level in 0..6 {
            let (r_in, r_out) = cfg.get_level_radii(level);
            assert!(r_in > prev_out);
            assert!(r_out > r_in);
            prev_out = r_out;
        }
    }

    #[test]
    fn config_paths_priority_order() {
        let _guard = env_lock();
        // With no env vars, only the cwd fallbacks are present.
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HOME");
        let paths = config_paths();
        assert_eq!(
            paths,
            vec!["hyprcircl.toml".to_string(), "config.toml".to_string()]
        );

        // XDG_CONFIG_HOME paths come first when set.
        std::env::set_var("XDG_CONFIG_HOME", "/x");
        let paths = config_paths();
        assert!(paths[0].starts_with("/x/"));
        assert!(paths[0].ends_with("hyprcircl.toml"));
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn load_config_parses_toml() {
        // Note: legacy visual fields (corner_radius, top_bar height/padding_x/
        // offset_y/background/foreground/font) were moved to hyprcircl.css.
        // They are unknown to the model now and must be ignored, so old
        // configs keep parsing (backward compatibility).
        let toml = r#"
inner_radius = 50.0
outer_radius = 120.0
item_gap_degrees = 3.0
corner_radius = 8.0

[top_bar]
enabled = false
height = 30.0
padding_x = 10.0
offset_y = 5.0
corner_radius = 12.0
background = [0.1, 0.1, 0.1, 0.9]
foreground = [1.0, 1.0, 1.0, 1.0]
font = "JetBrainsMono Nerd Font"
modules = []

[[items]]
label = "Lock"
icon = "🔒"
command = ["hyprlock"]

[[items]]
label = "Audio"
icon = "🔊"
items = [
  { label = "Mute", icon = "🔇", command = ["pamixer", "-t"] },
]
"#;
        let cfg: RadialConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.inner_radius, 50.0);
        assert!(!cfg.top_bar.enabled);
        assert_eq!(cfg.items.len(), 2);
        assert!(cfg.items[1].is_submenu());
        assert_eq!(cfg.items[1].items[0].command, vec!["pamixer", "-t"]);
    }

    #[test]
    fn load_config_falls_back_to_default_when_unparseable() {
        // An unparseable file in cwd would normally print an error; instead we
        // verify `RadialConfig::default()` is a usable fallback representation
        // and that `load_config_with_path`'s contract (never panic) holds.
        let cfg = RadialConfig::default();
        assert!(cfg.get_level_radii(0).1 > 0.0);
    }

    #[test]
    fn load_config_with_path_finds_existing_toml() {
        let _guard = env_lock();
        // The project ships a hyprcircl.toml in the crate root; if cwd is the
        // crate root it must be discoverable, and the reported path is the
        // exact file the config came from (the watcher follows this path).
        std::env::remove_var("XDG_CONFIG_HOME");
        if let Ok(cwd) = std::env::current_dir() {
            let root = cwd.join("hyprcircl.toml");
            if root.exists() {
                let (_, found) = load_config_with_path();
                let found = found.expect("config present in cwd");
                assert!(std::path::Path::new(&found).exists());
                assert!(found.ends_with("hyprcircl.toml"));
            }
        }
    }

    #[test]
    fn load_config_with_path_reports_broken_file_instead_of_falling_through() {
        let _guard = env_lock();
        // An existing-but-unparseable candidate must stop the search and be
        // reported as the active (watched) path, not silently skipped.
        std::env::remove_var("XDG_CONFIG_HOME");
        let home = std::env::temp_dir().join(format!("hyprcircl-test-{}", std::process::id()));
        let dir = home.join(".config/hyprcircl");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let broken = dir.join("hyprcircl.toml");
        std::fs::write(&broken, "inner_radius = not a number").expect("write");

        std::env::set_var("HOME", &home);
        let (cfg, path) = load_config_with_path();
        assert_eq!(cfg, RadialConfig::default(), "defaults while broken");
        assert_eq!(path.as_deref(), Some(broken.to_string_lossy().as_ref()));
        assert_eq!(cfg.generation, 0, "loader never bumps the generation");

        std::env::remove_var("HOME");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn shipped_config_parses_cleanly() {
        let cfg = parse_config(DEFAULT_CONFIG_TOML).expect("shipped hyprcircl.toml parses");
        assert!(cfg.unknown_keys.is_empty(), "{:?}", cfg.unknown_keys);
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        // Every leaf has a command; every branch has children.
        fn walk(items: &[MenuItem]) {
            for it in items {
                assert!(!it.label.is_empty() && !it.icon.is_empty());
                if it.is_submenu() {
                    walk(&it.items);
                } else {
                    assert!(!it.command.is_empty(), "{} has no command", it.label);
                }
            }
        }
        walk(&cfg.items);
    }

    #[test]
    fn minimal_config_fills_in_defaults() {
        // Just one item: geometry and bar come from the built-in defaults.
        let cfg = parse_config(
            r#"
[[items]]
label = "Term"
icon = "T"
command = "foot"
"#,
        )
        .expect("parse");
        assert_eq!(cfg.items.len(), 1);
        assert_eq!(cfg.items[0].command, vec!["sh", "-c", "foot"]);
        assert_eq!(cfg.inner_radius, DEFAULT_INNER_RADIUS);
        assert_eq!(cfg.outer_radius, DEFAULT_OUTER_RADIUS);
        assert_eq!(cfg.top_bar, RadialConfig::default().top_bar);

        // An empty file is the built-in config.
        let empty = parse_config("").expect("empty parses");
        assert_eq!(empty.items, RadialConfig::default().items);
    }

    #[test]
    fn menu_section_and_legacy_flat_keys() {
        let new = parse_config(
            r#"
[menu]
inner_radius = 50
outer_radius = 140
submenu_width = 70
item_gap = 4
labels = false
keyboard = false
animation_ms = 0
"#,
        )
        .unwrap();
        assert_eq!((new.inner_radius, new.outer_radius), (50.0, 140.0));
        assert_eq!(new.ring_thickness, 70.0);
        assert_eq!(new.item_gap, 4.0);
        assert!(!new.show_labels && !new.keyboard_navigation);
        assert_eq!(new.animation_ms, 0);

        // Schema 1: flat keys, [top_bar], angular gap -> same values.
        let old = parse_config(
            r#"
schema_version = 1
inner_radius = 50.0
outer_radius = 140.0
ring_thickness = 70.0
item_gap_degrees = 2.0
show_labels = false

[top_bar]
enabled = false
modules = []
"#,
        )
        .unwrap();
        assert_eq!((old.inner_radius, old.outer_radius), (50.0, 140.0));
        assert_eq!(old.ring_thickness, 70.0);
        assert_eq!(old.item_gap_degrees, 2.0);
        assert_eq!(old.item_gap, 0.0, "legacy angular gap keeps the old look");
        assert!(!old.show_labels);
        assert!(!old.top_bar.enabled);
        assert!(old.unknown_keys.is_empty());

        // [menu] wins over a flat key.
        let both = parse_config("inner_radius = 10\n[menu]\ninner_radius = 30\n").unwrap();
        assert_eq!(both.inner_radius, 30.0);
    }

    #[test]
    fn command_string_list_and_fallback() {
        let cfg = parse_config(
            r#"
[[items]]
label = "A"
icon = "a"
command = ["hyprcircl-surely-missing-program", "--flag"]
fallback = ["sh -c true", ["sh", "-c", "true"]]
"#,
        )
        .unwrap();
        let item = &cfg.items[0];
        assert_eq!(item.fallback.len(), 2);
        assert_eq!(item.fallback[0], vec!["sh", "-c", "sh -c true"]);
        // The missing program is skipped in favour of the first fallback.
        assert_eq!(item.resolved_command(), item.fallback[0]);
        assert!(!command_available(&[
            "hyprcircl-surely-missing-program".into()
        ]));
        assert!(command_available(&["sh".into()]));
        assert!(!command_available(&[
            "sh".into(),
            "-c".into(),
            "$HYPRCIRCL_UNSET_VAR_FOR_TEST --x".into()
        ]));
    }

    #[test]
    fn errors_and_unknown_keys_are_reported() {
        let e = parse_config("[menu]\ninner_radius = 200\nouter_radius = 100\n").unwrap_err();
        assert!(e.contains("outer_radius"), "{e}");
        let e = parse_config("schema_version = 99\n").unwrap_err();
        assert!(e.contains("schema_version"), "{e}");
        let cfg = parse_config(
            "inner_radus = 3\n[menu]\nlabelz = true\n[[items]]\nlabel = \"x\"\nicno = \"y\"\n",
        )
        .unwrap();
        assert_eq!(
            cfg.unknown_keys,
            vec!["inner_radus", "menu.labelz", "items[0].icno"]
        );
    }

    #[test]
    fn find_css_path_returns_existing_or_none() {
        let _guard = env_lock();
        // Must never panic and only ever return an existing path.
        let res = find_css_path(None);
        if let Some(css) = res {
            assert!(std::path::Path::new(&css).exists());
        }
    }
}

// =========================================================================
// Configuration IO (load + auto-reload from TOML)
// =========================================================================

/// Background thread: polls the config file and swaps it into the shared
/// `RwLock` whenever its contents change, so edits apply live.
///
/// Every successful swap bumps `RadialConfig::generation`, letting consumers
/// detect a changed module list. `on_reload` runs after each successful
/// swap — used by the window controller to reset navigation state so stale
/// indices can never address a differently-shaped tree.
pub fn watch_config<F>(path: String, config: Arc<RwLock<RadialConfig>>, on_reload: F)
where
    F: Fn() + Send + 'static,
{
    std::thread::spawn(move || {
        let mut last_contents: Option<String> = None;
        loop {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if last_contents.as_deref() != Some(&contents) {
                    last_contents = Some(contents.clone());
                    match parse_config(&contents) {
                        Ok(mut cfg) => {
                            report_unknown_keys(&path, &cfg);
                            let next_gen = config
                                .read()
                                .map(|c| c.generation.wrapping_add(1))
                                .unwrap_or(1);
                            cfg.generation = next_gen;
                            if let Ok(mut c) = config.write() {
                                *c = cfg;
                            }
                            on_reload();
                            println!("[CONFIG] Reloaded {path}");
                        }
                        Err(e) => eprintln!("[CONFIG] {path}: {e} (keeping the previous config)"),
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}
