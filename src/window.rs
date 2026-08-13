use gtk4::prelude::{GtkWindowExt, WidgetExt};
use gtk4::ApplicationWindow;

// =========================================================================
// Overlay window setup, feature-gated per platform.
//
// Linux:  Wayland layer-shell overlay (fullscreen, exclusive keyboard).
// macOS:  GTK4's native Quartz backend — borderless, transparent,
//         topmost, full-screen window approximating the layer-shell role.
// =========================================================================

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
    window.fullscreen();
    // The CSS provider already paints the window background transparent, so
    // the full-screen surface lets the desktop and other windows show through
    // around the radial menu.
    window.set_opacity(1.0);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn init_overlay(_window: &ApplicationWindow) {}

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