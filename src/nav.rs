use std::f64::consts::PI;

// =========================================================================
// Angular Math & Navigation Stack
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
pub fn get_item_angles(
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
pub fn hit_test_index(
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
