use std::cell::Cell;
use std::f64::consts::PI;
use std::io::{BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use gdk4::Key;
use gtk4::cairo::{self};
use gtk4::gdk;
use gtk4::glib::{ControlFlow, Propagation, timeout_add_local};
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, CssProvider, DrawingArea, EventControllerKey,
    EventControllerMotion, EventControllerScroll, EventControllerScrollFlags, GestureClick,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::{Deserialize, Serialize};

// =========================================================================
// 1. Data Configuration Model
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

fn default_font() -> String {
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
    pub items: Vec<MenuItem>,
}

fn default_ring_thickness() -> f64 {
    60.0
}

fn default_ring_gap() -> f64 {
    8.0
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
// 2. Cairo Geometry Helpers
// =========================================================================

/// ===== RADIAL MENU =====
/// Draws one annular (donut-wedge) sector with rounded corners on all 4 corners.
fn draw_rounded_sector(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    r_in: f64,
    r_out: f64,
    a1: f64,
    a2: f64,
    r_c: f64,
) {
    let span = a2 - a1;
    if span <= 0.0 {
        return;
    }

    // Clamp corner radius so it cannot exceed available geometry
    let max_r = ((r_out - r_in) / 2.0).min(r_out * 0.45);
    let r = r_c.clamp(0.0, max_r);

    if r <= 0.1 {
        // Fallback for near-zero rounding: draw standard sharp sector
        cr.new_path();
        cr.arc(cx, cy, r_out, a1, a2);
        cr.arc_negative(cx, cy, r_in, a2, a1);
        cr.close_path();
        return;
    }

    // Exact trigonometric angular offsets for corner arc centers
    let sin_out = (r / (r_out - r)).clamp(-1.0, 1.0);
    let delta_out = sin_out.asin();

    let sin_in = (r / (r_in + r)).clamp(-1.0, 1.0);
    let delta_in = sin_in.asin();

    // Guard against corner overlap when wedge angle is small
    if delta_out * 2.0 >= span || delta_in * 2.0 >= span {
        cr.new_path();
        cr.arc(cx, cy, r_out, a1, a2);
        cr.arc_negative(cx, cy, r_in, a2, a1);
        cr.close_path();
        return;
    }

    // Center angles for the 4 corner circles
    let theta_os = a1 + delta_out; // Outer Start
    let theta_oe = a2 - delta_out; // Outer End
    let theta_ie = a2 - delta_in;  // Inner End
    let theta_is = a1 + delta_in;  // Inner Start

    // Calculated center coordinates for each corner arc
    let c_os = (cx + (r_out - r) * theta_os.cos(), cy + (r_out - r) * theta_os.sin());
    let c_oe = (cx + (r_out - r) * theta_oe.cos(), cy + (r_out - r) * theta_oe.sin());
    let c_ie = (cx + (r_in + r) * theta_ie.cos(), cy + (r_in + r) * theta_ie.sin());
    let c_is = (cx + (r_in + r) * theta_is.cos(), cy + (r_in + r) * theta_is.sin());

    cr.new_path();

    // 1. Outer Arc (sweeps between outer corner contact points)
    cr.arc(cx, cy, r_out, theta_os, theta_oe);

    // 2. Top-Right / Outer-End Corner Arc
    cr.arc(c_oe.0, c_oe.1, r, theta_oe, a2 + PI / 2.0);

    // 3. Bottom-Right / Inner-End Corner Arc
    cr.arc(c_ie.0, c_ie.1, r, a2 + PI / 2.0, theta_ie + PI);

    // 4. Inner Arc (reversed / counter-clockwise)
    cr.arc_negative(cx, cy, r_in, theta_ie, theta_is);

    // 5. Bottom-Left / Inner-Start Corner Arc
    cr.arc(c_is.0, c_is.1, r, theta_is + PI, a1 - PI / 2.0);

    // 6. Top-Left / Outer-Start Corner Arc
    cr.arc(c_os.0, c_os.1, r, a1 - PI / 2.0, theta_os);

    cr.close_path();
}

/// ===== TOP PILL =====
/// The rounded capsule/bar floating above the menu (the "top pill").
/// Waybar-style: it auto-sizes to fit its modules and renders each module's
/// `format` with `{output}` replaced by the current command output.
///
/// Layout is computed once (in `top_bar_layout`, off-screen, so the geometry
/// is identical for drawing and for pointer hit-testing) and rendered here.
#[derive(Clone, Debug)]
struct BarLayout {
    /// Visible module text (empty modules are skipped).
    texts: Vec<String>,
    /// Index into `cfg.modules` for each visible text.
    indices: Vec<usize>,
    /// Canvas x of each module's text left edge.
    lefts: Vec<f64>,
    /// Canvas width of each module's text.
    widths: Vec<f64>,
    /// Pill rectangle in canvas coords.
    rx: f64,
    ry: f64,
    width: f64,
    height: f64,
}

/// Measure a module string's rendered width for the pill font/size.
fn measure_pill_text(font: &str, text: &str) -> f64 {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    cr.select_font_face(font, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    cr.text_extents(text).unwrap().width()
}

/// Compute pill geometry from the current module outputs. `None` when every
/// module is empty (the pill then isn't drawn at all).
fn top_bar_layout(
    cfg: &TopBarConfig,
    r_out: f64,
    outputs: &[String],
    cx: f64,
    cy: f64,
) -> Option<BarLayout> {
    let height = cfg.height;
    let gap = 18.0; // spacing between modules

    let mut texts = Vec::new();
    let mut indices = Vec::new();
    for (i, m) in cfg.modules.iter().enumerate() {
        let out = outputs.get(i).cloned().unwrap_or_default();
        let body = m.format.replace("{output}", &out);
        let text = if m.icon.is_empty() {
            body
        } else {
            format!("{} {}", m.icon, body)
        };
        if !text.trim().is_empty() {
            texts.push(text);
            indices.push(i);
        }
    }
    if texts.is_empty() {
        return None;
    }

    let mut widths = Vec::new();
    let mut total_w = 0.0;
    for (j, t) in texts.iter().enumerate() {
        let w = measure_pill_text(&cfg.font, t);
        widths.push(w);
        total_w += w;
        if j + 1 < texts.len() {
            total_w += gap;
        }
    }
    let width = total_w + cfg.padding_x * 2.0;
    let rx = cx - width / 2.0;
    let ry = cy - r_out - cfg.offset_y - height;

    let mut lefts = Vec::new();
    let mut x = cx - total_w / 2.0;
    for w in &widths {
        lefts.push(x);
        x += w + gap;
    }

    Some(BarLayout {
        texts,
        indices,
        lefts,
        widths,
        rx,
        ry,
        width,
        height,
    })
}

/// Return the module index (into `TopBarConfig::modules`) whose text contains
/// the pointer position, if that position is inside the pill.
fn hit_test_pill(layout: &BarLayout, mx: f64, my: f64) -> Option<usize> {
    if mx < layout.rx
        || mx > layout.rx + layout.width
        || my < layout.ry
        || my > layout.ry + layout.height
    {
        return None;
    }
    for (k, left) in layout.lefts.iter().enumerate() {
        if mx >= *left && mx < *left + layout.widths[k] {
            return Some(layout.indices[k]);
        }
    }
    None
}

fn draw_top_bar(cr: &cairo::Context, layout: &BarLayout, cfg: &TopBarConfig) {
    let rx = layout.rx;
    let ry = layout.ry;
    let width = layout.width;
    let height = layout.height;
    let r = cfg.corner_radius.min(height / 2.0);

    // Background pill
    cr.new_sub_path();
    cr.arc(rx + width - r, ry + r, r, -PI / 2.0, 0.0);
    cr.arc(rx + width - r, ry + height - r, r, 0.0, PI / 2.0);
    cr.arc(rx + r, ry + height - r, r, PI / 2.0, PI);
    cr.arc(rx + r, ry + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();

    let [bg_r, bg_g, bg_b, bg_a] = cfg.background;
    cr.set_source_rgba(bg_r, bg_g, bg_b, bg_a);
    let _ = cr.fill();

    // Module text, centered vertically and laid out left-to-right.
    cr.select_font_face(&cfg.font, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    let [fg_r, fg_g, fg_b, fg_a] = cfg.foreground;
    cr.set_source_rgba(fg_r, fg_g, fg_b, fg_a);
    let text_y = ry + height / 2.0;
    for (k, t) in layout.texts.iter().enumerate() {
        let ext = cr.text_extents(t).unwrap();
        cr.move_to(
            layout.lefts[k] - ext.x_bearing(),
            text_y - ext.height() / 2.0 - ext.y_bearing(),
        );
        let _ = cr.show_text(t);
    }
}

// =========================================================================
// 3. Process Execution & Cursor System
// =========================================================================

fn run(cmd: &[&str]) -> Option<String> {
    let out = Command::new(cmd[0]).args(&cmd[1..]).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a shell command and return its stdout (Waybar module commands).
fn run_shell(cmd: &str) -> Option<String> {
    let out = Command::new("sh").args(["-c", cmd]).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

/// Terminate a stream child and its whole process group. Stream commands are
/// spawned under `setsid`, so the child PID is its process-group leader;
/// killing the negative PID reaps grandchildren too (e.g. `nc`, `pactl`).
/// The `--` is required so `-<pid>` is parsed as a group PID, not a signal.
fn kill_process_group(child: &mut std::process::Child) {
    let pid = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-9", "--", &pid]).status();
    let _ = child.wait();
}

/// Path of the Unix socket used to signal a running instance to toggle.
fn socket_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(dir).join("radial-menu.sock")
}

/// Toggle: if another instance is already running, tell it to show/hide the
/// menu over its socket and report `true` so this process exits. Otherwise
/// report `false` so this process becomes the persistent daemon.
fn signal_toggle() -> bool {
    let path = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(b"toggle");
        return true;
    }
    // No live daemon: clear any stale socket left behind by a crash.
    let _ = std::fs::remove_file(&path);
    false
}

fn cursor_local_pos() -> Option<(f64, f64)> {
    if which("hyprctl") {
        if let Some(res) = run(&["hyprctl", "-j", "cursorpos"]) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&res) {
                if let (Some(x), Some(y)) = (v.get("x").and_then(|n| n.as_f64()), v.get("y").and_then(|n| n.as_f64())) {
                    if let Some(m_res) = run(&["hyprctl", "-j", "monitors"]) {
                        if let Ok(mv) = serde_json::from_str::<serde_json::Value>(&m_res) {
                            if let Some(mons) = mv.as_array() {
                                for mon in mons {
                                    let mx = mon.get("x").and_then(|n| n.as_f64())?;
                                    let my = mon.get("y").and_then(|n| n.as_f64())?;
                                    let mw = mon.get("width").and_then(|n| n.as_f64())?;
                                    let mh = mon.get("height").and_then(|n| n.as_f64())?;
                                    if mx <= x && x < mx + mw && my <= y && y < my + mh {
                                        return Some((x - mx, y - my));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

// =========================================================================
// 4. Configuration IO (load + auto-reload from TOML)
// =========================================================================

/// Candidate config file locations, in priority order.
/// The real file is `radial_menu.toml`; `config.toml` is kept as a compat
/// fallback (e.g. a symlink) since it's too generic a name for a config folder.
fn config_paths() -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            paths.push(format!("{xdg}/radial-menu/radial_menu.toml"));
            paths.push(format!("{xdg}/radial-menu/config.toml"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            paths.push(format!("{home}/.config/radial-menu/radial_menu.toml"));
            paths.push(format!("{home}/.config/radial-menu/config.toml"));
        }
    }
    paths.push("radial_menu.toml".into());
    paths.push("config.toml".into());
    paths
}

/// Load `RadialConfig` from TOML, falling back to defaults when no file is found.
fn load_config() -> RadialConfig {
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
fn find_config_path() -> Option<String> {
    config_paths()
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

/// Background thread: polls the config file and swaps it into the shared
/// `RwLock` whenever its contents change, so edits apply live.
fn watch_config(path: String, config: Arc<RwLock<RadialConfig>>) {
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

// =========================================================================
// 5. Angular Math & Navigation Stack
// =========================================================================

/// State per active stack level.
/// The menu center is shared/stationary; deeper levels fan outward as
/// partial arcs anchored on the parent wedge's mid-angle.
#[derive(Clone, Debug)]
pub struct LevelSelection {
    pub selected_child_index: Option<usize>,
    pub parent_mid_angle: f64,
}

/// Angular slice (start_angle, end_angle) for an item at a hierarchy level.
/// Level 0 spans the full 360° circle; deeper levels fan outward around the
/// parent wedge's mid-angle.
fn get_item_angles(
    level: usize,
    index: usize,
    count: usize,
    parent_mid_angle: f64,
    gap_rad: f64,
) -> (f64, f64) {
    if level == 0 {
        let step = 2.0 * PI / count as f64;
        let a1 = (index as f64 * step) - (PI / 2.0) + (gap_rad / 2.0);
        let a2 = ((index + 1) as f64 * step) - (PI / 2.0) - (gap_rad / 2.0);
        (a1, a2)
    } else {
        // Submenus: outward fan centered on the parent wedge's mid angle.
        let arc_span = (PI * 0.75).min((count as f64 * 35.0).to_radians());
        let start = parent_mid_angle - (arc_span / 2.0);
        let step = arc_span / count as f64;
        let a1 = start + (index as f64 * step) + (gap_rad / 2.0);
        let a2 = start + ((index + 1) as f64 * step) - (gap_rad / 2.0);
        (a1, a2)
    }
}

/// Wrap-around-aware angular containment test.
fn angle_in_slice(angle: f64, a1: f64, a2: f64) -> bool {
    let two_pi = 2.0 * PI;
    let a1n = a1.rem_euclid(two_pi);
    let a2r = (a2 - a1n).rem_euclid(two_pi);
    let angler = (angle - a1n).rem_euclid(two_pi);
    angler <= a2r
}

/// Find the item whose angular slice contains `angle`.
fn hit_test_index(
    level: usize,
    angle: f64,
    count: usize,
    parent_mid_angle: f64,
    gap_rad: f64,
) -> Option<usize> {
    for i in 0..count {
        let (a1, a2) = get_item_angles(level, i, count, parent_mid_angle, gap_rad);
        if angle_in_slice(angle, a1, a2) {
            return Some(i);
        }
    }
    None
}

// =========================================================================
// 6. Main Application Window & Controller Logic
// =========================================================================

fn main() {
    if signal_toggle() {
        std::process::exit(0);
    }

    let app = Application::builder()
        .application_id("com.omarchy.radial")
        .build();

    app.connect_activate(build_window);
    app.run();
}

fn build_window(app: &Application) {
    let window = ApplicationWindow::builder().application(app).build();
    let config: Arc<RwLock<RadialConfig>> = Arc::new(RwLock::new(load_config()));

    // Reload the config automatically when the file changes on disk.
    if let Some(path) = find_config_path() {
        watch_config(path, config.clone());
    }

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("radial-menu"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }

    let provider = CssProvider::new();
    provider.load_from_data("window { background-color: transparent; }");
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Stationary menu center, anchored at the cursor (falls back to screen center).
    let (root_x, root_y) = cursor_local_pos().unwrap_or((0.0, 0.0));
    let center_pos = Arc::new(RwLock::new((root_x, root_y)));

    // Whether the menu is currently visible (toggled by the daemon socket).
    let shown = Arc::new(AtomicBool::new(true));

    let nav_stack = Arc::new(RwLock::new(vec![LevelSelection {
        selected_child_index: None,
        parent_mid_angle: 0.0,
    }]));
    let hover_index = Arc::new(RwLock::new(None::<usize>));

    let canvas = DrawingArea::new();

    // ===== TOP PILL MODULES (Waybar-style) =====
    // Shared state: one string of current output per module.
    let module_count = config.read().map(|c| c.top_bar.modules.len()).unwrap_or(0);
    let bar_state: Arc<Mutex<Vec<String>>> =
        Arc::new(Mutex::new(vec![String::new(); module_count]));

    // Redraw on an interval so the worker thread's new module output and any
    // config reload show up on screen.
    let cv = canvas.clone();
    timeout_add_local(Duration::from_millis(200), move || {
        cv.queue_draw();
        ControlFlow::Continue
    });

    // ===== PILL MODULE REFRESH =====
    // Two cooperating threads keep the pill fresh without burning CPU:
    //  - WATCH thread: every ~100ms reads each module's `watch` files (tiny
    //    sysfs reads, ~µs each) and sets a refresh flag when any change.
    //  - RUN thread: executes a module's command only when its interval has
    //    elapsed OR its watch flag is set. File-backed modules (brightness,
    //    battery, link state) therefore update instantly at near-zero cost,
    //    while command-only modules keep their simple interval polling.
    // Both re-read the shared config every loop, so config reloads apply live.
    let refresh_flags: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));

    // Clean shutdown on SIGTERM/SIGINT: kill stream children and exit.
    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let sd = shutdown.clone();
        let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, sd);
        let sd = shutdown.clone();
        let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, sd);
    }

    // WATCH thread: cheap change detection on watched files.
    {
        let config_w = config.clone();
        let flags = refresh_flags.clone();
        let shown_w = shown.clone();
        std::thread::spawn(move || {
            let mut caches: Vec<Vec<Option<Vec<u8>>>> = Vec::new();
            loop {
                if !shown_w.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                let modules = config_w
                    .read()
                    .map(|c| c.top_bar.modules.clone())
                    .unwrap_or_default();
                let n = modules.len();
                if caches.len() != n {
                    caches.resize(n, Vec::new());
                }

                let mut dirty = vec![false; n];
                for (i, m) in modules.iter().enumerate() {
                    if m.watch.is_empty() {
                        continue;
                    }
                    if caches[i].len() != m.watch.len() {
                        caches[i] = vec![None; m.watch.len()];
                    }
                    for (j, path) in m.watch.iter().enumerate() {
                        let cur = std::fs::read(path).ok();
                        if caches[i][j] != cur {
                            caches[i][j] = cur;
                            dirty[i] = true;
                        }
                    }
                }

                if dirty.iter().any(|d| *d) {
                    if let Ok(mut f) = flags.lock() {
                        if f.len() < n {
                            f.resize(n, false);
                        }
                        for (i, d) in dirty.iter().enumerate() {
                            if *d {
                                f[i] = true;
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }

    // RUN thread: interval + watch-triggered command execution. Modules with a
    // `stream_command` instead run that long-lived process once per show and
    // update from its stdout lines — pure push, no polling (e.g. the Hyprland
    // event socket or `pactl subscribe`). Interval commands run in their own
    // threads so a slow one (e.g. `top -bn2`) never stalls stream handling or
    // the hide-kill path.
    {
        let config_w = config.clone();
        let state = bar_state.clone();
        let flags = refresh_flags.clone();
        let shown_w = shown.clone();
        let shutdown_w = shutdown.clone();
        std::thread::spawn(move || {
            let mut last: Vec<Instant> = Vec::new();
            let mut streams: Vec<Option<std::process::Child>> = Vec::new();
            loop {
                // SIGTERM/SIGINT: reap stream children, then die with the daemon.
                if shutdown_w.load(Ordering::Relaxed) {
                    for s in streams.iter_mut() {
                        if let Some(mut c) = s.take() {
                            kill_process_group(&mut c);
                        }
                    }
                    std::process::exit(0);
                }
                let shown = shown_w.load(Ordering::Relaxed);
                let modules = config_w
                    .read()
                    .map(|c| c.top_bar.modules.clone())
                    .unwrap_or_default();
                let n = modules.len();

                if let Ok(mut st) = state.lock() {
                    if st.len() != n {
                        st.resize(n, String::new());
                    }
                }
                if last.len() < n {
                    let now = Instant::now();
                    let past = now.checked_sub(Duration::from_secs(3600)).unwrap_or(now);
                    last.resize(n, past);
                }
                if streams.len() != n {
                    while streams.len() > n {
                        if let Some(mut c) = streams.pop().flatten() {
                            kill_process_group(&mut c);
                        }
                    }
                    while streams.len() < n {
                        streams.push(None);
                    }
                }

                if !shown {
                    // Hidden: kill any live stream processes; they restart on reopen.
                    for s in streams.iter_mut() {
                        if let Some(mut c) = s.take() {
                            kill_process_group(&mut c);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }

                let now = Instant::now();

                // Snapshot and clear the watch-thread refresh flags.
                let mut refresh = vec![false; n];
                if let Ok(mut f) = flags.lock() {
                    if f.len() < n {
                        f.resize(n, false);
                    }
                    refresh.clone_from_slice(&f[..n]);
                    f.fill(false);
                }

                for (i, m) in modules.iter().enumerate() {
                    if !m.stream_command.is_empty() {
                        // PUSH MODE: manage the long-lived stream process.
                        if let Some(mut child) = streams[i].take() {
                            match child.try_wait() {
                                Ok(Some(_)) => { /* exited: respawn below */ }
                                _ => {
                                    streams[i] = Some(child);
                                    continue;
                                }
                            }
                        }
                        if let Ok(mut child) = Command::new("setsid")
                            .args(["sh", "-c", &m.stream_command])
                            .stdout(Stdio::piped())
                            .spawn()
                        {
                            if let Some(stdout) = child.stdout.take() {
                                let state_r = state.clone();
                                let idx = i;
                                std::thread::spawn(move || {
                                    let mut reader = std::io::BufReader::new(stdout);
                                    let mut line = String::new();
                                    loop {
                                        line.clear();
                                        match reader.read_line(&mut line) {
                                            Ok(0) | Err(_) => break,
                                            Ok(_) => {
                                                let out = line.trim().to_string();
                                                if let Ok(mut st) = state_r.lock() {
                                                    if idx < st.len() {
                                                        st[idx] = out;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                            streams[i] = Some(child);
                        }
                    } else {
                        // Kill a leftover stream if the config switched modes.
                        if let Some(mut c) = streams[i].take() {
                            kill_process_group(&mut c);
                        }
                        let due = now.duration_since(last[i]).as_secs() >= m.interval;
                        if due || refresh[i] {
                            last[i] = now;
                            if m.command.is_empty() {
                                continue;
                            }
                            let cmd = m.command.clone();
                            let state_r = state.clone();
                            let idx = i;
                            std::thread::spawn(move || {
                                if let Some(out) = run_shell(&cmd) {
                                    let out = out.trim().to_string();
                                    if let Ok(mut st) = state_r.lock() {
                                        if idx < st.len() {
                                            st[idx] = out;
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }

    let nav = nav_stack.clone();
    let h_idx = hover_index.clone();
    let config_draw = config.clone();
    let bar_state_draw = bar_state.clone();
    let center_draw = center_pos.clone();

    // Render Function
    canvas.set_draw_func(move |_, cr, width, height| {
        let cfg = match config_draw.read() {
            Ok(c) => c,
            Err(_) => return,
        };

        let (mut cx, mut cy) = *center_draw.read().unwrap();
        if cx == 0.0 && cy == 0.0 {
            cx = width as f64 / 2.0;
            cy = height as f64 / 2.0;
            *center_draw.write().unwrap() = (cx, cy);
        }

        let stack = nav.read().unwrap();
        if stack.is_empty() {
            return;
        }

        // ===== TOP PILL =====
        // Renders only at the root menu (nav stack depth == 1).
        if cfg.top_bar.enabled && stack.len() == 1 {
            let outputs = bar_state_draw.lock().map(|s| s.clone()).unwrap_or_default();
            if let Some(layout) = top_bar_layout(&cfg.top_bar, cfg.outer_radius, &outputs, cx, cy) {
                draw_top_bar(cr, &layout, &cfg.top_bar);
            }
        }

        // ===== RADIAL MENU RINGS =====
        // Render each open level as a concentric ring of wedges.
        let mut current_items = &cfg.items;

        for level in 0..stack.len() {
            let count = current_items.len();
            if count == 0 {
                break;
            }

            let (r_in, r_out) = cfg.get_level_radii(level);
            let selected_child = stack[level].selected_child_index;
            let parent_angle = stack[level].parent_mid_angle;
            let is_active_level = level == stack.len() - 1;
            let gap_rad = cfg.item_gap_degrees.to_radians();

            for (i, item) in current_items.iter().enumerate() {
                let (a1, a2) = get_item_angles(level, i, count, parent_angle, gap_rad);

                draw_rounded_sector(cr, cx, cy, r_in, r_out, a1, a2, cfg.corner_radius);

                // --- COLOR SELECTION ---
                if selected_child == Some(i) {
                    // Parent branch slice leading to an open submenu -> RED
                    cr.set_source_rgba(0.88, 0.29, 0.29, 0.95);
                } else if is_active_level && *h_idx.read().unwrap() == Some(i) {
                    // Current active menu hovered slice -> Accent Blue
                    cr.set_source_rgba(0.48, 0.63, 0.96, 0.95);
                } else {
                    // Default Idle Gray
                    cr.set_source_rgba(0.85, 0.85, 0.88, 0.90);
                }
                let _ = cr.fill_preserve();

                // Stroke
                cr.set_source_rgba(0.7, 0.7, 0.75, 1.0);
                cr.set_line_width(1.5);
                let _ = cr.stroke();

                // Icon/Label
                let mid_angle = (a1 + a2) / 2.0;
                let mid_r = (r_in + r_out) / 2.0;
                let tx = cx + mid_r * mid_angle.cos();
                let ty = cy + mid_r * mid_angle.sin();

                cr.set_source_rgba(0.1, 0.1, 0.15, 1.0);
                cr.select_font_face(&cfg.top_bar.font, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
                cr.set_font_size(16.0);

                let extents = cr.text_extents(&item.icon).unwrap();
                cr.move_to(
                    tx - (extents.width() / 2.0 + extents.x_bearing()),
                    ty - (extents.height() / 2.0 + extents.y_bearing()),
                );
                let _ = cr.show_text(&item.icon);
            }

            // Descend to the open child level for the next iteration.
            if let Some(child_idx) = selected_child {
                if child_idx < current_items.len() {
                    current_items = &current_items[child_idx].items;
                } else {
                    break;
                }
            }
        }
    });

    window.set_child(Some(&canvas));

    // --- Hover Motion Controller ---
    let motion = EventControllerMotion::new();
    let nav_m = nav_stack.clone();
    let cfg_m = config.clone();
    let h_m = hover_index.clone();
    let cv_m = canvas.clone();
    let center_m = center_pos.clone();
    // Last pointer position, shared with the scroll controller so it knows
    // which pill module (if any) is under the cursor.
    let last_pos = Rc::new(Cell::new((0.0f64, 0.0f64)));
    let last_pos_m = last_pos.clone();

    motion.connect_motion(move |_, mx, my| {
        last_pos_m.set((mx, my));
        let cfg = match cfg_m.read() {
            Ok(c) => c,
            Err(_) => return,
        };
        let stack = nav_m.read().unwrap();
        if stack.is_empty() {
            return;
        }
        let (cx, cy) = *center_m.read().unwrap();
        let current_level = stack.len() - 1;

        // Traverse the config tree to the active level's items.
        let mut items = &cfg.items;
        for i in 0..current_level {
            if let Some(idx) = stack[i].selected_child_index {
                if idx < items.len() {
                    items = &items[idx].items;
                } else {
                    return;
                }
            }
        }
        if items.is_empty() {
            return;
        }

        let (r_in, r_out) = cfg.get_level_radii(current_level);
        let dx = mx - cx;
        let dy = my - cy;
        let dist = (dx * dx + dy * dy).sqrt();

        let new_hover = if dist >= r_in && dist <= r_out {
            let gap_rad = cfg.item_gap_degrees.to_radians();
            let parent_angle = stack[current_level].parent_mid_angle;
            hit_test_index(current_level, dy.atan2(dx), items.len(), parent_angle, gap_rad)
        } else {
            None
        };

        if *h_m.read().unwrap() != new_hover {
            *h_m.write().unwrap() = new_hover;
            cv_m.queue_draw();
        }
    });
    canvas.add_controller(motion);

    // --- Primary Click Controller (left button only) ---
    let click = GestureClick::new();
    click.set_button(1);
    let nav_c = nav_stack.clone();
    let cfg_c = config.clone();
    let h_c = hover_index.clone();
    let cv_c = canvas.clone();
    let win_c = window.clone();
    let center_c = center_pos.clone();
    let shown_c = shown.clone();
    let bar_state_c = bar_state.clone();

    click.connect_pressed(move |_, _, mx, my| {
        let cfg = match cfg_c.read() {
            Ok(c) => c,
            Err(_) => return,
        };

        // ===== TOP PILL APPLET =====
        // The pill only exists at the root level. A click on a module runs its
        // `on_click` command (Waybar-style applet) and keeps the menu open so
        // further modules can be used.
        let is_root = nav_c.read().map(|s| s.len() == 1).unwrap_or(false);
        if is_root && cfg.top_bar.enabled {
            let outputs = bar_state_c.lock().map(|s| s.clone()).unwrap_or_default();
            let (cx, cy) = *center_c.read().unwrap();
            if let Some(layout) = top_bar_layout(&cfg.top_bar, cfg.outer_radius, &outputs, cx, cy)
            {
                if let Some(idx) = hit_test_pill(&layout, mx, my) {
                    let cmd = cfg.top_bar.modules[idx].on_click.clone();
                    if !cmd.is_empty() {
                        drop(cfg);
                        let _ = Command::new("sh").args(["-c", &cmd]).spawn();
                    }
                    return;
                }
            }
        }

        let mut stack = nav_c.write().unwrap();
        let current_level = stack.len() - 1;
        let (cx, cy) = *center_c.read().unwrap();

        let mut items = &cfg.items;
        for i in 0..current_level {
            if let Some(idx) = stack[i].selected_child_index {
                if idx < items.len() {
                    items = &items[idx].items;
                } else {
                    return;
                }
            }
        }
        if items.is_empty() {
            return;
        }

        let (r_in, r_out) = cfg.get_level_radii(current_level);
        let dx = mx - cx;
        let dy = my - cy;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist >= r_in && dist <= r_out {
            let gap_rad = cfg.item_gap_degrees.to_radians();
            let parent_angle = stack[current_level].parent_mid_angle;

            if let Some(idx) =
                hit_test_index(current_level, dy.atan2(dx), items.len(), parent_angle, gap_rad)
            {
                let clicked = &items[idx];

                if clicked.is_submenu() {
                    // SPAWN SUBMENU: highlight wedge red, fan out a new ring.
                    stack[current_level].selected_child_index = Some(idx);
                    let (a1, a2) =
                        get_item_angles(current_level, idx, items.len(), parent_angle, gap_rad);
                    let mid_angle = (a1 + a2) / 2.0;

                    stack.push(LevelSelection {
                        selected_child_index: None,
                        parent_mid_angle: mid_angle,
                    });

                     *h_c.write().unwrap() = None;
                    cv_c.queue_draw();
                } else {
                    // ACTION: either notify (safety mode) or execute the real command.
                    let label = clicked.label.clone();
                    let command = clicked.command.clone();
                    let notify_only = cfg.notify_only;
                    drop(stack);
                    drop(cfg);
                    if notify_only {
                        let _ = Command::new("notify-send")
                            .args(["-a", "radial-menu", &label, &command.join(" ")])
                            .spawn();
                    } else if !command.is_empty() {
                        let _ = Command::new(&command[0]).args(&command[1..]).spawn();
                    }
                    shown_c.store(false, Ordering::Relaxed);
                    win_c.hide();
                }
            }
        } else {
            // Clicked outside the active ring: pop a level, or close at root.
            if stack.len() > 1 {
                stack.pop();
                if let Some(parent) = stack.last_mut() {
                    parent.selected_child_index = None;
                }
                 *h_c.write().unwrap() = None;
                cv_c.queue_draw();
            } else {
                drop(stack);
                drop(cfg);
                shown_c.store(false, Ordering::Relaxed);
                win_c.hide();
            }
        }
    });
    canvas.add_controller(click);

    // --- Secondary Click Controller (right button: applet / pop / close) ---
    let rclick = GestureClick::new();
    rclick.set_button(3);
    let nav_r = nav_stack.clone();
    let h_r = hover_index.clone();
    let cv_r = canvas.clone();
    let win_r = window.clone();
    let shown_r = shown.clone();
    let cfg_r = config.clone();
    let center_r = center_pos.clone();
    let bar_state_r = bar_state.clone();

    rclick.connect_pressed(move |_, _, mx, my| {
        let cfg = match cfg_r.read() {
            Ok(c) => c,
            Err(_) => return,
        };

        // Top-pill module right-click runs its `on_click_right` applet command.
        let is_root = nav_r.read().map(|s| s.len() == 1).unwrap_or(false);
        if is_root && cfg.top_bar.enabled {
            let outputs = bar_state_r.lock().map(|s| s.clone()).unwrap_or_default();
            let (cx, cy) = *center_r.read().unwrap();
            if let Some(layout) = top_bar_layout(&cfg.top_bar, cfg.outer_radius, &outputs, cx, cy)
            {
                if let Some(idx) = hit_test_pill(&layout, mx, my) {
                    let cmd = cfg.top_bar.modules[idx].on_click_right.clone();
                    if !cmd.is_empty() {
                        drop(cfg);
                        let _ = Command::new("sh").args(["-c", &cmd]).spawn();
                    }
                    return;
                }
            }
        }
        drop(cfg);

        let mut stack = nav_r.write().unwrap();
        if stack.len() > 1 {
            stack.pop();
            if let Some(parent) = stack.last_mut() {
                parent.selected_child_index = None;
            }
             *h_r.write().unwrap() = None;
            cv_r.queue_draw();
        } else {
            drop(stack);
            shown_r.store(false, Ordering::Relaxed);
            win_r.hide();
        }
    });
    canvas.add_controller(rclick);

    // --- Escape / workspace keys: pop a level, close, or switch workspace ---
    let key = EventControllerKey::new();
    let win_k = window.clone();
    let nav_k = nav_stack.clone();
    let cv_k = canvas.clone();
    let shown_k = shown.clone();

    key.connect_key_pressed(move |_, keyval, _, _| {
        match keyval {
            Key::Escape => {
                let mut stack = nav_k.write().unwrap();
                if stack.len() > 1 {
                    stack.pop();
                    if let Some(parent) = stack.last_mut() {
                        parent.selected_child_index = None;
                    }
                    cv_k.queue_draw();
                } else {
                    drop(stack);
                    shown_k.store(false, Ordering::Relaxed);
                    win_k.hide();
                }
                Propagation::Stop
            }
            // Workspace controls: Page_Up = next, Page_Down = previous.
            Key::Page_Up => {
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "workspace", "+1"])
                    .spawn();
                Propagation::Stop
            }
            Key::Page_Down => {
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "workspace", "-1"])
                    .spawn();
                Propagation::Stop
            }
            _ => Propagation::Proceed,
        }
    });
    window.add_controller(key);

    // --- Mouse wheel: module scroll applets (e.g. volume) or workspace switch ---
    let scroll = EventControllerScroll::new(EventControllerScrollFlags::BOTH_AXES);
    let acc = Rc::new(Cell::new(0.0f64));
    let last_pos_s = last_pos.clone();
    let cfg_s = config.clone();
    let nav_s = nav_stack.clone();
    let center_s = center_pos.clone();
    let bar_state_s = bar_state.clone();
    scroll.connect_scroll(move |_, _dx, dy| {
        // If the pointer is over a pill module with scroll handlers, dispatch
        // those instead of switching workspaces (Waybar `on-scroll-*`).
        let pos = last_pos_s.get();
        let cfg = match cfg_s.read() {
            Ok(c) => c,
            Err(_) => return Propagation::Proceed,
        };
        let is_root = nav_s.read().map(|s| s.len() == 1).unwrap_or(false);
        if is_root && cfg.top_bar.enabled {
            let outputs = bar_state_s.lock().map(|s| s.clone()).unwrap_or_default();
            let (cx, cy) = *center_s.read().unwrap();
            if let Some(layout) = top_bar_layout(&cfg.top_bar, cfg.outer_radius, &outputs, cx, cy)
            {
                if let Some(idx) = hit_test_pill(&layout, pos.0, pos.1) {
                    let cmd = if dy > 0.0 {
                        cfg.top_bar.modules[idx].on_scroll_up.clone()
                    } else {
                        cfg.top_bar.modules[idx].on_scroll_down.clone()
                    };
                    if !cmd.is_empty() {
                        drop(cfg);
                        let _ = Command::new("sh").args(["-c", &cmd]).spawn();
                        return Propagation::Stop;
                    }
                }
            }
        }
        drop(cfg);

        let mut a = acc.get() + dy;
        if a >= 15.0 {
            let _ = Command::new("hyprctl")
                .args(["dispatch", "workspace", "+1"])
                .spawn();
            a = 0.0;
        } else if a <= -15.0 {
            let _ = Command::new("hyprctl")
                .args(["dispatch", "workspace", "-1"])
                .spawn();
            a = 0.0;
        }
        acc.set(a);
        Propagation::Proceed
    });
    canvas.add_controller(scroll);

    // ===== DAEMON SOCKET =====
    // A "toggle" from a future invocation of the binary is only signalled via
    // an atomic flag; the flag is consumed on the GTK main thread by a fast
    // timer below, keeping all widget access main-thread only. Keeping the
    // process alive is what makes reopening near-instant (no GTK boot, no
    // cursor probe, no cold modules) while preserving all behaviour.
    let toggle_pending = Arc::new(AtomicBool::new(false));
    if let Ok(listener) = UnixListener::bind(socket_path()) {
        let pending = toggle_pending.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 16];
                let _ = s.read(&mut buf);
                pending.store(true, Ordering::Relaxed);
            }
        });
    } else {
        // Failed to bind (another daemon won the race): toggle it and exit.
        if signal_toggle() {
            std::process::exit(0);
        }
    }

    // Fast main-loop poller that applies pending toggles.
    {
        let win_t = window.clone();
        let canvas_t = canvas.clone();
        let center_t = center_pos.clone();
        let nav_t = nav_stack.clone();
        let hover_t = hover_index.clone();
        let shown_t = shown.clone();
        let pending_t = toggle_pending.clone();
        timeout_add_local(Duration::from_millis(20), move || {
            if pending_t.swap(false, Ordering::Relaxed) {
                if shown_t.load(Ordering::Relaxed) {
                    // Hide the menu, keep the daemon alive.
                    shown_t.store(false, Ordering::Relaxed);
                    win_t.hide();
                } else {
                    // Reopen: recenter at the cursor and reset navigation.
                    shown_t.store(true, Ordering::Relaxed);
                    if let Some(pos) = cursor_local_pos() {
                        *center_t.write().unwrap() = pos;
                    }
                    *nav_t.write().unwrap() = vec![LevelSelection {
                        selected_child_index: None,
                        parent_mid_angle: 0.0,
                    }];
                    *hover_t.write().unwrap() = None;
                    win_t.set_keyboard_mode(KeyboardMode::Exclusive);
                    canvas_t.queue_draw();
                    win_t.present();
                }
            }
            ControlFlow::Continue
        });
    }

    window.present();
}

// =========================================================================
// Unit tests for the top-pill layout and click hit-testing
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TopBarConfig {
        TopBarConfig {
            enabled: true,
            height: 28.0,
            padding_x: 14.0,
            offset_y: 24.0,
            corner_radius: 14.0,
            background: [0.0; 4],
            foreground: [1.0; 4],
            font: default_font(),
            modules: vec![
                BarModule {
                    format: "{output}".into(),
                    icon: "A".into(),
                    ..Default::default()
                },
                BarModule {
                    format: "{output}".into(),
                    icon: "B".into(),
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn pill_layout_hit_test() {
        let cfg = test_config();
        let outputs = vec!["1".to_string(), "2".to_string()];
        let layout = top_bar_layout(&cfg, 100.0, &outputs, 500.0, 500.0).expect("layout");

        assert_eq!(layout.texts, vec!["A 1".to_string(), "B 2".to_string()]);
        assert_eq!(layout.indices, vec![0, 1]);

        // Click dead-centre of each module's text hits that module.
        let mid_y = layout.ry + layout.height / 2.0;
        let x0 = layout.lefts[0] + layout.widths[0] / 2.0;
        let x1 = layout.lefts[1] + layout.widths[1] / 2.0;
        assert_eq!(hit_test_pill(&layout, x0, mid_y), Some(0));
        assert_eq!(hit_test_pill(&layout, x1, mid_y), Some(1));

        // Inside the pill background but in the gap between modules: no hit.
        let gap_x = layout.lefts[0] + layout.widths[0] + 3.0;
        assert!(gap_x < layout.lefts[1]);
        assert_eq!(hit_test_pill(&layout, gap_x, layout.ry + 2.0), None);

        // Entirely outside the pill: no hit.
        assert_eq!(hit_test_pill(&layout, layout.rx - 5.0, mid_y), None);
        assert_eq!(hit_test_pill(&layout, x0, layout.ry - 5.0), None);
    }

    #[test]
    fn pill_layout_skips_empty_modules() {
        let mut cfg = test_config();
        // Icon-less module whose output is empty renders nothing -> skipped.
        cfg.modules[0].icon = String::new();
        let outputs = vec![String::new(), "2".to_string()];
        let layout = top_bar_layout(&cfg, 100.0, &outputs, 500.0, 500.0).expect("layout");
        // First module is empty -> skipped, so its text must map to module 1.
        assert_eq!(layout.indices, vec![1]);
        assert_eq!(
            hit_test_pill(
                &layout,
                layout.lefts[0] + layout.widths[0] / 2.0,
                layout.ry + layout.height / 2.0
            ),
            Some(1)
        );
    }
}
