use std::cell::Cell;
use std::io::{BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use gdk4::Key;
use gtk4::glib::{ControlFlow, Propagation, timeout_add_local};
use gtk4::gdk;
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
mod window;

use crate::config::{find_config_path, find_css_path, load_config, watch_config, RadialConfig};
use crate::cursor::cursor_local_pos;
use crate::draw::{
    draw_rounded_sector, draw_top_bar, hit_test_pill, pango_measure, pango_show, top_bar_layout,
};
use crate::nav::{get_item_angles, hit_test_index, LevelSelection};
use crate::process::{kill_process_group, run_shell};

/// Path of the Unix socket used to signal a running instance to toggle.
fn socket_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(dir).join("hyprcircl.sock")
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

/// Re-anchor the menu center so the radial circle is exactly under the cursor.
///
/// Linux: the layer-shell surface spans the whole output, so the cursor is
/// taken in display-local coordinates (hyprctl, matching the canvas origin).
/// macOS: move the overlay onto the display under the cursor, then compute the
/// canvas center from the window's *actual* frame + cursor so the circle lands
/// on the cursor even when the frame doesn't exactly cover the display.
#[cfg(target_os = "macos")]
fn recenter_under_cursor(window: &gtk4::ApplicationWindow, center: &RwLock<(f64, f64)>) {
    if let Some(bounds) = window::macos_display_under_cursor() {
        window::macos_move_overlay_to(window, bounds);
    }
    if let Some(c) = window::macos_canvas_center_under_cursor(window) {
        *center.write().unwrap() = c;
    }
}

#[cfg(not(target_os = "macos"))]
fn recenter_under_cursor(_window: &gtk4::ApplicationWindow, center: &RwLock<(f64, f64)>) {
    if let Some(pos) = cursor_local_pos() {
        *center.write().unwrap() = pos;
    }
}

// =========================================================================
// Main Application Window & Controller Logic
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
    window.add_css_class("hyprcircl");
    let config: Arc<RwLock<RadialConfig>> = Arc::new(RwLock::new(load_config()));

    // Reload the config automatically when the file changes on disk.
    if let Some(path) = find_config_path() {
        watch_config(path, config.clone());
    }

    window::init_overlay(&window);

    let provider = CssProvider::new();
    provider.load_from_data("window { background-color: transparent; }");
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Load user CSS if available (hyprcircl.css in the config directory).
    if let Some(css_path) = find_css_path() {
        println!("[CSS] Loading {css_path}");
        if let Ok(css) = std::fs::read_to_string(&css_path) {
            let user_provider = CssProvider::new();
            user_provider.load_from_data(&css);
            gtk4::style_context_add_provider_for_display(
                &gdk::Display::default().unwrap(),
                &user_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

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

                // Icon/Label via Pango (handles Nerd Font glyphs)
                let mid_angle = (a1 + a2) / 2.0;
                let mid_r = (r_in + r_out) / 2.0;
                let tx = cx + mid_r * mid_angle.cos();
                let ty = cy + mid_r * mid_angle.sin();

                cr.set_source_rgba(0.05, 0.05, 0.1, 1.0);

                // Center icon in wedge
                let icon_w = pango_measure(&cfg.top_bar.font, 20 * pango::SCALE, &item.icon);
                cr.move_to(tx - icon_w / 2.0, ty - 12.0);
                pango_show(cr, &cfg.top_bar.font, 20 * pango::SCALE, &item.icon);

                // Label below icon (smaller to avoid overflow)
                if cfg.show_labels {
                    cr.set_source_rgba(0.15, 0.15, 0.2, 0.85);
                    let label_w = pango_measure(&cfg.top_bar.font, 8 * pango::SCALE, &item.label);
                    cr.move_to(tx - label_w / 2.0, ty + 10.0);
                    pango_show(cr, &cfg.top_bar.font, 8 * pango::SCALE, &item.label);
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
                            .args(["-a", "hyprcircl", &label, &command.join(" ")])
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
                    *nav_t.write().unwrap() = vec![LevelSelection {
                        selected_child_index: None,
                        parent_mid_angle: 0.0,
                    }];
                    *hover_t.write().unwrap() = None;
                    window::set_keyboard_exclusive(&win_t, true);
                    canvas_t.queue_draw();
                    win_t.present();

                    // Move the overlay onto the display holding the cursor and
                    // snap the circle center to it. Must happen AFTER present —
                    // the macOS backend defers showing until the next frame
                    // swap, so this lands before anything is rendered and
                    // avoids a one-frame flash at the old location. On Linux
                    // the layer-shell window already spans every monitor.
                    recenter_under_cursor(&win_t, &center_t);
                }
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
