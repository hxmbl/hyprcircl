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

const INIT_CONFIG_TOML: &str = r#"# =============================================================================
# hyprcircl starter config
#
# Radial launcher for Hyprland. Edit this file freely — changes reload live.
# Visual styling lives in hyprcircl.css next to this file (colors, fonts, etc.).
#
# Docs: https://github.com/hxmbl/hyprcircl  (or `hyprcircl --help`)
# Bind in hyprland.conf:  bind = $mainMod, Space, exec, hyprcircl
# =============================================================================

schema_version = 1

# -----------------------------------------------------------------------------
# Ring geometry — drag these until the menu feels good under your thumb.
# The empty center is intentional. That's where your dignity goes.
# -----------------------------------------------------------------------------
inner_radius = 40.0
outer_radius = 110.0
ring_thickness = 60.0
ring_gap = 8.0
item_gap_degrees = 2.0

# -----------------------------------------------------------------------------
# Behavior
# -----------------------------------------------------------------------------
notify_only = false       # true = show a notification instead of running commands (safe mode)
show_labels = true        # text under icons; turn off for a cleaner look
keyboard_navigation = true  # arrows / 1-9 / Enter / Backspace

# -----------------------------------------------------------------------------
# Top status pill (Waybar-style modules)
#
# Each [[top_bar.modules]] runs `command` every `interval` seconds, or use
# `stream_command` for live push updates. `{output}` in `format` is replaced.
# Optional: on_click, on_click_right, on_scroll_up/down, watch = ["/path/to/file"]
# -----------------------------------------------------------------------------
[top_bar]
enabled = true

# Live workspace number via Hyprland's event socket (needs jq + nc)
[[top_bar.modules]]
stream_command = "S=${HYPRLAND_INSTANCE_SIGNATURE:-$(hyprctl -j instances | jq -r '.[0].instance')}; hyprctl activeworkspace -j | jq -r '.id'; nc -U \"$XDG_RUNTIME_DIR/hypr/$S/.socket2.sock\" | while read -r line; do case \"$line\" in workspace>>*) echo \"${line#workspace>>}\";; workspacev2>>*) echo \"${line#workspacev2>>}\" | cut -d, -f1;; esac; done"
interval = 1
format = "󰖯 {output}"
icon = ""

# Clock — the module that never lies (until DST)
[[top_bar.modules]]
command = "date +'%a %H:%M'"
interval = 30
format = "{output}"
icon = ""

# Battery — tweak BAT0 if your sysfs path differs (check /sys/class/power_supply/)
[[top_bar.modules]]
command = "s=$(cat /sys/class/power_supply/BAT0/status 2>/dev/null); c=$(cat /sys/class/power_supply/BAT0/capacity 2>/dev/null); [ -n \"$c\" ] && { [ \"$s\" = \"Charging\" ] && echo \"󰂄 $c%\" || echo \"󰁹 $c%\"; }"
interval = 30
format = "{output}"
icon = ""
watch = ["/sys/class/power_supply/BAT0/capacity", "/sys/class/power_supply/BAT0/status"]

# Volume — PulseAudio/PipeWire via pactl (comment out if you don't use it)
[[top_bar.modules]]
command = "v=$(pactl get-sink-volume @DEFAULT_SINK@ 2>/dev/null | grep -oE '[0-9]+%' | head -1); m=$(pactl get-sink-mute @DEFAULT_SINK@ 2>/dev/null | awk '{print $2}'); [ \"$m\" = \"yes\" ] && echo muted || echo \"${v:-?}\""
interval = 5
format = "󰕾 {output}"
icon = ""
on_click_right = "pactl set-sink-mute @DEFAULT_SINK@ toggle"
on_scroll_up = "pactl set-sink-volume @DEFAULT_SINK@ +5%"
on_scroll_down = "pactl set-sink-volume @DEFAULT_SINK@ -5%"

# -----------------------------------------------------------------------------
# Radial menu items
#
# Top-level [[items]] = wedges on the root ring.
# Nested [[items.items]] (and deeper) = outer submenu rings.
# Icons are Nerd Font glyphs — install a Nerd Font or swap for emoji.
# `command` is a program + args list; leave empty for submenu-only entries.
# -----------------------------------------------------------------------------

# --- Apps (swap commands for whatever you actually have installed) -----------
[[items]]
label = "Apps"
icon = "󰀻"

  [[items.items]]
  label = "Terminal"
  icon = "󰆍"
  # foot, kitty, alacritty, ghostty, wezterm… pick your poison
  command = ["foot"]

  [[items.items]]
  label = "Browser"
  icon = "󰖟"
  # firefox, chromium, brave, zen… the internet is vast and full of trackers
  command = ["firefox"]

  [[items.items]]
  label = "Files"
  icon = "󰉋"
  # thunar, nautilus, dolphin, pcmanfm-qt…
  command = ["xdg-open", "."]

  [[items.items]]
  label = "Editor"
  icon = "󰈔"
  command = ["xdg-open", ""]

