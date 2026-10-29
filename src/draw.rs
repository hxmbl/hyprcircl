use std::f64::consts::PI;

use gtk4::cairo::{self};
use gtk4::glib;
use gtk4::glib::translate::ToGlibPtr;
use gtk4::pango;

use crate::config::{RadialConfig, TopBarConfig};
use crate::nav::{get_item_angles, LevelSelection};
use crate::theme::{Rgba, Theme};

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

/// Ink rectangle of rendered text relative to the layout origin:
/// `(x, y, width, height)` in pixels.
pub fn pango_ink_rect(
    font: &str,
    size: i32,
    weight: pango::Weight,
    text: &str,
) -> (f64, f64, f64, f64) {
    unsafe {
        let ctx_ptr = pango_font_map_create_context(pango_cairo_font_map_get_default());
        let ctx: pango::Context = glib::translate::from_glib_full(ctx_ptr);
        let layout = pango::Layout::new(&ctx);
        let mut desc = pango::FontDescription::from_string(font);
        desc.set_weight(weight);
        desc.set_size(size);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        let ink = layout.pixel_extents().0;
        (
            ink.x() as f64,
            ink.y() as f64,
            ink.width() as f64,
            ink.height() as f64,
        )
    }
}

/// Logical (line box) height of a single line of text, in pixels.
pub fn pango_line_height(font: &str, size: i32, weight: pango::Weight, text: &str) -> f64 {
    unsafe {
        let ctx_ptr = pango_font_map_create_context(pango_cairo_font_map_get_default());
        let ctx: pango::Context = glib::translate::from_glib_full(ctx_ptr);
        let layout = pango::Layout::new(&ctx);
        let mut desc = pango::FontDescription::from_string(font);
        desc.set_weight(weight);
        desc.set_size(size);
        layout.set_font_description(Some(&desc));
        layout.set_text(text);
        layout.pixel_size().1 as f64
    }
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
#[cfg(test)]
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
    draw_rounded_sector_gap(cr, cx, cy, r_in, r_out, a1, a2, r_c, 0.0);
}

