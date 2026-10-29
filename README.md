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

### Quick start

```sh
hyprcircl init          # writes ~/.config/hyprcircl/{hyprcircl.toml,hyprcircl.css}
hyprcircl config path   # print which config file is active
```

### CLI

| Command | Description |
| ------- | ----------- |
| *(none)* / `toggle` | Toggle the menu (default) |
| `show` | Show the menu |
| `hide` | Hide the menu |
| `quit` | Stop the background daemon |
| `init` | Create starter config + theme (`--force` to overwrite) |
| `config path` | Print active config path |

Options: `-c` / `--config PATH` (or `HYPRCIRCL_CONFIG`), `--help`, `--version`.

## Configuration

Config is loaded from the first existing path, in this priority order:

1. `$XDG_CONFIG_HOME/hyprcircl/hyprcircl.toml`
2. `$XDG_CONFIG_HOME/hyprcircl/config.toml` (legacy compat fallback)
3. `$HOME/.config/hyprcircl/hyprcircl.toml`
4. `$HOME/.config/hyprcircl/config.toml`
5. `hyprcircl.toml` in the current directory
6. `config.toml` in the current directory

If no file is found, the shipped [`hyprcircl.toml`](./hyprcircl.toml) is the
built-in default (it is compiled in). Every key is optional: a file that leaves
a key, `[bar]` or `[[items]]` out gets the shipped value. The file is watched
and reloaded on change, with no restart needed. Unknown keys are reported on
stderr as `[CONFIG] ... unknown key` and ignored.

The shipped file is the full reference, commented key by key. Colours, fonts
and the pill's size live in [`hyprcircl.css`](#theming-css), not the TOML.

### `[menu]`: ring geometry and behaviour

Sizes are logical pixels, so they scale with the monitor's scale factor.

| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `inner_radius` | float | `60` | Radius of the empty hole in the middle. |
| `outer_radius` | float | `156` | Outer edge of the main ring. |
| `submenu_width` | float | `76` | Width of each submenu ring that opens outside it. |
| `ring_gap` | float | `8` | Space between rings. |
| `item_gap` | float | `6` | Space between wedges, the same width from hole to rim. |
| `animation_ms` | int | `160` | Open/expand animation; `0` turns it off. |
| `labels` | bool | `true` | Text under icons. |
| `keyboard` | bool | `true` | ←/→ or `h`/`l`, `1`–`9`, Enter, Backspace, Esc. |
| `notify_only` | bool | `false` | Show a notification instead of running the command (dry run). |

Schema 1 configs (flat top-level keys such as `inner_radius`, `ring_thickness`,
`item_gap_degrees`, and `[top_bar]`) still load. A config that sets only
`item_gap_degrees` keeps its angular gaps; geometry keys it leaves out now get
the values above.

### Menu items

Each `[[items]]` entry is a menu item, clockwise from the top:

| Key | Type | Description |
| --- | ---- | ----------- |
| `label` | string | Text shown under the icon. |
| `icon` | string | Glyph shown in the wedge (a Nerd Font icon). |
| `command` | string or array | Run on click. A string runs through `sh -c` (pipes, `$(...)`, `~` all work); a list like `["playerctl", "next"]` is run directly. Leave it out on submenu parents. |
| `fallback` | array | Commands tried in order when `command`'s program isn't installed, e.g. `["kitty", "foot", "alacritty"]`. |
| `items` | array | Child items; a non-empty list makes this a submenu ring. |

Nesting is recursive: a child can have its own `[[items.items.items]]`.

```toml
[[items]]
label = "Media"
icon = "󰝚"

  [[items.items]]
  label = "Play"
  icon = "󰐎"
  command = ["playerctl", "play-pause"]
```

### `[bar]`: the status pill

```toml
[bar]
enabled = true
```

The shipped pill shows the workspace, the focused window's title, the clock,
network, volume and battery. Each `[[bar.modules]]` is a Waybar-style module,
and a module whose command prints nothing is hidden, so missing hardware (a
desktop's battery, say) just disappears:

| Key | Type | Description |
| --- | ---- | ----------- |
| `command` | string | Shell command whose stdout becomes the module text. |
| `interval` | int | Seconds between runs (default 5). |
| `stream_command` | string | Long-lived command; each stdout line replaces the text (no polling). Used for Hyprland's event socket, `pactl subscribe`, etc. A stream that dies is respawned with backoff. |
| `watch` | array of strings | Files (sysfs works) whose changes re-run `command` immediately. |
| `format` | string | `{output}` is replaced by the text (default `"{output}"`). A format *without* `{output}` renders literally, but only while the command prints something. |
| `icon` | string | Optional leading glyph. |
| `max_length` | int | Longest text in characters; longer text is cut and ends in `…` (default `0`: no limit). |
| `on_click` / `on_click_right` | string | Shell command on left / right click. |
| `on_scroll_up` / `on_scroll_down` | string | Shell command on wheel up / down. |

Write multi-line commands as TOML literal strings (`'''...'''`): in `"""`
strings, `\n` and friends are TOML escapes and reach the shell changed.

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
- `.pill` — the status pill (`background-color`, `color`, `border-color`,
  `border-width`, `height`, `padding`, `margin-top`, `border-radius`, `font-*`,
  `--module-gap` for spacing, and `--separator-color` for the hairline between
  modules; `transparent` hides it)

Colors accept `#rgb` / `#rrggbb` / `#rrggbbaa`, `rgb()` / `rgba()` (channels as
standard 0–255 numbers, percentages, or the 0..1 floats used by the shipped
theme), `hsl()` / `hsla()`, or CSS color names. Sizes accept plain numbers,
`px`, or `pt` (leading-dot decimals like `.5px` included).

## Interaction

- **Show / hide** — run `hyprcircl` (or your keybinding) to toggle.
- **Click** — activate the hovered wedge; submenus open as new rings.
- **Mouse wheel over a pill applet** — runs that module's `on_scroll_up` /
  `on_scroll_down` (e.g. volume), otherwise switches workspaces.
- **Esc** — pop one submenu level; press again at the root to close.
- **← / →** or **h / l** — move selection between wedges (when `keyboard_navigation` is on).
- **1–9** — jump to wedge by number.
- **Enter / Space** — activate the selected wedge.
- **Backspace** — pop one submenu level without closing at root.
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

MIT — see [LICENSE](LICENSE).