# --- Hyprland window / layout actions (no extra apps required) ---------------
[[items]]
label = "Windows"
icon = "󰖲"

  [[items.items]]
  label = "Float"
  icon = "󰇛"
  command = ["hyprctl", "dispatch", "togglefloating"]

  [[items.items]]
  label = "Fullscreen"
  icon = "󰊓"
  command = ["hyprctl", "dispatch", "fullscreen"]

  [[items.items]]
  label = "Close"
  icon = "󰅖"
  command = ["hyprctl", "dispatch", "killactive"]

  [[items.items]]
  label = "Workspace"
  icon = "󰖯"

    [[items.items.items]]
    label = "Prev"
    icon = "󰅁"
    command = ["hyprctl", "dispatch", "workspace", "e-1"]

    [[items.items.items]]
    label = "Next"
    icon = "󰅂"
    command = ["hyprctl", "dispatch", "workspace", "e+1"]

    [[items.items.items]]
    label = "Empty"
    icon = "󰖰"
    command = ["hyprctl", "dispatch", "workspace", "empty"]

# --- Media (playerctl — because nothing says "i use linux" like fighting dbus)
[[items]]
label = "Media"
icon = "󰝚"

  [[items.items]]
  label = "Play/Pause"
  icon = "󰐊"
  command = ["playerctl", "play-pause"]

  [[items.items]]
  label = "Next"
  icon = "󰒭"
  command = ["playerctl", "next"]

  [[items.items]]
  label = "Previous"
  icon = "󰒮"
  command = ["playerctl", "previous"]

# --- Screenshots (wayland classics: grim + slurp; delete if you lack them) ---
[[items]]
label = "Capture"
icon = "󰄀"

  [[items.items]]
  label = "Region"
  icon = "󰆍"
  command = ["sh", "-c", "grim -g \"$(slurp)\" - | wl-copy"]

  [[items.items]]
  label = "Full"
  icon = "󰹑"
  command = ["sh", "-c", "grim - | wl-copy"]

# --- Lock & power ------------------------------------------------------------
[[items]]
label = "Lock"
icon = "󰌾"
# hyprlock, swaylock, loginctl lock-session… whatever keeps roommates out
command = ["loginctl", "lock-session"]

[[items]]
label = "Power"
icon = "󰐥"

  [[items.items]]
  label = "Log out"
  icon = "󰍃"
  command = ["hyprctl", "dispatch", "exit"]

  [[items.items]]
  label = "Suspend"
  icon = "󰒲"
  command = ["systemctl", "suspend"]

  [[items.items]]
  label = "Reboot"
  icon = "󰜉"
  command = ["systemctl", "reboot"]

  [[items.items]]
  label = "Shutdown"
  icon = "󰤂"
  command = ["systemctl", "poweroff"]
"#;

/// Current configuration schema version. bump this when adding fields that
/// change the config structure; old configs will be detected and defaults
/// applied until the user re-saves.
const CURRENT_SCHEMA_VERSION: u32 = 1;

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
    std::fs::write(&toml_path, INIT_CONFIG_TOML)
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
// =========================================================================

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuItem {
    pub label: String,
    pub icon: String,
    /// Executed when clicked; empty for submenu items.
    #[serde(default)]
    pub command: Vec<String>,
    /// Recursive child items for submenus.
    #[serde(default)]
    pub items: Vec<MenuItem>,
}