/// Like `draw_rounded_sector`, but the two straight sides are moved
/// `half_gap` pixels inward, parallel to the radial line at `a1`/`a2`. Two
/// neighbouring wedges sharing a boundary angle are then separated by a gap
/// of constant width (`2 * half_gap`) from the hole to the rim, instead of
/// an angular gap that is thin at the hole and wide at the rim.
pub fn draw_rounded_sector_gap(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    r_in: f64,
    r_out: f64,
    a1: f64,
    a2: f64,
    r_c: f64,
    half_gap: f64,
) {
    let span = a2 - a1;
    if span <= 0.0 || r_out <= r_in {
        return;
    }
    let h = half_gap.max(0.0);

    // Clamp corner radius so it cannot exceed available geometry.
    let max_r = ((r_out - r_in) / 2.0).min(r_out * 0.45);
    let r = r_c.clamp(0.0, max_r);

    // Unit vectors along each side and the inward normals of the two sides.
    let (u1, u2) = ((a1.cos(), a1.sin()), (a2.cos(), a2.sin()));
    let n1 = (-a1.sin(), a1.cos()); // points into the wedge from side a1
    let n2 = (a2.sin(), -a2.cos()); // points into the wedge from side a2

    // Centre of the circle of radius `r` tangent to a side (offset `h`) and
    // to a ring circle: |c| = dist, c·n = h + r.
    let corner = |u: (f64, f64), n: (f64, f64), dist: f64| -> Option<(f64, f64)> {
        let b = h + r;
        let a2v = dist * dist - b * b;
        if a2v <= 0.0 {
            return None;
        }
        let a = a2v.sqrt();
        Some((a * u.0 + b * n.0, a * u.1 + b * n.1))
    };
    let ang = |p: (f64, f64)| p.1.atan2(p.0);
    let unwrap_after = |base: f64, a: f64| {
        // Smallest angle >= base equivalent to `a`.
        let mut v = a;
        while v < base {
            v += 2.0 * PI;
        }
        while v - 2.0 * PI >= base {
            v -= 2.0 * PI;
        }
        v
    };

    let sharp = r <= 0.1;
    let (c_os, c_oe, c_ie, c_is) = if sharp {
        (None, None, None, None)
    } else {
        (
            corner(u1, n1, r_out - r),
            corner(u2, n2, r_out - r),
            corner(u2, n2, r_in + r),
            corner(u1, n1, r_in + r),
        )
    };

    match (c_os, c_oe, c_ie, c_is) {
        (Some(os), Some(oe), Some(ie), Some(is))
            // Corners must not cross each other on either arc.
            if ang_between(ang(os), ang(oe), a1, a2) && ang_between(ang(is), ang(ie), a1, a2) =>
        {
            let t_os = unwrap_after(a1 - PI, ang(os));
            let t_oe = unwrap_after(t_os, ang(oe));
            let t_is = unwrap_after(a1 - PI, ang(is));
            let t_ie = unwrap_after(t_is, ang(ie));
            let pt = |c: (f64, f64)| (cx + c.0, cy + c.1);
            let (os, oe, ie, is) = (pt(os), pt(oe), pt(ie), pt(is));
            cr.new_path();
            cr.arc(cx, cy, r_out, t_os, t_oe);
            cr.arc(oe.0, oe.1, r, t_oe, unwrap_after(t_oe, a2 + PI / 2.0));
            let e3 = unwrap_after(a2 + PI / 2.0, t_ie + PI);
            cr.arc(ie.0, ie.1, r, a2 + PI / 2.0, e3);
            cr.arc_negative(cx, cy, r_in, t_ie, t_is);
            let s5 = t_is + PI;
            cr.arc(is.0, is.1, r, s5, unwrap_after(s5, a1 - PI / 2.0));
            let s6 = a1 - PI / 2.0;
            cr.arc(os.0, os.1, r, s6, unwrap_after(s6, t_os));
            cr.close_path();
        }
        _ => {
            // Sharp corners: intersect the offset sides with both circles.
            let side_angle = |dist: f64| -> Option<f64> {
                (h < dist).then(|| (h / dist).asin())
            };
            let (Some(d_out), Some(d_in)) = (side_angle(r_out), side_angle(r_in.max(h + 0.01)))
            else {
                return;
            };
            if a1 + d_out >= a2 - d_out {
                return;
            }
            let ri = r_in.max(h + 0.01);
            cr.new_path();
            cr.arc(cx, cy, r_out, a1 + d_out, a2 - d_out);
            if a1 + d_in < a2 - d_in {
                cr.arc_negative(cx, cy, ri, a2 - d_in, a1 + d_in);
            } else {
                // Sides meet before the hole: close at their intersection.
                let mid = (a1 + a2) / 2.0;
                let half = span / 2.0;
                let dist = h / half.sin();
                cr.line_to(cx + dist * mid.cos(), cy + dist * mid.sin());
            }
            cr.close_path();
        }
    }
}

/// Whether both corner angles fall inside [a1, a2] in order (start <= end).
fn ang_between(start: f64, end: f64, a1: f64, a2: f64) -> bool {
    let norm = |a: f64| (a - a1).rem_euclid(2.0 * PI);
    let span = a2 - a1;
    let (s, e) = (norm(start), norm(end));
    s <= span && e <= span && s <= e
}

