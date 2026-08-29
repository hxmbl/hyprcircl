#[cfg(target_os = "macos")]
use crate::window;

// =========================================================================
// Cursor System
// =========================================================================

#[cfg(target_os = "linux")]
fn run(cmd: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn which(cmd: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {cmd} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Cursor position in the active menu's local canvas coordinates.
///
/// Linux: queried from hyprctl and converted into the monitor's local space.
/// macOS: read from CoreGraphics and converted into the display's local
/// space (the overlay window is moved to cover that same display, so the two
/// coordinate systems line up exactly).
#[cfg(target_os = "linux")]
pub fn cursor_local_pos() -> Option<(f64, f64)> {
    if which("hyprctl") {
        if let Some(res) = run(&["hyprctl", "-j", "cursorpos"]) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&res) {
                if let (Some(x), Some(y)) = (
                    v.get("x").and_then(|n| n.as_f64()),
                    v.get("y").and_then(|n| n.as_f64()),
                ) {
                    if let Some(m_res) = run(&["hyprctl", "-j", "monitors"]) {
                        if let Ok(mv) = serde_json::from_str::<serde_json::Value>(&m_res) {
                            if let Some(mons) = mv.as_array() {
                                for mon in mons {
                                    // A malformed entry must only skip that
                                    // monitor, never abort the whole query.
                                    let get = |k: &str| mon.get(k).and_then(|n| n.as_f64());
                                    if let (Some(mx), Some(my), Some(mw), Some(mh)) =
                                        (get("x"), get("y"), get("width"), get("height"))
                                    {
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
    }
    None
}

#[cfg(target_os = "macos")]
pub fn cursor_local_pos() -> Option<(f64, f64)> {
    let (x, y) = window::macos_cursor_point()?;
    let (dx, dy, _w, _h) = window::macos_display_under_cursor()?;
    Some((x - dx, y - dy))
}
