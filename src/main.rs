use std::cell::Cell;
use std::io::{BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use gdk4::Key;
use gtk4::gdk;
use gtk4::glib::{timeout_add_local, ControlFlow, Propagation};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, CssProvider, DrawingArea, EventControllerKey,
    EventControllerMotion, EventControllerScroll, EventControllerScrollFlags, GestureClick,
};

mod config;
mod cursor;
mod draw;
mod nav;
mod process;
mod theme;
mod window;

use crate::config::{
    find_css_path, init_user_config, load_config_with_path, resolved_config_path,
    set_config_override, watch_config, RadialConfig,
};
use crate::cursor::cursor_local_pos;
use crate::draw::{
    draw_rounded_sector, draw_top_bar, hit_test_pill, pango_extents, pango_measure, pango_show,
    pango_weight, top_bar_layout,
};
use crate::nav::{get_item_angles, hit_test_index, LevelSelection};
use crate::process::{kill_process_group, run_shell};
use crate::theme::{watch_theme, Theme};

/// Directory for the single-instance socket: a per-user runtime directory
/// when one exists (`XDG_RUNTIME_DIR`, then `TMPDIR` on macOS), falling back
/// to the user-owned `~/.cache`. The world-writable `/tmp` is only used when
/// even `HOME` is unavailable — a predictable path there would let any local
/// user toggle or squat the daemon's socket.
fn runtime_dir() -> Option<std::path::PathBuf> {
    for var in ["XDG_RUNTIME_DIR", "TMPDIR"] {
        if let Ok(dir) = std::env::var(var) {
            if !dir.is_empty() {
                return Some(std::path::PathBuf::from(dir));
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(std::path::PathBuf::from(home).join(".cache"));
        }
    }
    None
}

/// Path of the Unix socket used to signal a running instance to toggle.
fn socket_path() -> std::path::PathBuf {
    runtime_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("hyprcircl.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_uses_runtime_dir_or_private_fallback() {
        let _guard = crate::config::env_lock();
        // XDG_RUNTIME_DIR wins when present.
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp/xdg");
        let p = socket_path();
        assert!(p.starts_with("/tmp/xdg"));
        assert_eq!(p.file_name().unwrap(), "hyprcircl.sock");
        std::env::remove_var("XDG_RUNTIME_DIR");

        // Without it, TMPDIR (per-user on macOS) is honoured...
        std::env::set_var("TMPDIR", "/tmp/user-tmp");
        assert!(socket_path().starts_with("/tmp/user-tmp"));
        std::env::remove_var("TMPDIR");

        // ...then a user-owned ~/.cache instead of shared /tmp.
        std::env::set_var("HOME", "/tmp/fake-home");
        assert!(socket_path().starts_with("/tmp/fake-home/.cache"));
        std::env::remove_var("HOME");

        // Only with no environment at all does /tmp remain.
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("TMPDIR");
        std::env::remove_var("HOME");
        assert_eq!(
            socket_path(),
            std::path::PathBuf::from("/tmp/hyprcircl.sock")
        );
    }

    #[test]
    fn needs_shell_detects_metacharacters() {
        assert!(!needs_shell(&["playerctl".into(), "play-pause".into()]));
        assert!(!needs_shell(&["pamixer".into(), "-i".into(), "5".into()]));
        assert!(needs_shell(&[
            "grim".into(),
            "-g".into(),
            "$(slurp)".into(),
            "-".into()
        ]));
        assert!(needs_shell(&["cliphist".into(), "list".into(), "|".into()]));
        assert!(!needs_shell(&[
            "omarchy-brightness-display".into(),
            "+5%".into()
        ]));
    }
}

// =========================================================================
// Fire-and-forget child bookkeeping
// =========================================================================

/// Every detached child spawned by input handlers is registered here and
/// reaped by a periodic GTK timer, so repeated clicks/scrolls never leave
/// zombie processes behind in the long-lived daemon.
static SPAWNED: Mutex<Vec<std::process::Child>> = Mutex::new(Vec::new());

fn spawn_tracked(cmd: &mut Command) -> bool {
    match cmd.spawn() {
        Ok(child) => {
            if let Ok(mut queue) = SPAWNED.lock() {
                queue.push(child);
            }
            true
        }
        Err(_) => false,
    }
}

fn reap_children() {
    if let Ok(mut queue) = SPAWNED.lock() {
        queue.retain_mut(|child| child.try_wait().map_or(true, |status| status.is_none()));
    }
}

// =========================================================================
// Command execution helpers
// =========================================================================

/// True when any argument carries shell syntax (`$(...)`, pipes, redirection,
/// globs, quotes...) so the command must go through `sh -c` instead of raw
/// argv execution.
fn needs_shell(command: &[String]) -> bool {
    command.iter().any(|arg| {
        arg.chars().any(|c| {
            matches!(
                c,
                '$' | '`'
                    | '|'
                    | ';'
                    | '&'
                    | '>'
                    | '<'
                    | '('
                    | ')'
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '~'
                    | '\\'
                    | '"'
                    | '\''
                    | '\n'
            )
        })
    })
}

/// Execute a menu item's `command`. Plain argv lists exec directly (no
/// quoting surprises); anything containing shell syntax is joined verbatim
/// and handed to `sh -c`, so shipped entries like
/// `["grim", "-g", "$(slurp)", "-"]` behave as written.
fn spawn_menu_command(command: &[String]) {
    if command.is_empty() {
        return;
    }
    if needs_shell(command) {
        let line = command.join(" ");
        let _ = spawn_tracked(Command::new("sh").args(["-c", &line]));
    } else {
        let mut cmd = Command::new(&command[0]);
        cmd.args(&command[1..]);
        let _ = spawn_tracked(&mut cmd);
    }
}

/// Run an applet shell snippet (pill click/scroll handlers).
fn spawn_shell(cmd: &str) -> bool {
    spawn_tracked(Command::new("sh").args(["-c", cmd]))
}

/// Render a command for display (notify_only mode): arguments containing
/// whitespace or quotes are single-quoted so the notification stays readable.
fn display_command(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            if arg
                .chars()
                .any(|c| c.is_whitespace() || c == '\'' || c == '"')
            {
                let escaped = arg.replace("'", "'\"'\"'");
                format!("'{escaped}'")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Switch Hyprland workspaces relative to the current one. No-op where
/// `hyprctl` does not exist (macOS).
#[cfg(target_os = "linux")]
fn dispatch_workspace(direction: &str) {
    let _ = spawn_tracked(Command::new("hyprctl").args(["dispatch", "workspace", direction]));
}

#[cfg(not(target_os = "linux"))]
fn dispatch_workspace(_direction: &str) {}

/// Exponential respawn backoff for failing stream commands: 500ms doubling,
/// capped at 30s, so a dead stream never becomes a fork-per-100ms loop.
fn stream_backoff(failures: u32) -> Duration {
    Duration::from_millis((250u64.saturating_mul(1 << failures.min(7))).min(30_000))
}

/// Socket commands understood by the running daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DaemonCmd {
    Toggle,
    Show,
    Hide,
    Quit,
}

impl DaemonCmd {
    fn parse(bytes: &[u8]) -> Option<Self> {
        match bytes {
            b"toggle" => Some(Self::Toggle),
            b"show" => Some(Self::Show),
            b"hide" => Some(Self::Hide),
            b"quit" | b"exit" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// Tell a running daemon to run `cmd`. Returns true if a daemon answered.
fn signal_daemon(cmd: DaemonCmd) -> bool {
    let path = socket_path();
    let msg: &[u8] = match cmd {
        DaemonCmd::Toggle => b"toggle",
        DaemonCmd::Show => b"show",
        DaemonCmd::Hide => b"hide",
        DaemonCmd::Quit => b"quit",
    };
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(msg);
        return true;
    }
    if cmd == DaemonCmd::Toggle {
        let _ = std::fs::remove_file(&path);
    }
    false
}

/// Toggle: if another instance is already running, tell it to show/hide the
/// menu over its socket and report `true` so this process exits. Otherwise
/// report `false` so this process becomes the persistent daemon.
fn signal_toggle() -> bool {
    signal_daemon(DaemonCmd::Toggle)
}

/// Re-anchor the menu center so the radial circle is exactly under the cursor.
///
/// Linux: the layer-shell surface spans the whole output, so the cursor is
/// taken in display-local coordinates (hyprctl, matching the canvas origin).
/// macOS: move the overlay onto the display under the cursor, then compute the
/// canvas center from the window's *actual* frame + cursor so the circle lands
/// on the cursor even when the frame doesn't exactly cover the display.
#[cfg(target_os = "macos")]
fn recenter_under_cursor(window: &gtk4::ApplicationWindow, center: &RwLock<Option<(f64, f64)>>) {
    if let Some(bounds) = window::macos_display_under_cursor() {
        window::macos_move_overlay_to(window, bounds);
    }
    if let Some(c) = window::macos_canvas_center_under_cursor(window) {
        if let Ok(mut center) = center.write() {
            *center = Some(c);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn recenter_under_cursor(_window: &gtk4::ApplicationWindow, center: &RwLock<Option<(f64, f64)>>) {
    if let Some(pos) = cursor_local_pos() {
        if let Ok(mut center) = center.write() {
            *center = Some(pos);
        }
    }
}

// =========================================================================
// Main Application Window & Controller Logic
// =========================================================================

// =========================================================================
// CLI
// =========================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!(
        r#"hyprcircl {VERSION} — radial launcher for Hyprland

USAGE:
    hyprcircl [COMMAND] [OPTIONS]

COMMANDS:
    (none)          Toggle the menu (default; starts daemon on first run)
    toggle          Same as no command
    show            Show the menu
    hide            Hide the menu
    quit            Stop the background daemon
    init            Create ~/.config/hyprcircl/ with starter config + theme
    config path     Print the active (or default) config file path

OPTIONS:
    -c, --config <PATH>   Use this config file (also HYPRCIRCL_CONFIG)
    -h, --help            Show this help
    -V, --version         Show version

EXAMPLES:
    hyprcircl init
    hyprcircl init --force
    hyprcircl --config ~/my-menu.toml
    hyprcircl config path

Hyprland keybinding:
    bind = $mainMod, Space, exec, hyprcircl
"#
    );
}

struct CliOptions {
    config: Option<String>,
    command: CliCommand,
}

enum CliCommand {
    RunDaemon,
    Init { force: bool },
    ConfigPath,
}

fn parse_cli() -> CliOptions {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut config = None;
    let mut command = CliCommand::RunDaemon;

    while !args.is_empty() {
        match args[0].as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("hyprcircl {VERSION}");
                std::process::exit(0);
            }
            "-c" | "--config" => {
                if args.len() < 2 {
                    eprintln!("error: --config requires a path");
                    std::process::exit(2);
                }
                config = Some(args.remove(1));
                args.remove(0);
            }
            "--force" => {
                if matches!(command, CliCommand::Init { .. }) {
                    command = CliCommand::Init { force: true };
                }
                args.remove(0);
            }
            "toggle" => {
                command = CliCommand::RunDaemon;
                args.remove(0);
            }
            "show" => {
                if signal_daemon(DaemonCmd::Show) {
                    std::process::exit(0);
                }
                eprintln!("hyprcircl: no running daemon");
                std::process::exit(1);
            }
            "hide" => {
                if signal_daemon(DaemonCmd::Hide) {
                    std::process::exit(0);
                }
                eprintln!("hyprcircl: no running daemon");
                std::process::exit(1);
            }
            "quit" | "exit" => {
                if signal_daemon(DaemonCmd::Quit) {
                    std::process::exit(0);
                }
                eprintln!("hyprcircl: no running daemon");
                std::process::exit(1);
            }
            "init" => {
                command = CliCommand::Init { force: false };
                args.remove(0);
            }
            "config" => {
                args.remove(0);
                match args.first().map(String::as_str) {
                    Some("path") => {
                        command = CliCommand::ConfigPath;
                        args.remove(0);
                    }
                    Some(other) => {
                        eprintln!("error: unknown config subcommand '{other}'");
                        std::process::exit(2);
                    }
                    None => {
                        command = CliCommand::ConfigPath;
                    }
                }
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown option '{other}'");
                std::process::exit(2);
            }
            _ => {
                eprintln!("error: unknown command '{}'", args[0]);
                eprintln!("run `hyprcircl --help` for usage");
                std::process::exit(2);
            }
        }
    }

    CliOptions { config, command }
}

fn main() {
    let cli = parse_cli();
    if let Some(path) = cli.config {
        set_config_override(path);
    }

    match cli.command {
        CliCommand::ConfigPath => {
            println!("{}", resolved_config_path().display());
            return;
        }
        CliCommand::Init { force } => match init_user_config(force) {
            Ok((toml, css)) => {
                println!("Created {}", toml.display());
                println!("Created {}", css.display());
                println!();
                println!("Edit the config, then bind a key:");
                println!("  bind = $mainMod, Space, exec, hyprcircl");
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        CliCommand::RunDaemon => {
            if signal_toggle() {
                std::process::exit(0);
            }
        }
    }

    if !matches!(cli.command, CliCommand::RunDaemon) {
        return;
    }

    run_daemon();
}

fn run_daemon() {
    let app = Application::builder()
        .application_id("com.omarchy.radial")
        .build();

    app.connect_activate(build_window);
    app.run();
}

fn build_window(app: &Application) {
    let window = ApplicationWindow::builder().application(app).build();
    window.add_css_class("hyprcircl");

    // Navigation state exists up front so the config-watcher's reload hook
    // can reset it (see below).
    let nav_stack = Arc::new(RwLock::new(vec![LevelSelection {
        selected_child_index: None,
        parent_mid_angle: 0.0,
    }]));
    let hover_index = Arc::new(RwLock::new(None::<usize>));

    // Load the config and remember exactly which file won — the watcher must
    // follow THAT file, not the first one that happens to exist on disk.
    let (loaded_cfg, config_file) = load_config_with_path();
    let config: Arc<RwLock<RadialConfig>> = Arc::new(RwLock::new(loaded_cfg));

    // Reload the config automatically when the file changes on disk. Any
    // live reload collapses navigation to the root so stale indices can
    // never address a differently-shaped menu tree.
    if let Some(path) = config_file.clone() {
        let nav_r = nav_stack.clone();
        let hover_r = hover_index.clone();
        watch_config(path, config.clone(), move || {
            if let Ok(mut stack) = nav_r.write() {
                *stack = vec![LevelSelection {
                    selected_child_index: None,
                    parent_mid_angle: 0.0,
                }];
            }
            if let Ok(mut hover) = hover_r.write() {
                *hover = None;
            }
        });
    }

    window::init_overlay(&window);

    // Shared theme: every visual property of the Cairo-drawn menu, seeded by
    // `hyprcircl.css` in the config directory. Live-reloads on file changes.
    let theme: Arc<RwLock<Theme>> = Arc::new(RwLock::new(Theme::default()));
    if let Some(css_path) = find_css_path(config_file.as_deref()) {
        println!("[CSS] Loading {css_path}");
        if let Ok(css) = std::fs::read_to_string(&css_path) {
            if let Some(display) = gdk::Display::default() {
                // Window/widget-level rules go through GTK's own CSS engine.
                let user_provider = CssProvider::new();
                user_provider.load_from_data(&css);
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &user_provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            // The rest of the file (`.item`, `.pill`, `.icon`, ...) drives our
            // Cairo drawing via the typed Theme.
            if let Ok(mut t) = theme.write() {
                *t = Theme::from_css(&css);
            }
        }
        watch_theme(css_path, theme.clone());
    }

    // Transparent-window rule for GTK's CSS engine.
    if let Some(display) = gdk::Display::default() {
        let provider = CssProvider::new();
        provider.load_from_data("window { background-color: transparent; }");
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    // Stationary menu center, anchored at the cursor. `None` until a real
    // cursor position is known — the renderer falls back to canvas center.
    // (A literal `(0, 0)` cursor position must not double as "unset".)
    let center_pos: Arc<RwLock<Option<(f64, f64)>>> = Arc::new(RwLock::new(cursor_local_pos()));

    // Whether the menu is currently visible (toggled by the daemon socket).
    let shown = Arc::new(AtomicBool::new(true));

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
            let mut last_gen: u64 = 0;
            loop {
                if !shown_w.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(150));
                    continue;
                }
                let (modules, gen) = config_w
                    .read()
                    .map(|c| (c.top_bar.modules.clone(), c.generation))
                    .unwrap_or_default();
                let n = modules.len();
                // Config changed on disk: drop the per-index caches so every
                // watch file is re-read once under its new meaning.
                if gen != last_gen {
                    last_gen = gen;
                    caches.clear();
                }
                caches.resize(n, Vec::new());

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
            // Per-stream respawn bookkeeping (see `stream_backoff`).
            let mut fail_counts: Vec<u32> = Vec::new();
            let mut next_try: Vec<Option<Instant>> = Vec::new();
            let mut last_gen: u64 = 0;
            loop {
                // SIGTERM/SIGINT: reap stream children, then die with the daemon.
                if shutdown_w.load(Ordering::Relaxed) {
                    for s in streams.iter_mut() {
                        if let Some(mut c) = s.take() {
                            kill_process_group(&mut c);
                        }
                    }
                    let _ = std::fs::remove_file(socket_path());
                    std::process::exit(0);
                }
                let shown = shown_w.load(Ordering::Relaxed);
                let (modules, gen) = config_w
                    .read()
                    .map(|c| (c.top_bar.modules.clone(), c.generation))
                    .unwrap_or_default();
                let n = modules.len();

                // Config changed on disk: every index-keyed piece of state is
                // stale. Drop streams/timers/outputs wholesale so the new
                // module list rebuilds them instead of inheriting the old
                // list's processes and text by position.
                if gen != last_gen {
                    last_gen = gen;
                    for s in streams.iter_mut() {
                        if let Some(mut c) = s.take() {
                            kill_process_group(&mut c);
                        }
                    }
                    streams.clear();
                    last.clear();
                    fail_counts.clear();
                    next_try.clear();
                    if let Ok(mut st) = state.lock() {
                        st.clear();
                    }
                    if let Ok(mut f) = flags.lock() {
                        f.clear();
                    }
                }

                if let Ok(mut st) = state.lock() {
                    if st.len() != n {
                        st.resize(n, String::new());
                    }
                }
                if last.len() != n {
                    let now = Instant::now();
                    let past = now.checked_sub(Duration::from_secs(3600)).unwrap_or(now);
                    last.resize(n, past);
                }
                fail_counts.resize(n, 0);
                next_try.resize(n, None);
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
                        // A stream that exits immediately (typo, missing
                        // binary, dead socket) must back off instead of being
                        // respawned every 100ms tick.
                        if let Some(t) = next_try[i] {
                            if now < t {
                                continue;
                            }
                        }
                        if let Some(mut child) = streams[i].take() {
                            match child.try_wait() {
                                Ok(Some(_)) => {
                                    // Exited: count as a failure and wait out
                                    // the backoff before respawning.
                                    fail_counts[i] = fail_counts[i].saturating_add(1);
                                    next_try[i] = Some(now + stream_backoff(fail_counts[i]));
                                    continue;
                                }
                                _ => {
                                    streams[i] = Some(child);
                                    continue;
                                }
                            }
                        }
                        let mut cmd = Command::new("sh");
                        cmd.args(["-c", &m.stream_command]);
                        cmd.stdout(Stdio::piped());
                        // Own process group (portable replacement for the
                        // external `setsid` wrapper, which does not exist on
                        // macOS): the child becomes its group leader so
                        // `kill_process_group` can reap the whole pipeline.
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::CommandExt;
                            cmd.process_group(0);
                        }
                        match cmd.spawn() {
                            Ok(mut child) => {
                                fail_counts[i] = 0;
                                next_try[i] = None;
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
                            Err(_) => {
                                fail_counts[i] = fail_counts[i].saturating_add(1);
                                next_try[i] = Some(now + stream_backoff(fail_counts[i]));
                            }
                        }
                    } else {
                        // Kill a leftover stream if the config switched modes.
                        if let Some(mut c) = streams[i].take() {
                            kill_process_group(&mut c);
                        }
                        // Clamp to >= 1s: interval = 0 would otherwise spawn
                        // the command on every 100ms loop tick.
                        let due = now.duration_since(last[i]).as_secs() >= m.interval.max(1);
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
    let theme_draw = theme.clone();

    // Render Function
    canvas.set_draw_func(move |_, cr, width, height| {
        let cfg = match config_draw.read() {
            Ok(c) => c,
            Err(_) => return,
        };
        let theme = match theme_draw.read() {
            Ok(t) => t,
            Err(_) => return,
        };

        // Resolve the menu center. `None` means "no cursor fix yet" — fall
        // back to canvas center once and remember it. A genuine cursor at
        // (0, 0) is a real position and stays put.
        let (cx, cy) = {
            let current = center_draw.read().ok().and_then(|c| *c);
            match current {
                Some(c) => c,
                None => {
                    let fallback = (width as f64 / 2.0, height as f64 / 2.0);
                    if let Ok(mut center) = center_draw.write() {
                        *center = Some(fallback);
                    }
                    fallback
                }
            }
        };

        let stack = match nav.read() {
            Ok(s) => s,
            Err(_) => return,
        };
        if stack.is_empty() {
            return;
        }

        // Read hover state once per frame instead of once per wedge.
        let hover = h_idx.read().ok().and_then(|h| *h);

        // ===== TOP PILL =====
        // Renders only at the root menu (nav stack depth == 1).
        if cfg.top_bar.enabled && stack.len() == 1 {
            let outputs = bar_state_draw.lock().map(|s| s.clone()).unwrap_or_default();
            if let Some(layout) =
                top_bar_layout(&cfg.top_bar, &theme, cfg.outer_radius, &outputs, cx, cy)
            {
                draw_top_bar(cr, &layout, &theme);
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

                draw_rounded_sector(cr, cx, cy, r_in, r_out, a1, a2, theme.item_corner);

                // --- COLOR SELECTION ---
                // Parent branch slice leading to an open submenu -> selected;
                // hovered slice of the active ring -> hover; the active ring
                // itself -> active; everything else -> default.
                let color = if selected_child == Some(i) {
                    theme.item_selected
                } else if is_active_level && hover == Some(i) {
                    theme.item_hover
                } else if is_active_level {
                    theme.item_active
                } else {
                    theme.item_default
                };
                cr.set_source_rgba(color.0, color.1, color.2, color.3);
                let _ = cr.fill_preserve();

                // Stroke
                let stroke = theme.item_stroke;
                cr.set_source_rgba(stroke.0, stroke.1, stroke.2, stroke.3);
                cr.set_line_width(theme.item_stroke_width);
                let _ = cr.stroke();

                // Icon/Label via Pango (handles Nerd Font glyphs)
                let mid_angle = (a1 + a2) / 2.0;
                let mid_r = (r_in + r_out) / 2.0;
                let tx = cx + mid_r * mid_angle.cos();
                let ty = cy + mid_r * mid_angle.sin();

                let icon_c = theme.icon_color;
                cr.set_source_rgba(icon_c.0, icon_c.1, icon_c.2, icon_c.3);

                // Center icon in wedge using its measured extents so the
                // offsets hold for any font size.
                let icon_size = (theme.icon_font_size * pango::SCALE as f64) as i32;
                let icon_weight = pango_weight(theme.icon_font_weight);
                let (icon_w, icon_h) =
                    pango_extents(&theme.icon_font, icon_size, icon_weight, &item.icon);
                cr.move_to(tx - icon_w / 2.0, ty - icon_h / 2.0 - 3.0);
                pango_show(cr, &theme.icon_font, icon_size, icon_weight, &item.icon);

                // Label below icon (smaller to avoid overflow).
                // Hidden when TOML `show_labels` is off OR CSS sets `.label { display: none; }`.
                if cfg.show_labels && theme.label_visible {
                    let label_c = theme.label_color;
                    cr.set_source_rgba(label_c.0, label_c.1, label_c.2, label_c.3);
                    let label_size = (theme.label_font_size * pango::SCALE as f64) as i32;
                    let label_weight = pango_weight(theme.label_font_weight);
                    let label_w =
                        pango_measure(&theme.label_font, label_size, label_weight, &item.label);
                    cr.move_to(tx - label_w / 2.0, ty + icon_h / 2.0 + 1.0);
                    pango_show(cr, &theme.label_font, label_size, label_weight, &item.label);
                }
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
        let stack = match nav_m.read() {
            Ok(s) => s,
            Err(_) => return,
        };
        if stack.is_empty() {
            return;
        }
        let Some((cx, cy)) = center_m.read().ok().and_then(|c| *c) else {
            return;
        };
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
            hit_test_index(
                current_level,
                dy.atan2(dx),
                items.len(),
                parent_angle,
                gap_rad,
            )
        } else {
            None
        };

        let changed = h_m.read().map(|h| *h != new_hover).unwrap_or(false);
        if changed {
            if let Ok(mut hover) = h_m.write() {
                *hover = new_hover;
            }
            cv_m.queue_draw();
        }
    });

    // Pointer left the canvas: drop any stale highlight so a wedge on the
    // active ring can't stay lit while the cursor is elsewhere.
    {
        let h_l = hover_index.clone();
        let cv_l = canvas.clone();
        motion.connect_leave(move |_| {
            let had_hover = h_l.read().map(|h| h.is_some()).unwrap_or(false);
            if had_hover {
                if let Ok(mut hover) = h_l.write() {
                    *hover = None;
                }
                cv_l.queue_draw();
            }
        });
    }
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
    let theme_c = theme.clone();

    click.connect_pressed(move |_, _, mx, my| {
        let cfg = match cfg_c.read() {
            Ok(c) => c,
            Err(_) => return,
        };
        let theme = match theme_c.read() {
            Ok(t) => t,
            Err(_) => return,
        };

        // ===== TOP PILL APPLET =====
        // The pill only exists at the root level. A click on a module runs its
        // `on_click` command (Waybar-style applet) and keeps the menu open so
        // further modules can be used.
        let is_root = nav_c.read().map(|s| s.len() == 1).unwrap_or(false);
        if is_root && cfg.top_bar.enabled {
            let Some((cx, cy)) = center_c.read().ok().and_then(|c| *c) else {
                return;
            };
            let outputs = bar_state_c.lock().map(|s| s.clone()).unwrap_or_default();
            if let Some(layout) =
                top_bar_layout(&cfg.top_bar, &theme, cfg.outer_radius, &outputs, cx, cy)
            {
                if let Some(idx) = hit_test_pill(&layout, mx, my) {
                    let cmd = cfg.top_bar.modules[idx].on_click.clone();
                    if !cmd.is_empty() {
                        drop(cfg);
                        spawn_shell(&cmd);
                    }
                    return;
                }
            }
        }

        let mut stack = match nav_c.write() {
            Ok(s) => s,
            Err(_) => return,
        };
        let current_level = stack.len() - 1;
        let Some((cx, cy)) = center_c.read().ok().and_then(|c| *c) else {
            return;
        };

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

            if let Some(idx) = hit_test_index(
                current_level,
                dy.atan2(dx),
                items.len(),
                parent_angle,
                gap_rad,
            ) {
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

                    if let Ok(mut hover) = h_c.write() {
                        *hover = None;
                    }
                    cv_c.queue_draw();
                } else {
                    // ACTION: either notify (safety mode) or execute the real command.
                    let label = clicked.label.clone();
                    let command = clicked.command.clone();
                    let notify_only = cfg.notify_only;
                    drop(stack);
                    drop(cfg);
                    if notify_only {
                        let _ = spawn_tracked(Command::new("notify-send").args([
                            "-a",
                            "hyprcircl",
                            &label,
                            &display_command(&command),
                        ]));
                    } else {
                        spawn_menu_command(&command);
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
                if let Ok(mut hover) = h_c.write() {
                    *hover = None;
                }
                cv_c.queue_draw();
            } else {
                // Release locks before driving widget state.
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
    let theme_r = theme.clone();

    rclick.connect_pressed(move |_, _, mx, my| {
        let cfg = match cfg_r.read() {
            Ok(c) => c,
            Err(_) => return,
        };
        let theme = match theme_r.read() {
            Ok(t) => t,
            Err(_) => return,
        };

        // Top-pill module right-click runs its `on_click_right` applet command.
        let is_root = nav_r.read().map(|s| s.len() == 1).unwrap_or(false);
        if is_root && cfg.top_bar.enabled {
            let Some((cx, cy)) = center_r.read().ok().and_then(|c| *c) else {
                return;
            };
            let outputs = bar_state_r.lock().map(|s| s.clone()).unwrap_or_default();
            if let Some(layout) =
                top_bar_layout(&cfg.top_bar, &theme, cfg.outer_radius, &outputs, cx, cy)
            {
                if let Some(idx) = hit_test_pill(&layout, mx, my) {
                    let cmd = cfg.top_bar.modules[idx].on_click_right.clone();
                    if !cmd.is_empty() {
                        drop(cfg);
                        spawn_shell(&cmd);
                    }
                    return;
                }
            }
        }
        drop(cfg);

        let mut stack = match nav_r.write() {
            Ok(s) => s,
            Err(_) => return,
        };
        if stack.len() > 1 {
            stack.pop();
            if let Some(parent) = stack.last_mut() {
                parent.selected_child_index = None;
            }
            if let Ok(mut hover) = h_r.write() {
                *hover = None;
            }
            cv_r.queue_draw();
        } else {
            drop(stack);
            shown_r.store(false, Ordering::Relaxed);
            win_r.hide();
        }
    });
    canvas.add_controller(rclick);

    // --- Escape / workspace / keyboard navigation ---
    let key = EventControllerKey::new();
    let win_k = window.clone();
    let nav_k = nav_stack.clone();
    let h_k = hover_index.clone();
    let cv_k = canvas.clone();
    let shown_k = shown.clone();
    let cfg_k = config.clone();

    key.connect_key_pressed(move |_, keyval, _, _| {
        let cfg = match cfg_k.read() {
            Ok(c) => c,
            Err(_) => return Propagation::Proceed,
        };

        // Keyboard wedge navigation (left/right, 1-9, enter, backspace).
        if cfg.keyboard_navigation {
            let count = {
                let stack = match nav_k.read() {
                    Ok(s) => s,
                    Err(_) => return Propagation::Proceed,
                };
                let current_level = stack.len().saturating_sub(1);
                let mut items = &cfg.items;
                for i in 0..current_level {
                    if let Some(idx) = stack[i].selected_child_index {
                        if idx < items.len() {
                            items = &items[idx].items;
                        } else {
                            return Propagation::Proceed;
                        }
                    }
                }
                items.len()
            };

            let cycle_hover = |delta: isize| {
                if count == 0 {
                    return;
                }
                let cur = h_k.read().ok().and_then(|h| *h).unwrap_or(0);
                let next = (cur as isize + delta).rem_euclid(count as isize) as usize;
                if h_k.read().map(|h| *h != Some(next)).unwrap_or(true) {
                    if let Ok(mut hover) = h_k.write() {
                        *hover = Some(next);
                    }
                    cv_k.queue_draw();
                }
            };

            match keyval {
                Key::Left | Key::h => {
                    cycle_hover(-1);
                    return Propagation::Stop;
                }
                Key::Right | Key::l => {
                    cycle_hover(1);
                    return Propagation::Stop;
                }
                Key::BackSpace => {
                    if let Ok(mut stack) = nav_k.write() {
                        if stack.len() > 1 {
                            stack.pop();
                            if let Some(parent) = stack.last_mut() {
                                parent.selected_child_index = None;
                            }
                            if let Ok(mut hover) = h_k.write() {
                                *hover = None;
                            }
                            cv_k.queue_draw();
                            return Propagation::Stop;
                        }
                    }
                }
                Key::Return | Key::KP_Enter | Key::space => {
                    let idx = h_k.read().ok().and_then(|h| *h).unwrap_or(0);
                    if count == 0 || idx >= count {
                        return Propagation::Proceed;
                    }
                    let mut stack = match nav_k.write() {
                        Ok(s) => s,
                        Err(_) => return Propagation::Proceed,
                    };
                    let current_level = stack.len() - 1;
                    let mut items: &Vec<crate::config::MenuItem> = &cfg.items;
                    for i in 0..current_level {
                        if let Some(iidx) = stack[i].selected_child_index {
                            if iidx < items.len() {
                                items = &items[iidx].items;
                            } else {
                                return Propagation::Proceed;
                            }
                        }
                    }
                    let clicked = &items[idx];
                    if clicked.is_submenu() {
                        stack[current_level].selected_child_index = Some(idx);
                        let gap_rad = cfg.item_gap_degrees.to_radians();
                        let parent_angle = stack[current_level].parent_mid_angle;
                        let (a1, a2) =
                            get_item_angles(current_level, idx, items.len(), parent_angle, gap_rad);
                        let mid_angle = (a1 + a2) / 2.0;
                        stack.push(LevelSelection {
                            selected_child_index: None,
                            parent_mid_angle: mid_angle,
                        });
                        if let Ok(mut hover) = h_k.write() {
                            *hover = None;
                        }
                        cv_k.queue_draw();
                    } else {
                        let label = clicked.label.clone();
                        let command = clicked.command.clone();
                        let notify_only = cfg.notify_only;
                        drop(stack);
                        if notify_only {
                            let _ = spawn_tracked(Command::new("notify-send").args([
                                "-a",
                                "hyprcircl",
                                &label,
                                &display_command(&command),
                            ]));
                        } else {
                            spawn_menu_command(&command);
                        }
                        shown_k.store(false, Ordering::Relaxed);
                        win_k.hide();
                    }
                    return Propagation::Stop;
                }
                _ => {
                    if let Some(digit) = keyval.to_unicode() {
                        if digit.is_ascii_digit() && digit != '0' {
                            let n = (digit as u8 - b'0') as usize;
                            if n <= count && count > 0 {
                                let pick = n - 1;
                                if let Ok(mut hover) = h_k.write() {
                                    *hover = Some(pick);
                                }
                                cv_k.queue_draw();
                                return Propagation::Stop;
                            }
                        }
                    }
                }
            }
        }

        match keyval {
            Key::Escape => {
                // Popping a level must clear hover too: the stored index came
                // from the popped ring and would light the wrong wedge on the
                // newly active one until the pointer moved.
                if let Ok(mut stack) = nav_k.write() {
                    if stack.len() > 1 {
                        stack.pop();
                        if let Some(parent) = stack.last_mut() {
                            parent.selected_child_index = None;
                        }
                        if let Ok(mut hover) = h_k.write() {
                            *hover = None;
                        }
                        cv_k.queue_draw();
                    } else {
                        drop(stack);
                        shown_k.store(false, Ordering::Relaxed);
                        win_k.hide();
                    }
                }
                Propagation::Stop
            }
            // Workspace controls: Page_Up = next, Page_Down = previous.
            Key::Page_Up => {
                dispatch_workspace("+1");
                Propagation::Stop
            }
            Key::Page_Down => {
                dispatch_workspace("-1");
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
    let theme_s = theme.clone();
    scroll.connect_scroll(move |_, _dx, dy| {
        // If the pointer is over a pill module with scroll handlers, dispatch
        // those instead of switching workspaces (Waybar `on-scroll-*`).
        // GTK4 scroll deltas: dy < 0 = wheel UP, dy > 0 = wheel DOWN.
        let pos = last_pos_s.get();
        let cfg = match cfg_s.read() {
            Ok(c) => c,
            Err(_) => return Propagation::Proceed,
        };
        let theme = match theme_s.read() {
            Ok(t) => t,
            Err(_) => return Propagation::Proceed,
        };
        let is_root = nav_s.read().map(|s| s.len() == 1).unwrap_or(false);
        if is_root && cfg.top_bar.enabled {
            let Some((cx, cy)) = center_s.read().ok().and_then(|c| *c) else {
                return Propagation::Proceed;
            };
            let outputs = bar_state_s.lock().map(|s| s.clone()).unwrap_or_default();
            if let Some(layout) =
                top_bar_layout(&cfg.top_bar, &theme, cfg.outer_radius, &outputs, cx, cy)
            {
                if let Some(idx) = hit_test_pill(&layout, pos.0, pos.1) {
                    let cmd = if dy < 0.0 {
                        cfg.top_bar.modules[idx].on_scroll_up.clone()
                    } else {
                        cfg.top_bar.modules[idx].on_scroll_down.clone()
                    };
                    if !cmd.is_empty() {
                        drop(cfg);
                        spawn_shell(&cmd);
                        return Propagation::Stop;
                    }
                }
            }
        }
        drop(cfg);

        let mut a = acc.get() + dy;
        if a >= 15.0 {
            dispatch_workspace("+1");
            a = 0.0;
        } else if a <= -15.0 {
            dispatch_workspace("-1");
            a = 0.0;
        }
        acc.set(a);
        Propagation::Proceed
    });
    canvas.add_controller(scroll);

    // ===== DAEMON SOCKET =====
    // Commands from future invocations are queued as a small integer and
    // consumed on the GTK main thread by the fast timer below.
    const CMD_NONE: u8 = 0;
    const CMD_TOGGLE: u8 = 1;
    const CMD_SHOW: u8 = 2;
    const CMD_HIDE: u8 = 3;
    const CMD_QUIT: u8 = 4;

    let daemon_cmd = Arc::new(std::sync::atomic::AtomicU8::new(CMD_NONE));
    if let Ok(listener) = UnixListener::bind(socket_path()) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(socket_path(), std::fs::Permissions::from_mode(0o600));
        }
        let pending = daemon_cmd.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 16];
                let n = s.read(&mut buf).unwrap_or(0);
                let code = match DaemonCmd::parse(&buf[..n]) {
                    Some(DaemonCmd::Toggle) => CMD_TOGGLE,
                    Some(DaemonCmd::Show) => CMD_SHOW,
                    Some(DaemonCmd::Hide) => CMD_HIDE,
                    Some(DaemonCmd::Quit) => CMD_QUIT,
                    None => continue,
                };
                pending.store(code, Ordering::Relaxed);
            }
        });
    } else {
        eprintln!("[SOCKET] could not bind {}", socket_path().display());
        if signal_toggle() {
            std::process::exit(0);
        }
        eprintln!("[SOCKET] no live daemon answered; running without single-instance toggle");
    }

    // Reap finished fire-and-forget children so clicks/scrolls never leave
    // zombies behind.
    {
        timeout_add_local(Duration::from_millis(500), move || {
            reap_children();
            ControlFlow::Continue
        });
    }

    // Fast main-loop poller that applies pending socket commands.
    {
        let win_t = window.clone();
        let canvas_t = canvas.clone();
        let center_t = center_pos.clone();
        let nav_t = nav_stack.clone();
        let hover_t = hover_index.clone();
        let shown_t = shown.clone();
        let pending_t = daemon_cmd.clone();
        timeout_add_local(Duration::from_millis(20), move || {
            let cmd = pending_t.swap(CMD_NONE, Ordering::Relaxed);
            if cmd == CMD_QUIT {
                let _ = std::fs::remove_file(socket_path());
                std::process::exit(0);
            }
            let show = cmd == CMD_SHOW || (cmd == CMD_TOGGLE && !shown_t.load(Ordering::Relaxed));
            let hide = cmd == CMD_HIDE || (cmd == CMD_TOGGLE && shown_t.load(Ordering::Relaxed));
            if hide {
                shown_t.store(false, Ordering::Relaxed);
                win_t.hide();
            } else if show {
                shown_t.store(true, Ordering::Relaxed);
                if let Ok(mut stack) = nav_t.write() {
                    *stack = vec![LevelSelection {
                        selected_child_index: None,
                        parent_mid_angle: 0.0,
                    }];
                }
                if let Ok(mut hover) = hover_t.write() {
                    *hover = None;
                }
                window::set_keyboard_exclusive(&win_t, true);
                canvas_t.queue_draw();
                win_t.present();
                recenter_under_cursor(&win_t, &center_t);
            }
            ControlFlow::Continue
        });
    }

    window.present();

    #[cfg(target_os = "macos")]
    {
        // Position the overlay before the first frame renders (present() is
        // synchronous and creates the surface), so launch shows no flash.
        if let Some(bounds) = window::macos_display_under_cursor() {
            window::macos_move_overlay_to(&window, bounds);
        }
        // Retry-loop safety net for config/positioning if the surface wasn't
        // created synchronously (e.g. cold GTK boot).
        window::setup_macos_overlay(&window);
        // Snap the circle center to the cursor using the window's actual
        // frame (the window may not cover the display exactly, e.g. the
        // macOS menu-bar strip), so the menu always spawns on the cursor.
        recenter_under_cursor(&window, &center_pos);
    }
}