/// Soft shadow ring under the band `r_in..r_out`, fading out over `size`
/// pixels on both edges and shifted down by `offset_y`.
pub fn draw_ring_shadow(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    r_in: f64,
    r_out: f64,
    theme: &Theme,
    alpha: f64,
) {
    let size = theme.shadow_size;
    let (sr, sg, sb, sa) = theme.shadow_color;
    if size <= 0.0 || sa <= 0.0 || alpha <= 0.0 {
        return;
    }
    let cy = cy + theme.shadow_offset_y;
    let outer = r_out + size;
    let inner = (r_in - size).max(0.0);
    let grad = cairo::RadialGradient::new(cx, cy, inner, cx, cy, outer);
    let span = outer - inner;
    let stop = |r: f64| ((r - inner) / span).clamp(0.0, 1.0);
    let a = sa * alpha;
    grad.add_color_stop_rgba(0.0, sr, sg, sb, 0.0);
    grad.add_color_stop_rgba(stop(r_in + size * 0.15), sr, sg, sb, a * 0.6);
    grad.add_color_stop_rgba(stop(r_out - size * 0.15), sr, sg, sb, a);
    grad.add_color_stop_rgba(stop(r_out + size * 0.35), sr, sg, sb, a * 0.35);
    grad.add_color_stop_rgba(1.0, sr, sg, sb, 0.0);
    cr.new_path();
    cr.arc(cx, cy, outer, 0.0, 2.0 * PI);
    cr.new_sub_path();
    cr.arc_negative(cx, cy, inner, 2.0 * PI, 0.0);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();
}

/// ===== WHOLE MENU =====
/// Everything needed to draw one frame of the menu.
pub struct MenuFrame<'a> {
    pub stack: &'a [LevelSelection],
    pub hover: Option<usize>,
    pub outputs: &'a [String],
    pub cx: f64,
    pub cy: f64,
    /// Open animation, 0.0 (just shown) ..= 1.0 (settled).
    pub open: f64,
    /// Expand animation of the deepest ring, 0.0 ..= 1.0.
    pub expand: f64,
}

/// Ease-out cubic.
pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn set_rgba(cr: &cairo::Context, c: Rgba, alpha: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3 * alpha);
}

