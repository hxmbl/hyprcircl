use std::f64::consts::PI;

use gtk4::cairo::{self};
use gtk4::glib;
use gtk4::glib::translate::ToGlibPtr;
use gtk4::pango;

use crate::config::TopBarConfig;

// pangocairo FFI — symbols are already linked through gtk4
extern "C" {
    fn pango_cairo_font_map_get_default() -> *mut std::ffi::c_void;
    fn pango_font_map_create_context(fontmap: *mut std::ffi::c_void) -> *mut pango::ffi::PangoContext;
    fn pango_cairo_show_layout(
        cr: *mut cairo::ffi::cairo_t,
        layout: *mut pango::ffi::PangoLayout,
    );
}

/// Render text onto a Cairo context using Pango (handles Nerd Font glyphs).
pub fn pango_show(cr: &cairo::Context, font: &str, size: i32, text: &str) {
    unsafe {
        let ctx_ptr = pango_font_map_create_context(pango_cairo_font_map_get_default());
        let ctx: pango::Context = glib::translate::from_glib_full(ctx_ptr);
        let layout = pango::Layout::new(&ctx);
        let mut desc = pango::FontDescription::from_string(font);
        desc.set_weight(pango::Weight::Bold);
        desc.set_size(size);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        pango_cairo_show_layout(cr.to_raw_none(), layout.to_glib_none().0);
    }
}

/// Measure text width using Pango.
pub fn pango_measure(font: &str, size: i32, text: &str) -> f64 {
    unsafe {
        let ctx_ptr = pango_font_map_create_context(pango_cairo_font_map_get_default());
        let ctx: pango::Context = glib::translate::from_glib_full(ctx_ptr);
        let layout = pango::Layout::new(&ctx);
        let mut desc = pango::FontDescription::from_string(font);
        desc.set_weight(pango::Weight::Bold);
        desc.set_size(size);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        let (rect, _) = layout.pixel_extents();
        rect.width() as f64
    }
}

// =========================================================================
// Cairo Geometry Helpers
// =========================================================================

/// ===== RADIAL MENU =====
/// Draws one annular (donut-wedge) sector with rounded corners on all 4 corners.
pub fn draw_rounded_sector(
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
pub struct BarLayout {
    /// Visible module text (empty modules are skipped).
    pub texts: Vec<String>,
    /// Index into `cfg.modules` for each visible text.
    pub indices: Vec<usize>,
    /// Canvas x of each module's text left edge.
    pub lefts: Vec<f64>,
    /// Canvas width of each module's text.
    pub widths: Vec<f64>,
    /// Pill rectangle in canvas coords.
    pub rx: f64,
    pub ry: f64,
    pub width: f64,
    pub height: f64,
}

/// Measure a module string's rendered width for the pill font/size.
fn measure_pill_text(font: &str, text: &str) -> f64 {
    pango_measure(font, 13 * pango::SCALE, text)
}

/// Compute pill geometry from the current module outputs. `None` when every
/// module is empty (the pill then isn't drawn at all).
pub fn top_bar_layout(
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
pub fn hit_test_pill(layout: &BarLayout, mx: f64, my: f64) -> Option<usize> {
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

pub fn draw_top_bar(cr: &cairo::Context, layout: &BarLayout, cfg: &TopBarConfig) {
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

    // Module text via Pango (handles Nerd Font glyphs correctly).
    let [fg_r, fg_g, fg_b, fg_a] = cfg.foreground;
    cr.set_source_rgba(fg_r, fg_g, fg_b, fg_a);
    let text_y = ry + height / 2.0;
    for (k, t) in layout.texts.iter().enumerate() {
        cr.move_to(layout.lefts[k], text_y);
        pango_show(cr, &cfg.font, 13 * pango::SCALE, t);
    }
}

// =========================================================================
// Unit tests for the top-pill layout and click hit-testing
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_font, BarModule};

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