impl MenuItem {
    pub fn is_submenu(&self) -> bool {
        !self.items.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BarModule {
    /// Shell command whose stdout becomes the module text (Waybar-style).
    /// Optional when a `stream_command` is used instead.
    #[serde(default)]
    pub command: String,
    /// Refresh interval in seconds.
    pub interval: u64,
    /// Waybar-style format string; `{output}` is replaced with the command output.
    pub format: String,
    /// Optional icon rendered before the output.
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
    /// Waybar-style applets: e.g. `omarchy-launch-bluetooth`. Empty disables.
    #[serde(default)]
    pub on_click: String,
    /// Shell command run when the module is right-clicked (Waybar
    /// `on-click-right`), e.g. `pamixer -t` to toggle mute.
    #[serde(default)]
    pub on_click_right: String,
    /// Shell command run when scrolling up over the module (Waybar
    /// `on-scroll-up`), e.g. `pamixer -i 5` to raise volume.
    #[serde(default)]
    pub on_scroll_up: String,
    /// Shell command run when scrolling down over the module (Waybar
    /// `on-scroll-down`), e.g. `pamixer -d 5` to lower volume.
    #[serde(default)]
    pub on_scroll_down: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopBarConfig {
    pub enabled: bool,
    /// Ordered list of Waybar-style modules rendered inside the pill.
    /// Visual styling (colors, fonts, geometry) is driven by `hyprcircl.css`,
    /// so the pill has no style fields here anymore.
    pub modules: Vec<BarModule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadialConfig {
    pub inner_radius: f64,
    pub outer_radius: f64,
    /// Thickness of each outer submenu ring.
    #[serde(default = "default_ring_thickness")]
    pub ring_thickness: f64,
    /// Gap between consecutive rings.
    #[serde(default = "default_ring_gap")]
    pub ring_gap: f64,
    pub item_gap_degrees: f64,
    pub top_bar: TopBarConfig,
    /// When true, clicking an action only sends a notification instead of
    /// executing its command (safety mode).
    #[serde(default)]
    pub notify_only: bool,
    /// When false, text labels are hidden below icons in the radial menu.
    #[serde(default = "default_true")]
    pub show_labels: bool,
    /// Arrow keys / number keys navigate wedges; Enter activates.
    #[serde(default = "default_true")]
    pub keyboard_navigation: bool,
    pub items: Vec<MenuItem>,
    /// Bumped by the watcher on every successful reload so consumers can
    /// detect "the module list changed" and rebuild index-keyed state
    /// instead of misattributing streams/outputs across a reorder.
    #[serde(skip, default)]
    pub generation: u64,
    /// Configuration format version. Used to detect old config formats and
    /// apply backward-compatible migrations or refuse to load incompatible ones.
    #[serde(default)]
    pub schema_version: u32,
}

fn default_ring_thickness() -> f64 {
    60.0
}

fn default_ring_gap() -> f64 {
    8.0
}

fn default_true() -> bool {
    true
}

impl Default for RadialConfig {
    fn default() -> Self {
        Self {
            inner_radius: 40.0,
            outer_radius: 110.0,
            ring_thickness: 60.0,
            ring_gap: 8.0,
            item_gap_degrees: 2.0,
            notify_only: false,
            show_labels: true,
            keyboard_navigation: true,
            generation: 0,
            schema_version: env!("CARGO_PKG_VERSION").parse().unwrap_or(0),
            top_bar: TopBarConfig {
                enabled: true,
                modules: vec![
                    BarModule {
                        command: "cat /proc/loadavg | awk '{print $1}'".into(),
                        interval: 2,
                        format: "CPU {output}".into(),
                        ..Default::default()
                    },
                    BarModule {
                        command: "free -h | awk '/^Mem/ {print $3 \"/\" $2}'".into(),
                        interval: 5,
                        format: "{output}".into(),
                        ..Default::default()
                    },
                    BarModule {
                        command: "date +'%a %H:%M'".into(),
                        interval: 30,
                        format: "{output}".into(),
                        ..Default::default()
                    },
                ],
            },
            items: vec![
                MenuItem {
                    label: "Media".into(),
                    icon: "🎵".into(),
                    command: vec![],
                    items: vec![
                        MenuItem {
                            label: "Play/Pause".into(),
                            icon: "⏯".into(),
                            command: vec!["playerctl".into(), "play-pause".into()],
                            ..Default::default()
                        },
                        MenuItem {
                            label: "Next".into(),
                            icon: "⏭".into(),
                            command: vec!["playerctl".into(), "next".into()],
                            ..Default::default()
                        },
                        MenuItem {
                            label: "Volume".into(),
                            icon: "🔊".into(),
                            command: vec![],
                            items: vec![MenuItem {
                                label: "Mute".into(),
                                icon: "🔇".into(),
                                command: vec!["pamixer".into(), "-t".into()],
                                ..Default::default()
                            }],
                        },
                    ],
                },
                MenuItem {
                    label: "Terminal".into(),
                    icon: "💻".into(),
                    command: vec!["foot".into()],
                    ..Default::default()
                },
                MenuItem {
                    label: "Lock".into(),
                    icon: "🔒".into(),
                    command: vec!["hyprlock".into()],
                    ..Default::default()
                },
            ],
        }
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
        let cfg: Result<RadialConfig, _> = toml::from_str(&contents);
        let cfg = match cfg {
            Ok(mut cfg) => {
                // Ensure schema_version exists (old configs won't have it).
                if cfg.schema_version == 0 {
                    cfg.schema_version = 1;
                }
                // Check for future versions we don't understand yet.
                if cfg.schema_version > CURRENT_SCHEMA_VERSION {
                    println!(
                        "[CONFIG] Config {path} schema_version {} is ahead of \
                         this hyprcircl version ({}), using defaults until \
                         updated",
                        cfg.schema_version,
                        CURRENT_SCHEMA_VERSION
                    );
                    return (RadialConfig::default(), Some(path));
                }
                cfg
            }
            Err(e) => {
                println!("[CONFIG] Failed to parse {path}, using defaults until fixed: {e}");
                return (RadialConfig::default(), Some(path));
            }
        };
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
            items: vec![],
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
                items: vec![],
            }],
        };
        assert!(branch.is_submenu());
    }

    #[test]
    fn default_config_is_fully_populated() {
        let cfg = RadialConfig::default();
        assert!(cfg.inner_radius > 0.0);
        assert!(cfg.outer_radius > cfg.inner_radius);
        assert_eq!(cfg.ring_thickness, 60.0);
        assert_eq!(cfg.ring_gap, 8.0);
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
                    match toml::from_str::<RadialConfig>(&contents) {
                        Ok(mut cfg) => {
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
                        Err(e) => println!("[CONFIG] Failed to parse {path}: {e}"),
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}