/// Draw the backdrop, rings, icons, labels and pill for one frame. Shared by
/// the live window and `hyprcircl preview`, so screenshots match reality.
pub fn draw_menu(
    cr: &cairo::Context,
    cfg: &RadialConfig,
    theme: &Theme,
    frame: &MenuFrame,
    width: f64,
    height: f64,
) {
    let stack = frame.stack;
    if stack.is_empty() {
        return;
    }
    let (cx, cy) = (frame.cx, frame.cy);
    let open = ease_out(frame.open);
    let expand = ease_out(frame.expand);

    // Backdrop tint fades in with the menu.
    if theme.backdrop.3 > 0.0 {
        set_rgba(cr, theme.backdrop, open);
        cr.rectangle(0.0, 0.0, width, height);
        let _ = cr.fill();
    }

    // Menu content scales up slightly from the centre while fading in.
    let _ = cr.save();
    let scale = 0.94 + 0.06 * open;
    cr.translate(cx, cy);
    cr.scale(scale, scale);
    cr.translate(-cx, -cy);
    let alpha = open;

    // ===== RINGS =====
    let gap_rad = cfg.gap_rad();
    let half_gap = cfg.item_gap / 2.0;
    let icon_size = (theme.icon_font_size * pango::SCALE as f64) as i32;
    let icon_weight = pango_weight(theme.icon_font_weight);
    let label_size = (theme.label_font_size * pango::SCALE as f64) as i32;
    let label_weight = pango_weight(theme.label_font_weight);
    let show_labels = cfg.show_labels && theme.label_visible;

    let mut current_items = &cfg.items;
    for level in 0..stack.len() {
        let count = current_items.len();
        if count == 0 {
            break;
        }
        let is_active_level = level == stack.len() - 1;
        let (r_in, mut r_out) = cfg.get_level_radii(level);
        // A freshly opened submenu ring grows outward from its inner edge.
        let level_alpha = if is_active_level && level > 0 {
            r_out = r_in + (r_out - r_in) * (0.6 + 0.4 * expand);
            alpha * expand
        } else {
            alpha
        };
        let selected_child = stack[level].selected_child_index;
        let parent_angle = stack[level].parent_mid_angle;

        cr.push_group();
        if level == 0 {
            draw_ring_shadow(cr, cx, cy, r_in, r_out, theme, 1.0);
        } else {
            // Submenus are partial arcs: shadow only behind their wedges.
            let (a_first, _) = get_item_angles(level, 0, count, parent_angle, gap_rad);
            let (_, a_last) = get_item_angles(level, count - 1, count, parent_angle, gap_rad);
            let _ = cr.save();
            cr.new_path();
            cr.move_to(cx, cy);
            let pad = 0.12;
            cr.arc(cx, cy, r_out * 3.0, a_first - pad, a_last + pad);
            cr.close_path();
            cr.clip();
            draw_ring_shadow(cr, cx, cy, r_in, r_out, theme, 0.8);
            let _ = cr.restore();
        }

        for (i, item) in current_items.iter().enumerate() {
            let (a1, a2) = get_item_angles(level, i, count, parent_angle, gap_rad);
            draw_rounded_sector_gap(cr, cx, cy, r_in, r_out, a1, a2, theme.item_corner, half_gap);

            // Parent wedge of an open submenu -> selected; hovered wedge of
            // the active ring -> hover; the active ring -> active; rings
            // already passed through -> default (dimmer).
            let hovered = is_active_level && frame.hover == Some(i);
            let selected = selected_child == Some(i);
            let (fill, stroke, icon_c, label_c) = if selected {
                (
                    theme.item_selected,
                    theme.item_stroke_selected,
                    theme.icon_selected,
                    theme.label_selected,
                )
            } else if hovered {
                (
                    theme.item_hover,
                    theme.item_stroke_hover,
                    theme.icon_hover,
                    theme.label_hover,
                )
            } else if is_active_level {
                (
                    theme.item_active,
                    theme.item_stroke,
                    theme.icon_color,
                    theme.label_color,
                )
            } else {
                (
                    theme.item_default,
                    theme.item_stroke,
                    theme.icon_color,
                    theme.label_color,
                )
            };
            set_rgba(cr, fill, 1.0);
            let _ = cr.fill_preserve();
            if theme.item_stroke_width > 0.0 && stroke.3 > 0.0 {
                set_rgba(cr, stroke, 1.0);
                cr.set_line_width(theme.item_stroke_width);
                let _ = cr.stroke();
            } else {
                cr.new_path();
            }

            // Icon + label, centred as one block on the wedge's mid point.
            let mid_angle = (a1 + a2) / 2.0;
            let mid_r = (r_in + r_out) / 2.0;
            let tx = cx + mid_r * mid_angle.cos();
            let ty = cy + mid_r * mid_angle.sin();
            let (ix, iy, iw, ih) =
                pango_ink_rect(&theme.icon_font, icon_size, icon_weight, &item.icon);

            // Room across the wedge at the label's radius (minus padding).
            let chord = 2.0 * mid_r * ((a2 - a1) / 2.0).sin() - cfg.item_gap - 10.0;
            let label = if show_labels && !item.label.is_empty() {
                let lw = pango_measure(&theme.label_font, label_size, label_weight, &item.label);
                let lh =
                    pango_line_height(&theme.label_font, label_size, label_weight, &item.label);
                (lw <= chord).then_some((lw, lh))
            } else {
                None
            };
            let block_h = match label {
                Some((_, lh)) => ih + theme.label_gap + lh,
                None => ih,
            };
            let top = ty - block_h / 2.0;

            set_rgba(cr, icon_c, 1.0);
            cr.move_to(tx - ix - iw / 2.0, top - iy);
            pango_show(cr, &theme.icon_font, icon_size, icon_weight, &item.icon);

            if let Some((lw, _)) = label {
                set_rgba(cr, label_c, 1.0);
                cr.move_to(tx - lw / 2.0, top + ih + theme.label_gap);
                pango_show(cr, &theme.label_font, label_size, label_weight, &item.label);
            }
        }
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(level_alpha);

        // Descend to the open child level for the next iteration.
        match selected_child {
            Some(child_idx) if child_idx < current_items.len() => {
                current_items = &current_items[child_idx].items;
            }
            _ => break,
        }
    }

    // ===== TOP PILL (drawn last so ring shadows never darken it) =====
    // Only at the root menu (nav stack depth == 1).
    if cfg.top_bar.enabled && stack.len() == 1 {
        if let Some(layout) =
            top_bar_layout(&cfg.top_bar, theme, cfg.outer_radius, frame.outputs, cx, cy)
        {
            cr.push_group();
            draw_top_bar(cr, &layout, theme);
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(alpha);
        }
    }

    let _ = cr.restore();
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

/// Cut `text` to at most `max` characters, ending in "…" when it was longer.
/// Counts chars (not bytes), so multi-byte titles never split mid-glyph.
/// `max == 0` means no limit.
pub fn truncate_chars(text: &str, max: usize) -> String {
    if max == 0 || text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
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
        let out = truncate_chars(
            outputs.get(i).map(|o| o.trim()).unwrap_or_default(),
            m.max_length,
        );
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
    let _ = cr.fill_preserve();
    if theme.pill_stroke_width > 0.0 && theme.pill_stroke.3 > 0.0 {
        let (r_, g_, b_, a_) = theme.pill_stroke;
        cr.set_source_rgba(r_, g_, b_, a_);
        cr.set_line_width(theme.pill_stroke_width);
        let _ = cr.stroke();
    } else {
        cr.new_path();
    }

    // Module text via Pango (handles Nerd Font glyphs correctly).
    let (fg_r, fg_g, fg_b, fg_a) = theme.pill_fg;
    let size = (theme.pill_font_size * pango::SCALE as f64) as i32;
    let weight = pango_weight(theme.pill_font_weight);

    // Hairline separators centred in each gap: short (40% of the pill) and
    // faint, so modules read as groups without boxing them in.
    let (sr, sg, sb, sa) = theme.pill_separator;
    if sa > 0.0 && layout.texts.len() > 1 {
        let _ = cr.save();
        cr.set_source_rgba(sr, sg, sb, sa);
        cr.set_line_width(1.0);
        let h = (height * 0.4).round();
        let y0 = ry + (height - h) / 2.0;
        let ends = layout.lefts.iter().zip(&layout.widths).map(|(l, w)| l + w);
        for (end, next) in ends.zip(layout.lefts.iter().skip(1)) {
            let x = ((end + next) / 2.0).round() + 0.5;
            cr.move_to(x, y0);
            cr.line_to(x, y0 + h);
        }
        let _ = cr.stroke();
        let _ = cr.restore();
    }

    cr.set_source_rgba(fg_r, fg_g, fg_b, fg_a);
    for (k, t) in layout.texts.iter().enumerate() {
        // Pango draws from the layout's top-left; centre the line box (not
        // the ink, so glyphs with different ink heights share a baseline).
        let line_h = pango_line_height(&theme.pill_font, size, weight, t);
        cr.move_to(layout.lefts[k], ry + (height - line_h) / 2.0);
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
    fn truncate_chars_counts_characters() {
        assert_eq!(truncate_chars("short", 0), "short");
        assert_eq!(truncate_chars("short", 5), "short");
        assert_eq!(truncate_chars("nvim draw.rs", 6), "nvim…");
        // Multi-byte text is cut on character boundaries.
        assert_eq!(truncate_chars("ééééé", 3), "éé…");
    }

    #[test]
    fn max_length_truncates_module_output() {
        let mut cfg = test_config();
        cfg.modules[0].max_length = 6;
        let outputs = vec!["a long window title\n".to_string(), "2".to_string()];
        let layout =
            top_bar_layout(&cfg, &Theme::default(), 100.0, &outputs, 500.0, 500.0).expect("layout");
        assert_eq!(
            layout.texts,
            vec!["A a lon…".to_string(), "B 2".to_string()]
        );
    }

    #[test]
    fn draw_top_bar_with_separators_renders() {
        use gtk4::cairo::{Context, Format, ImageSurface};

        let surface = ImageSurface::create(Format::ARgb32, 1000, 1000).expect("image surface");
        let cr = Context::new(&surface).expect("context");
        let outputs = vec!["1".to_string(), "2".to_string()];
        let theme = Theme::default();
        let layout =
            top_bar_layout(&test_config(), &theme, 100.0, &outputs, 500.0, 500.0).expect("layout");
        draw_top_bar(&cr, &layout, &theme);
        assert!(theme.pill_separator.3 > 0.0);
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
