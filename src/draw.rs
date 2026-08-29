use std::f64::consts::PI;

use gtk4::cairo::{self};
use gtk4::glib;
use gtk4::glib::translate::ToGlibPtr;
use gtk4::pango;

use crate::config::TopBarConfig;
use crate::theme::Theme;

// pangocairo FFI — symbols are already linked through gtk4
extern "C" {
    fn pango_cairo_font_map_get_default() -> *mut std::ffi::c_void;
    fn pango_font_map_create_context(
        fontmap: *mut std::ffi::c_void,
    ) -> *mut pango::ffi::PangoContext;
    fn pango_cairo_show_layout(cr: *mut cairo::ffi::cairo_t, layout: *mut pango::ffi::PangoLayout);
}

/// Render text onto a Cairo context using Pango (handles Nerd Font glyphs).
pub fn pango_show(cr: &cairo::Context, font: &str, size: i32, weight: pango::Weight, text: &str) {
    unsafe {
        let ctx_ptr = pango_font_map_create_context(pango_cairo_font_map_get_default());
        let ctx: pango::Context = glib::translate::from_glib_full(ctx_ptr);
        let layout = pango::Layout::new(&ctx);
        let mut desc = pango::FontDescription::from_string(font);
        desc.set_weight(weight);
        desc.set_size(size);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        pango_cairo_show_layout(cr.to_raw_none(), layout.to_glib_none().0);
    }
}

/// Measure rendered text extents using Pango: `(width, height)` in pixels.
pub fn pango_extents(font: &str, size: i32, weight: pango::Weight, text: &str) -> (f64, f64) {
    unsafe {
        let ctx_ptr = pango_font_map_create_context(pango_cairo_font_map_get_default());
        let ctx: pango::Context = glib::translate::from_glib_full(ctx_ptr);
        let layout = pango::Layout::new(&ctx);
        let mut desc = pango::FontDescription::from_string(font);
        desc.set_weight(weight);
        desc.set_size(size);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        let rect = layout.pixel_extents().0;
        (rect.width() as f64, rect.height() as f64)
    }
}

/// Measure text width using Pango.
pub fn pango_measure(font: &str, size: i32, weight: pango::Weight, text: &str) -> f64 {
    pango_extents(font, size, weight, text).0
}

