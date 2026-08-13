#![allow(deprecated)] // cocoa crate is deprecated in favour of objc2; keep it for now

use gtk4::prelude::GtkWindowExt;
use gtk4::ApplicationWindow;

// =========================================================================
// Overlay window setup, feature-gated per platform.
//
// Linux:  Wayland layer-shell overlay (fullscreen, exclusive keyboard).
// macOS:  GTK4's native Quartz backend — borderless, transparent,
//         topmost, full-screen window approximating the layer-shell role.
// =========================================================================

// Returns the NSWindow backing a GDK surface (public since GTK 4.8).
// gdk-4.0 is already linked by the `gdk4` crate, so no `#[link]` is needed.
#[cfg(target_os = "macos")]
extern "C" {
    fn gdk_macos_surface_get_native_window(
        surface: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
}

#[cfg(target_os = "linux")]
pub fn init_overlay(window: &ApplicationWindow) {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("hyprcircl"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
}

#[cfg(target_os = "macos")]
pub fn init_overlay(window: &ApplicationWindow) {
    window.set_decorated(false);
    // NOTE: we must NOT call `fullscreen()` here. GTK4's macOS backend turns
    // fullscreen into `toggleFullScreen:` (gdkmacostoplevelsurface.c), which
    // switches the window into its own exclusive Space — the opposite of a
    // layer-shell overlay. Instead `setup_macos_overlay` (called once the
    // surface exists) reconfigures the NSWindow directly.
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn init_overlay(_window: &ApplicationWindow) {}

#[cfg(target_os = "macos")]
pub fn setup_macos_overlay(window: &ApplicationWindow) {
    use gtk4::glib::{timeout_add_local, ControlFlow};
    use std::time::Duration;

    let win = window.clone();
    timeout_add_local(Duration::from_millis(50), move || {
        if apply_macos_overlay(&win) {
            ControlFlow::Break
        } else {
            // Surface not realized yet — retry until it exists.
            ControlFlow::Continue
        }
    });
}

#[cfg(target_os = "macos")]
const NS_FLOATING_WINDOW_LEVEL: i64 = 3;

/// Reconfigure the underlying NSWindow so the GTK toplevel behaves like an
/// overlay: borderless + transparent + floating above other windows, present
/// on every Space (including over full-screen apps) WITHOUT ever entering
/// macOS's exclusive full-screen Space. Returns false until the GDK surface
/// (and thus the NSWindow) actually exists.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn apply_macos_overlay(window: &ApplicationWindow) -> bool {
    use cocoa::base::id;
    use gtk4::glib::translate::ToGlibPtr;
    use gtk4::prelude::NativeExt;

    let Some(surface) = window.surface() else {
        return false;
    };
    let ns_window = unsafe {
        let raw: *mut gdk4::ffi::GdkSurface = surface.to_glib_none().0;
        gdk_macos_surface_get_native_window(raw.cast()) as id
    };
    if ns_window.is_null() {
        return false;
    }

    // Size to the display under the cursor when we can, so the overlay lands
    // on the screen the menu is invoked from. Falls back to the display the
    // window currently sits on.
    unsafe { configure_ns_window(ns_window, macos_display_under_cursor()) };
    true
}

/// Configure an overlay NSWindow: floating level, transparent, no shadow,
/// present on every Space (but never entering an exclusive Space), and sized
/// to the given Quartz display bounds if provided.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn configure_ns_window(
    ns_window: cocoa::base::id,
    bounds: Option<(f64, f64, f64, f64)>,
) {
    use cocoa::appkit::{NSWindow, NSWindowCollectionBehavior};
    use cocoa::base::{NO, YES};

    // Overwrite GTK's NSWindowCollectionBehaviorFullScreenPrimary mask:
    // join every Space, show above full-screen apps as an auxiliary window,
    // and don't shift with Spaces. No FullScreenPrimary => no exclusive
    // Space is ever created.
    ns_window.setCollectionBehavior_(
        NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary
            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary,
    );
    ns_window.setLevel_(NS_FLOATING_WINDOW_LEVEL);
    ns_window.setOpaque_(NO);
    ns_window.setHasShadow_(NO);

    match bounds {
        Some(b) => move_overlay_to(ns_window, b),
        None => {
            let mut screen = ns_window.screen();
            if screen.is_null() {
                screen = cocoa::appkit::NSScreen::mainScreen(std::ptr::null_mut());
            }
            if !screen.is_null() {
                let frame = cocoa::appkit::NSScreen::frame(screen);
                ns_window.setFrame_display_(frame, YES);
            }
        }
    }
}

/// Convert a display's Quartz bounds (top-left origin, y-down, the same
/// space CGEventGetLocation uses) into the AppKit screen-frame space (bottom-
/// left origin, y-up) used by NSWindow frames. The flip is exact and global:
/// vertical flipping across the primary display's top edge.
#[cfg(target_os = "macos")]
fn quartz_to_appkit_frame(bounds: (f64, f64, f64, f64)) -> cocoa::foundation::NSRect {
    use cocoa::foundation::{NSPoint, NSRect, NSSize};

    let (x, y, w, h) = bounds;
    let primary_h = core_graphics::display::CGDisplay::main().bounds().size.height;
    NSRect::new(
        NSPoint::new(x, primary_h - (y + h)),
        NSSize::new(w, h),
    )
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn move_overlay_to(ns_window: cocoa::base::id, bounds: (f64, f64, f64, f64)) {
    use cocoa::appkit::NSWindow;
    use cocoa::base::YES;

    ns_window.setFrame_display_(quartz_to_appkit_frame(bounds), YES);
}

/// Current global cursor position in Quartz coordinates (top-left origin,
/// y-down). Uses the CGEventCreate(NULL) trick: a null event's location IS
/// the current mouse position.
#[cfg(target_os = "macos")]
pub fn macos_cursor_point() -> Option<(f64, f64)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let p = event.location();
    Some((p.x, p.y))
}

/// `(x, y, w, h)` of the active display containing the cursor, in Quartz
/// global coordinates. `None` if no display matches (shouldn't happen).
#[cfg(target_os = "macos")]
pub fn macos_display_under_cursor() -> Option<(f64, f64, f64, f64)> {
    use core_graphics::display::CGDisplay;

    let (x, y) = macos_cursor_point()?;
    let ids = CGDisplay::active_displays().ok()?;
    for id in ids {
        let b = CGDisplay::new(id).bounds();
        if x >= b.origin.x
            && x < b.origin.x + b.size.width
            && y >= b.origin.y
            && y < b.origin.y + b.size.height
        {
            return Some((b.origin.x, b.origin.y, b.size.width, b.size.height));
        }
    }
    None
}

/// Move the overlay window so it covers the display with the given Quartz
/// bounds (e.g. the display now under the cursor, on daemon reopen). Also
/// (re)applies the overlay NSWindow configuration. Safe to call right after
/// `present()`: the macOS backend defers the actual on-screen swap until the
/// next frame, so the window never renders at the old location.
#[cfg(target_os = "macos")]
pub fn macos_move_overlay_to(window: &ApplicationWindow, bounds: (f64, f64, f64, f64)) {
    use gtk4::glib::translate::ToGlibPtr;
    use gtk4::prelude::NativeExt;

    if let Some(surface) = window.surface() {
        let ns_window = unsafe {
            let raw: *mut gdk4::ffi::GdkSurface = surface.to_glib_none().0;
            gdk_macos_surface_get_native_window(raw.cast()) as cocoa::base::id
        };
        if !ns_window.is_null() {
            unsafe {
                configure_ns_window(ns_window, Some(bounds));
            }
        }
    }
}

/// Canvas coordinates for the radial menu center such that the circle lands
/// exactly on the physical cursor, given the window's *current* frame. This
/// is robust to the window not covering the display exactly (e.g. the macOS
/// menu-bar strip on top), unlike using display-local cursor coordinates.
#[cfg(target_os = "macos")]
pub fn macos_canvas_center_under_cursor(window: &ApplicationWindow) -> Option<(f64, f64)> {
    use gtk4::glib::translate::ToGlibPtr;
    use gtk4::prelude::NativeExt;

    let (cx, cy) = macos_cursor_point()?;
    let surface = window.surface()?;
    let ns_window = unsafe {
        let raw: *mut gdk4::ffi::GdkSurface = surface.to_glib_none().0;
        gdk_macos_surface_get_native_window(raw.cast()) as cocoa::base::id
    };
    if ns_window.is_null() {
        return None;
    }
    unsafe {
        use cocoa::appkit::NSWindow;
        let frame = NSWindow::frame(ns_window);
        // Window content top-left in Quartz (top-left origin, y-down).
        let primary_h = core_graphics::display::CGDisplay::main().bounds().size.height;
        let canvas_top_quartz = primary_h - (frame.origin.y + frame.size.height);
        Some((cx - frame.origin.x, cy - canvas_top_quartz))
    }
}

#[cfg(target_os = "linux")]
pub fn set_keyboard_exclusive(window: &ApplicationWindow, exclusive: bool) {
    use gtk4_layer_shell::{KeyboardMode, LayerShell};

    window.set_keyboard_mode(if exclusive {
        KeyboardMode::Exclusive
    } else {
        KeyboardMode::None
    });
}

#[cfg(target_os = "macos")]
pub fn set_keyboard_exclusive(_window: &ApplicationWindow, _exclusive: bool) {
    // macOS has no layer-shell keyboard exclusivity. The focused window
    // receives key events, which is sufficient for the port.
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn set_keyboard_exclusive(_window: &ApplicationWindow, _exclusive: bool) {}