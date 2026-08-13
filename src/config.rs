use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

// =========================================================================
// Data Configuration Model
// =========================================================================

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopBarConfig {
    pub enabled: bool,
    pub height: f64,
    pub padding_x: f64,
    pub offset_y: f64,
    pub corner_radius: f64,
    /// RGBA [r, g, b, a] pill background color.
    pub background: [f64; 4],
    /// RGBA [r, g, b, a] module text color.
    pub foreground: [f64; 4],
    /// Pango/Cairo font family for module text (use a Nerd Font so icons render).
    #[serde(default = "default_font")]
    pub font: String,
    /// Ordered list of Waybar-style modules rendered inside the pill.
    pub modules: Vec<BarModule>,
}

pub fn default_font() -> String {
    "JetBrainsMono Nerd Font".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub corner_radius: f64,
    pub top_bar: TopBarConfig,
    /// When true, clicking an action only sends a notification instead of
    /// executing its command (safety mode).
    #[serde(default)]
    pub notify_only: bool,
    /// When false, text labels are hidden below icons in the radial menu.
    #[serde(default = "default_true")]
    pub show_labels: bool,
    pub items: Vec<MenuItem>,
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
            corner_radius: 6.0,
            notify_only: false,
            show_labels: true,
            top_bar: TopBarConfig {
                enabled: true,
                height: 28.0,
                padding_x: 14.0,
                offset_y: 20.0,
                corner_radius: 14.0,
                background: [0.15, 0.15, 0.2, 0.9],
                foreground: [0.9, 0.9, 0.95, 1.0],
                font: default_font(),
                modules: vec![
                    BarModule { command: "cat /proc/loadavg | awk '{print $1}'".into(), interval: 2, format: "CPU {output}".into(), ..Default::default() },
                    BarModule { command: "free -h | awk '/^Mem/ {print $3 \"/\" $2}'".into(), interval: 5, format: "{output}".into(), ..Default::default() },
                    BarModule { command: "date +'%a %H:%M'".into(), interval: 30, format: "{output}".into(), ..Default::default() },
                ],
            },
            items: vec![
                MenuItem {
                    label: "Media".into(),
                    icon: "🎵".into(),
                    command: vec![],
                    items: vec![
                        MenuItem { label: "Play/Pause".into(), icon: "⏯".into(), command: vec!["playerctl".into(), "play-pause".into()], ..Default::default() },
                        MenuItem { label: "Next".into(), icon: "⏭".into(), command: vec!["playerctl".into(), "next".into()], ..Default::default() },
                        MenuItem {
                            label: "Volume".into(),
                            icon: "🔊".into(),
                            command: vec![],
                            items: vec![
                                MenuItem { label: "Mute".into(), icon: "🔇".into(), command: vec!["pamixer".into(), "-t".into()], ..Default::default() },
                            ],
                        },
                    ],
                },
                MenuItem { label: "Terminal".into(), icon: "💻".into(), command: vec!["foot".into()], ..Default::default() },
                MenuItem { label: "Lock".into(), icon: "🔒".into(), command: vec!["hyprlock".into()], ..Default::default() },
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

/// Load `RadialConfig` from TOML, falling back to defaults when no file is found.
pub fn load_config() -> RadialConfig {
    for path in config_paths() {
        if let Ok(contents) = std::fs::read_to_string(&path) {
            match toml::from_str::<RadialConfig>(&contents) {
                Ok(cfg) => {
                    println!("[CONFIG] Loaded {path}");
                    return cfg;
                }
                Err(e) => println!("[CONFIG] Failed to parse {path}: {e}"),
            }
        }
    }

    println!("[CONFIG] No config found, using defaults");
    RadialConfig::default()
}

/// First existing config file, if any (used to watch for changes).
pub fn find_config_path() -> Option<String> {
    config_paths()
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

/// Find a CSS file in the same directories as the config.
pub fn find_css_path() -> Option<String> {
    let dirs: Vec<String> = config_paths()
        .into_iter()
        .filter_map(|p| std::path::Path::new(&p).parent().map(|d| d.to_string_lossy().to_string()))
        .collect();
    for dir in dirs {
        let css = format!("{}/hyprcircl.css", dir);
        if std::path::Path::new(&css).exists() {
            return Some(css);
        }
    }
    None
}

/// Background thread: polls the config file and swaps it into the shared
/// `RwLock` whenever its contents change, so edits apply live.
pub fn watch_config(path: String, config: Arc<RwLock<RadialConfig>>) {
    std::thread::spawn(move || {
        let mut last_contents: Option<String> = None;
        loop {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if last_contents.as_deref() != Some(&contents) {
                    last_contents = Some(contents.clone());
                    match toml::from_str::<RadialConfig>(&contents) {
                        Ok(cfg) => {
                            if let Ok(mut c) = config.write() {
                                *c = cfg;
                            }
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