/// Map a CSS numeric font-weight (100..900) onto a Pango weight.
pub fn pango_weight(w: i32) -> pango::Weight {
    match w {
        100 => pango::Weight::Thin,
        200 => pango::Weight::Ultralight,
        300 => pango::Weight::Light,
        400 => pango::Weight::Normal,
        500 => pango::Weight::Medium,
        600 => pango::Weight::Semibold,
        800 => pango::Weight::Ultrabold,
        900 => pango::Weight::Heavy,
        _ => pango::Weight::Bold,
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
    let theta_ie = a2 - delta_in; // Inner End
    let theta_is = a1 + delta_in; // Inner Start

    // Calculated center coordinates for each corner arc
    let c_os = (
        cx + (r_out - r) * theta_os.cos(),
        cy + (r_out - r) * theta_os.sin(),
    );
    let c_oe = (
        cx + (r_out - r) * theta_oe.cos(),
        cy + (r_out - r) * theta_oe.sin(),
    );
    let c_ie = (
        cx + (r_in + r) * theta_ie.cos(),
        cy + (r_in + r) * theta_ie.sin(),
    );
    let c_is = (
        cx + (r_in + r) * theta_is.cos(),
        cy + (r_in + r) * theta_is.sin(),
    );

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
fn measure_pill_text(theme: &Theme, text: &str) -> f64 {
    let size = (theme.pill_font_size * pango::SCALE as f64) as i32;
    pango_measure(
        &theme.pill_font,
        size,
        pango_weight(theme.pill_font_weight),
        text,
    )
}

/// Compute pill geometry from the current module outputs. `None` when every
/// module is empty (the pill then isn't drawn at all).
pub fn top_bar_layout(
    cfg: &TopBarConfig,
    theme: &Theme,
    r_out: f64,
    outputs: &[String],
    cx: f64,
    cy: f64,
) -> Option<BarLayout> {
    let height = theme.pill_height;
    let gap = theme.pill_gap; // spacing between modules

    let mut texts = Vec::new();
    let mut indices = Vec::new();
    for (i, m) in cfg.modules.iter().enumerate() {
        let out = outputs.get(i).cloned().unwrap_or_default();
        let has_placeholder = m.format.contains("{output}");
        let body = m.format.replace("{output}", &out);
        let text = if m.icon.is_empty() {
            body
        } else {
            format!("{} {}", m.icon, body)
        };
        // Placeholder formats render whenever their text is non-empty.
        // Literal formats (e.g. `ICON_UPDATE` status glyphs) only render
        // while the command actually produces output.
        let visible = if has_placeholder {
            !text.trim().is_empty()
        } else {
            !out.trim().is_empty()
        };
        if visible {
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
        let w = measure_pill_text(theme, t);
        widths.push(w);
        total_w += w;
        if j + 1 < texts.len() {
            total_w += gap;
        }
    }
    let width = total_w + theme.pill_padding_x * 2.0;
    let rx = cx - width / 2.0;
    let ry = cy - r_out - theme.pill_offset_y - height;
    // Not enough headroom above the ring: skip the pill entirely rather than
    // clipping it offscreen while its hit-test zone stays clickable.
    if ry < 0.0 {
        return None;
    }

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

pub fn draw_top_bar(cr: &cairo::Context, layout: &BarLayout, theme: &Theme) {
    let rx = layout.rx;
    let ry = layout.ry;
    let width = layout.width;
    let height = layout.height;
    let r = theme.pill_corner.min(height / 2.0);

    // Background pill
    cr.new_sub_path();
    cr.arc(rx + width - r, ry + r, r, -PI / 2.0, 0.0);
    cr.arc(rx + width - r, ry + height - r, r, 0.0, PI / 2.0);
    cr.arc(rx + r, ry + height - r, r, PI / 2.0, PI);
    cr.arc(rx + r, ry + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();

    let (bg_r, bg_g, bg_b, bg_a) = theme.pill_bg;
    cr.set_source_rgba(bg_r, bg_g, bg_b, bg_a);
    let _ = cr.fill();

    // Module text via Pango (handles Nerd Font glyphs correctly).
    let (fg_r, fg_g, fg_b, fg_a) = theme.pill_fg;
    cr.set_source_rgba(fg_r, fg_g, fg_b, fg_a);
    let text_y = ry + height / 2.0;
    let size = (theme.pill_font_size * pango::SCALE as f64) as i32;
    let weight = pango_weight(theme.pill_font_weight);
    for (k, t) in layout.texts.iter().enumerate() {
        cr.move_to(layout.lefts[k], text_y);
        pango_show(cr, &theme.pill_font, size, weight, t);
    }
}

// =========================================================================
// Unit tests for the top-pill layout and click hit-testing
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BarModule;
    use crate::theme::Theme;

    fn test_config() -> TopBarConfig {
        TopBarConfig {
            enabled: true,
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
        let layout =
            top_bar_layout(&cfg, &Theme::default(), 100.0, &outputs, 500.0, 500.0).expect("layout");

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
    fn pango_measure_returns_positive_width() {
        let w = pango_measure(
            &Theme::default().pill_font,
            13 * pango::SCALE,
            pango::Weight::Bold,
            "Terminal",
        );
        assert!(w > 0.0, "measured text width must be positive");
        // Wider text measures wider.
        let w2 = pango_measure(
            &Theme::default().pill_font,
            13 * pango::SCALE,
            pango::Weight::Bold,
            "Terminal Terminal",
        );
        assert!(w2 > w);
    }

    #[test]
    fn draw_rounded_sector_builds_path() {
        use gtk4::cairo::{Context, Format, ImageSurface};

        let surface = ImageSurface::create(Format::ARgb32, 400, 400).expect("image surface");
        let cr = Context::new(&surface).expect("context");

        // A normal wedge produces a non-empty path.
        draw_rounded_sector(&cr, 200.0, 200.0, 40.0, 110.0, 0.0, PI / 2.0, 6.0);
        assert!(cr.has_current_point().unwrap());

        // A degenerate (zero/negative span) wedge issues no path commands.
        cr.new_path();
        draw_rounded_sector(&cr, 200.0, 200.0, 40.0, 110.0, 1.0, 1.0, 6.0);
        assert!(!cr.has_current_point().unwrap());
    }

    #[test]
    fn draw_rounded_sector_clamps_corner_radius() {
        use gtk4::cairo::{Context, Format, ImageSurface};

        let surface = ImageSurface::create(Format::ARgb32, 400, 400).expect("image surface");
        let cr = Context::new(&surface).expect("context");

        // A huge requested corner radius on a thin ring must not panic and
        // still produce a path (radius is clamped internally).
        draw_rounded_sector(&cr, 200.0, 200.0, 100.0, 110.0, 0.0, PI, 999.0);
        assert!(cr.has_current_point().unwrap());
    }

    #[test]
    fn top_bar_layout_centers_on_cursor() {
        let cfg = test_config();
        let layout = top_bar_layout(
            &cfg,
            &Theme::default(),
            100.0,
            &["x".to_string(), "y".to_string()],
            500.0,
            500.0,
        )
        .expect("layout");
        // Pill is horizontally centered on the cursor x.
        assert!((layout.rx + layout.width / 2.0 - 500.0).abs() < 1e-6);
        // Pill sits above the menu (above outer radius + offset).
        assert!(layout.ry < 500.0 - 100.0);
    }

    #[test]
    fn top_bar_layout_all_empty_modules_is_none() {
        let mut cfg = test_config();
        // Strip icons from every module and feed empty outputs so no module
        // produces visible text -> the pill is not drawn at all.
        for m in cfg.modules.iter_mut() {
            m.icon = String::new();
        }
        let outputs = vec![String::new(); cfg.modules.len()];
        assert!(top_bar_layout(&cfg, &Theme::default(), 100.0, &outputs, 500.0, 500.0).is_none());
    }

    #[test]
    fn pango_show_renders_without_panic() {
        use gtk4::cairo::{Context, Format, ImageSurface};

        let surface = ImageSurface::create(Format::ARgb32, 200, 60).expect("image surface");
        let cr = Context::new(&surface).expect("context");
        // Smoke test: drawing text onto a headless surface must not panic.
        let t = Theme::default();
        pango_show(
            &cr,
            &t.pill_font,
            13 * pango::SCALE,
            pango::Weight::Bold,
            "Terminal",
        );
    }

    #[test]
    fn draw_top_bar_renders_without_panic() {
        use gtk4::cairo::{Context, Format, ImageSurface};

        let cfg = test_config();
        let layout = top_bar_layout(
            &cfg,
            &Theme::default(),
            100.0,
            &["1".to_string(), "2".to_string()],
            500.0,
            500.0,
        )
        .expect("layout");
        let surface = ImageSurface::create(Format::ARgb32, 800, 800).expect("image surface");
        let cr = Context::new(&surface).expect("context");
        // Smoke test: pill background + module text must render without panic.
        draw_top_bar(&cr, &layout, &Theme::default());
    }

    #[test]
    fn pill_layout_skips_empty_modules() {
        let mut cfg = test_config();
        // Icon-less module whose output is empty renders nothing -> skipped.
        cfg.modules[0].icon = String::new();
        let outputs = vec![String::new(), "2".to_string()];
        let layout =
            top_bar_layout(&cfg, &Theme::default(), 100.0, &outputs, 500.0, 500.0).expect("layout");
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

    #[test]
    fn literal_format_renders_only_while_command_produces_output() {
        let cfg = TopBarConfig {
            enabled: true,
            modules: vec![BarModule {
                format: "ICON_UPDATE".into(),
                ..Default::default()
            }],
        };
        // No output -> literal glyph hidden entirely (no dead "ICON_UPDATE" pill).
        assert!(top_bar_layout(
            &cfg,
            &Theme::default(),
            100.0,
            &["".to_string()],
            500.0,
            500.0
        )
        .is_none());
        // Output present -> literal glyph shown as-is.
        let layout = top_bar_layout(
            &cfg,
            &Theme::default(),
            100.0,
            &["yes".to_string()],
            500.0,
            500.0,
        )
        .expect("layout");
        assert_eq!(layout.texts, vec!["ICON_UPDATE".to_string()]);
        assert_eq!(layout.indices, vec![0]);
    }

    #[test]
    fn pill_hidden_when_it_does_not_fit_above_the_ring() {
        let cfg = test_config();
        let outputs = vec!["1".to_string(), "2".to_string()];
        // Cursor near the very top of the canvas: the pill would clip offscreen
        // and must not be laid out at all (no invisible clickable zone).
        assert!(top_bar_layout(&cfg, &Theme::default(), 110.0, &outputs, 400.0, 40.0).is_none());
    }
}
