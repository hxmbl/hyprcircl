# hyprcircl

A radial (pie-menu) launcher for [Hyprland](https://hyprland.org) and macOS,
built with GTK4. `hyprcircl` opens a concentric ring menu under the cursor with
a Waybar-style status pill on top, then runs the selected command — ideal for a
single keybinding that summons your whole app/window/system menu.

- **Radial menu** — nested submenus laid out as concentric rings centered on the
  pointer; the hovered wedge is highlighted.
- **Top pill** — a live status bar (clock, battery, volume, network, etc.)
  rendered Waybar-style above the menu, with click / scroll applets.
- **Live theming** — visual style is driven by a CSS file that reloads on save,
  no daemon restart required.
- **Live config** — the TOML config is watched and reloaded automatically.
- **Single-instance toggle** — a Unix socket lets repeated launches toggle the
  menu instead of spawning duplicates (so one keybinding opens and closes it).

## Platforms

| Platform | Backend | Notes |
| -------- | ------- | ----- |
| Linux (Wayland / Hyprland) | `gtk4-layer-shell` | Surface is placed on a layer-shell overlay covering the output. |
| macOS | GTK4 + Cocoa | Overlay window follows the display under the cursor. |

Windows is not supported.

## Dependencies

### Linux

```sh
# Debian / Ubuntu 24.04 — gtk4-layer-shell is not packaged, so it is built from source.
sudo apt-get install -y --no-install-recommends \
  libgtk-4-dev libwayland-dev wayland-protocols meson ninja-build pkg-config
```

`gtk4-layer-shell` is built and installed from source in CI (see
`.github/actions/setup-linux-layer-shell`). You can do the same locally:

```sh
git clone --depth 1 --branch v1.3.0 https://github.com/wmww/gtk4-layer-shell.git /tmp/gtk4-layer-shell
cd /tmp/gtk4-layer-shell
meson setup build -Dexamples=false -Ddocs=false -Dtests=false -Dintrospection=false -Dvapi=false
ninja -C build
sudo ninja -C build install
sudo ldconfig
```

### macOS

```sh
brew install gtk4
```

## Build & install

```sh
cargo build --release
# Optional: copy the binary somewhere on your PATH
install -Dm755 target/release/hyprcircl ~/.local/bin/hyprcircl
```

Run the test suite (Linux + macOS in CI):

```sh
cargo test
```

Lint/format gates (CI denies warnings):

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

## Binding a key (Hyprland)

`hyprcircl` is a single-instance daemon: the first launch starts it, and every
subsequent launch sends a `toggle` over its socket to show/hide the menu. Bind a
key in your `hyprland.conf`:

```ini
bind = $mainMod, Space, exec, hyprcircl
```

Launching `hyprcircl` with no running instance starts the persistent daemon and
shows the menu; launching it again toggles visibility.

## Configuration

Config is loaded from the first existing path, in this priority order:

1. `$XDG_CONFIG_HOME/hyprcircl/hyprcircl.toml`
2. `$XDG_CONFIG_HOME/hyprcircl/config.toml` (legacy compat fallback)
3. `$HOME/.config/hyprcircl/hyprcircl.toml`
4. `$HOME/.config/hyprcircl/config.toml`
5. `hyprcircl.toml` in the current directory
6. `config.toml` in the current directory

If no file is found, built-in defaults are used (see `RadialConfig::default` in
`src/config.rs`). The TOML file is watched and reloaded on change — no restart
needed.

A complete reference config ships in [`hyprcircl.toml`](./hyprcircl.toml) at the
repo root.

### Radial menu geometry

Top-level keys:

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `inner_radius` | float | `40.0` | Radius of the menu's empty center hole. |
| `outer_radius` | float | `110.0` | Outer edge of the root ring. |
| `ring_thickness` | float | `60.0` | Thickness of each outer submenu ring. |
| `ring_gap` | float | `8.0` | Gap between consecutive rings. |
| `item_gap_degrees` | float | `2.0` | Angular gap between wedges. |
| `notify_only` | bool | `false` | If true, clicking an action only sends a notification instead of running it (safety mode). |
| `show_labels` | bool | `true` | Show text labels under icons. |
| `items` | array | — | The menu tree (see below). |
| `top_bar` | table | — | The status pill (see below). |

### Menu items

Each `[[items]]` entry is a `MenuItem`:

| Key | Type | Description |
| --- | ---- | ----------- |
| `label` | string | Text shown under the icon. |
| `icon` | string | Glyph shown in the wedge (typically a Nerd Font icon). |
| `command` | array of strings | Executed on click. Empty for submenu parents. |
| `items` | array of `MenuItem` | Child items; a non-empty list makes this a submenu. |

Nesting is recursive — a child can itself contain `items` for deeper submenus.
Submenus open as additional concentric rings outward from the root.

```toml
[[items]]
label = "Media"
icon = "󰝚"

  [[items.items]]
  label = "Play/Pause"
  icon = "󰐊"
  command = ["playerctl", "play-pause"]
```

### Top pill (`[top_bar]`)

```toml
[top_bar]
enabled = true
```

Each `[[top_bar.modules]]` is a Waybar-style module:

| Key | Type | Description |
| --- | ---- | ----------- |
| `command` | string | Shell command whose stdout becomes the module text. |
| `interval` | int | Refresh interval in seconds. |
| `format` | string | `{output}` is replaced with the command output; `ICON_UPDATE` and similar are special-cased. |
| `icon` | string | Optional leading glyph. |
| `stream_command` | string | Long-lived push command; each stdout line updates the module (no interval polling). Used for the Hyprland event socket, `pactl subscribe`, etc. |
| `watch` | array of strings | File paths (e.g. sysfs nodes) whose changes re-run `command` immediately. |
| `on_click` | string | Shell command on left click. |
| `on_click_right` | string | Shell command on right click. |
| `on_scroll_up` | string | Shell command on scroll up. |
| `on_scroll_down` | string | Shell command on scroll down. |

## Theming (CSS)

Visual styling lives in a `hyprcircl.css` file placed next to your
`hyprcircl.toml` (e.g. `~/.config/hyprcircl/hyprcircl.css`). It is watched and
reloaded live. A reference theme ships in [`hyprcircl.css`](./hyprcircl.css).

Supported selectors:

- `.item` — wedge fill (use `border-radius` to round corners)
- `.item:hover` — wedge under the pointer
- `.item:active` — open ring (non-hovered wedges)
- `.item:selected` — parent wedge leading into an open submenu
- `.item-stroke` — wedge outline (`border-color`, `border-width`)
- `.icon` — the Nerd Font glyph (`color`, `font-size`, `font-family`, `font-weight`)
- `.label` — the text label (`display: none;` hides labels)
- `.pill` — the status pill (`background-color`, `color`, `height`, `padding`,
  `margin-top`, `border-radius`, `font-*`, and `--module-gap` for spacing)

Colors accept `#rgb` / `#rrggbb` / `#rrggbbaa`, `rgb()` / `rgba()`,
`hsl()` / `hsla()`, or CSS color names. Sizes accept plain numbers, `px`, or
`pt`.

## Interaction

- **Show / hide** — run `hyprcircl` (or your keybinding) to toggle.
- **Click** — activate the hovered wedge; submenus open as new rings.
- **Mouse wheel over a pill applet** — runs that module's `on_scroll_up` /
  `on_scroll_down` (e.g. volume), otherwise switches workspaces.
- **Esc** — pop one submenu level; press again at the root to close.
- **Page Up / Page Down** — next / previous Hyprland workspace.
- **Left/right click & scroll on pill modules** — module `on_click*` actions.

## Project layout

| Path | Purpose |
| ---- | ------- |
| `src/main.rs` | Application entry, window controller, input handling, single-instance socket. |
| `src/config.rs` | TOML config model, load/reload, and radii math. |
| `src/theme.rs` | CSS theme loading and live watching. |
| `src/draw.rs` | Ring/wedge and pill rendering, hit-testing. |
| `src/nav.rs` | Menu tree angle layout and hit-testing. |
| `src/window.rs` | Platform-specific overlay window setup. |
| `src/cursor.rs` | Cursor position in display-local coordinates (Linux). |
| `src/process.rs` | Shell command execution and process-group management. |
| `.github/` | CI workflow (`lint` + `test`) and the Linux layer-shell setup action. |

## License
no.
