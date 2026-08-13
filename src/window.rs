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

/// Reconfigure the underlying NSWindow so the GTK toplevel behaves like an
/// overlay: borderless + transparent + floating above other windows, present
/// on every Space (including over full-screen apps) WITHOUT ever entering
/// macOS's exclusive full-screen Space. Returns false until the GDK surface
/// (and thus the NSWindow) actually exists.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn apply_macos_overlay(window: &ApplicationWindow) -> bool {
    use cocoa::appkit::{NSScreen, NSWindow, NSWindowCollectionBehavior};
    use cocoa::base::{id, NO, YES};
    use gtk4::glib::translate::ToGlibPtr;
    use gtk4::prelude::NativeExt;

    const NS_FLOATING_WINDOW_LEVEL: i64 = 3;

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

    unsafe {
        // Overwrite GTK's NSWindowCollectionBehaviorFullScreenPrimary mask:
        // join every Space, show above full-screen apps as an auxiliary
        // window, and don't shift with Spaces. No FullScreenPrimary => no
        // exclusive Space is ever created.
        ns_window.setCollectionBehavior_(
            NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary,
        );
        ns_window.setLevel_(NS_FLOATING_WINDOW_LEVEL);
        ns_window.setOpaque_(NO);
        ns_window.setHasShadow_(NO);

        // Size to the screen the window sits on (the frame is in the global
        // bottom-left-origin coordinate space, so it covers the display fully
        // including the menu-bar strip).
        let mut screen = ns_window.screen();
        if screen.is_null() {
            screen = NSScreen::mainScreen(std::ptr::null_mut());
        }
        if !screen.is_null() {
            let frame = NSScreen::frame(screen);
            ns_window.setFrame_display_(frame, YES);
        }
    }
    true
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