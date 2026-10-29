// =========================================================================
// `hyprcircl preview`: render one frame of the menu to a PNG without a
// compositor. It uses the same config/theme loading and the same
// `draw::draw_menu` as the live window, so it is a faithful way to iterate
// on a theme (and how the screenshots in docs/ are made).
// =========================================================================

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::config::{find_css_path, load_config_with_path, RadialConfig};
use crate::draw::{draw_menu, MenuFrame};
use crate::nav::{get_item_angles, LevelSelection};
use crate::process::run_shell;
use crate::theme::Theme;

pub struct PreviewOptions {
    pub out: String,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// Submenus to open, as item indices from the root down.
    pub open: Vec<usize>,
    /// Hovered wedge on the deepest open ring.
    pub hover: Option<usize>,
    /// Optional wallpaper PNG, scaled to cover the canvas.
    pub background: Option<String>,
    /// Menu centre in logical pixels (default: canvas centre).
    pub center: Option<(f64, f64)>,
}

/// First line a stream command prints, waiting at most `timeout`.
fn first_stream_line(cmd: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new("sh")
        .args(["-c", cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(timeout).ok();
    let _ = child.kill();
    let _ = child.wait();
    line.map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

/// Current text of every bar module, as the live pill would show it.
fn bar_outputs(cfg: &RadialConfig) -> Vec<String> {
    cfg.top_bar
        .modules
        .iter()
        .map(|m| {
            if !m.stream_command.is_empty() {
                first_stream_line(&m.stream_command, Duration::from_secs(2)).unwrap_or_default()
            } else if !m.command.is_empty() {
                run_shell(&m.command)
                    .map(|o| o.trim().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        })
        .collect()
}

/// Build the nav stack for `open` (root -> ... -> deepest), validating each
/// index against the config's menu tree.
fn nav_stack(cfg: &RadialConfig, open: &[usize]) -> Result<Vec<LevelSelection>, String> {
    let mut stack = vec![LevelSelection {
        selected_child_index: None,
        parent_mid_angle: 0.0,
    }];
    let mut items = &cfg.items;
    for (level, &idx) in open.iter().enumerate() {
        let item = items
            .get(idx)
            .ok_or_else(|| format!("--open: level {level} has no item {idx}"))?;
        if !item.is_submenu() {
            return Err(format!("--open: \"{}\" has no submenu", item.label));
        }
        let parent = stack[level].parent_mid_angle;
        let (a1, a2) = get_item_angles(level, idx, items.len(), parent, cfg.gap_rad());
        stack[level].selected_child_index = Some(idx);
        stack.push(LevelSelection {
            selected_child_index: None,
            parent_mid_angle: (a1 + a2) / 2.0,
        });
        items = &item.items;
    }
    Ok(stack)
}

pub fn run(opts: PreviewOptions) -> Result<(), String> {
    use gtk4::cairo::{Context, Format, ImageSurface};

    let (cfg, cfg_path) = load_config_with_path();
    let theme = match find_css_path(cfg_path.as_deref()) {
        Some(css) => {
            let text = std::fs::read_to_string(&css).map_err(|e| format!("{css}: {e}"))?;
            Theme::from_css(&text)
        }
        None => Theme::default(),
    };
    let stack = nav_stack(&cfg, &opts.open)?;
    let outputs = bar_outputs(&cfg);

    let pw = (opts.width as f64 * opts.scale).round() as i32;
    let ph = (opts.height as f64 * opts.scale).round() as i32;
    let surface = ImageSurface::create(Format::ARgb32, pw, ph).map_err(|e| e.to_string())?;
    let cr = Context::new(&surface).map_err(|e| e.to_string())?;

    if let Some(bg) = &opts.background {
        let mut file = std::fs::File::open(bg).map_err(|e| format!("{bg}: {e}"))?;
        let img = ImageSurface::create_from_png(&mut file).map_err(|e| format!("{bg}: {e}"))?;
        let sx = pw as f64 / img.width() as f64;
        let sy = ph as f64 / img.height() as f64;
        let s = sx.max(sy);
        let _ = cr.save();
        cr.scale(s, s);
        let _ = cr.set_source_surface(&img, 0.0, 0.0);
        let _ = cr.paint();
        let _ = cr.restore();
    }

    // Everything below is in logical pixels, like a HiDPI GTK surface.
    cr.scale(opts.scale, opts.scale);
    let (cx, cy) = opts
        .center
        .unwrap_or((opts.width as f64 / 2.0, opts.height as f64 / 2.0));
    let frame = MenuFrame {
        stack: &stack,
        hover: opts.hover,
        outputs: &outputs,
        cx,
        cy,
        open: 1.0,
        expand: 1.0,
    };
    draw_menu(
        &cr,
        &cfg,
        &theme,
        &frame,
        opts.width as f64,
        opts.height as f64,
    );
    drop(cr);
    let mut out = std::fs::File::create(&opts.out).map_err(|e| format!("{}: {e}", opts.out))?;
    surface
        .write_to_png(&mut out)
        .map_err(|e| format!("{}: {e}", opts.out))?;
    Ok(())
}
