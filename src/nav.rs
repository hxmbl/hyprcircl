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

// =========================================================================
// Unit tests for angular math and navigation hit-testing
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_angles_span_full_circle() {
        let count = 4;
        let gap = 0.0f64;
        let mut prev_end: Option<f64> = None;
        for i in 0..count {
            let (a1, a2) = get_item_angles(0, i, count, 0.0, gap);
            assert!(a2 > a1, "wedge {i} must be forward-facing");
            if let Some(p) = prev_end {
                assert!((a1 - p).abs() < 1e-9, "root wedges must be contiguous");
            }
            prev_end = Some(a2);
        }
        // Four quarters cover all 360° (no gaps when gap == 0).
        let total = prev_end.unwrap() - get_item_angles(0, 0, count, 0.0, gap).0;
        assert!((total - 2.0 * PI).abs() < 1e-9);
    }

    #[test]
    fn root_angles_are_centered_at_top() {
        // Item 0 should start at -90° (top), per the -PI/2 anchor.
        let (a1, _a2) = get_item_angles(0, 0, 4, 0.0, 0.0);
        assert!((a1 - (-PI / 2.0)).abs() < 1e-9);
    }

    #[test]
    fn gap_shrinks_each_wedge() {
        let no_gap = get_item_angles(0, 0, 4, 0.0, 0.0);
        let with_gap = get_item_angles(0, 0, 4, 0.0, 0.2);
        assert!(with_gap.1 - with_gap.0 < no_gap.1 - no_gap.0);
    }

    #[test]
    fn submenu_angles_fan_around_parent() {
        let count = 3;
        let parent = 1.0f64; // arbitrary mid-angle
        let (a1, _a2) = get_item_angles(1, 0, count, parent, 0.0);
        let (b1, _b2) = get_item_angles(1, 1, count, parent, 0.0);
        let (c1, c2) = get_item_angles(1, 2, count, parent, 0.0);
        // All wedges sit around the parent mid-angle (-PI*0.375 .. +PI*0.375).
        assert!(a1 < parent && c2 > parent);
        assert!(b1 > a1 && c1 > b1);
    }

    #[test]
    fn angle_in_slice_wrap_around() {
        // A slice crossing the 0/2π boundary (e.g. 350°..10°).
        let two_pi = 2.0 * PI;
        let a1 = (350.0f64).to_radians();
        let a2 = a1 + (20.0f64).to_radians();
        assert!(angle_in_slice(0.0, a1, a2));
        assert!(angle_in_slice(two_pi - 0.01, a1, a2));
        assert!(!angle_in_slice(PI, a1, a2));
    }

    #[test]
    fn angle_in_slice_endpoint_inclusive() {
        let (a1, a2) = get_item_angles(0, 1, 4, 0.0, 0.0);
        assert!(angle_in_slice(a1, a1, a2));
        assert!(angle_in_slice(a2, a1, a2));
    }

    #[test]
    fn hit_test_index_finds_correct_wedge() {
        let count = 4;
        for i in 0..count {
            let (a1, a2) = get_item_angles(0, i, count, 0.0, 0.0);
            let mid = (a1 + a2) / 2.0;
            assert_eq!(hit_test_index(0, mid, count, 0.0, 0.0), Some(i));
        }
    }

    #[test]
    fn hit_test_index_empty_level() {
        assert_eq!(hit_test_index(0, 0.0, 0, 0.0, 0.0), None);
    }

    #[test]
    fn hit_test_index_gap_returns_none() {
        // An angle in the gap between two wedges hits nothing when gaps are wide.
        let count = 4;
        let gap = 0.5; // large gap
                       // Probe the midpoint of the gap between item 0 and item 1.
        let (_, a0_end) = get_item_angles(0, 0, count, 0.0, gap);
        let (a1_start, _) = get_item_angles(0, 1, count, 0.0, gap);
        let in_gap = (a0_end + a1_start) / 2.0;
        assert_eq!(hit_test_index(0, in_gap, count, 0.0, gap), None);
    }
}
