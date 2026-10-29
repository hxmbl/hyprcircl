#!/usr/bin/env python3
"""Check that a built hyprcircl binary loads a user's config dir and that its
hyprcircl.css still drives the look.

    scripts/screenshots/check-user-config.py path/to/hyprcircl [out-dir]

It writes a hyprcircl.toml and a loud hyprcircl.css into a throwaway
$XDG_CONFIG_HOME/hyprcircl (and then ~/.config/hyprcircl), renders real
`hyprcircl preview` frames with the binary and checks pixels: ring geometry
from the TOML; wedge, hover, stroke, icon and label colours from the CSS;
the pill's colour, height, position, text colour and font, and the module
separator (a colour, then `transparent`). When sway is installed, it also
starts the daemon on a headless sway output and checks its log for the same
two files and a live theme reload.

Standard library only. Exits non-zero on the first failed check.
"""

import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib

BIN = os.path.realpath(sys.argv[1]) if len(sys.argv) > 1 else "target/release/hyprcircl"
OUT = os.path.realpath(sys.argv[2]) if len(sys.argv) > 2 else None

W, H = 800, 700
CX, CY = 400, 420
INNER, OUTER = 30, 200
PILL_H, PILL_MARGIN = 60, 20

TOML = f"""
schema_version = 2

[menu]
inner_radius = {INNER}
outer_radius = {OUTER}
item_gap = 6
animation_ms = 0
labels = true

[bar]
enabled = true

[[bar.modules]]
command = "echo AAAA"

[[bar.modules]]
command = "echo BBBB"

[[items]]
label = "North"
icon = "N"
command = "true"

[[items]]
label = "East"
icon = "E"
command = "true"

[[items]]
label = "South"
icon = "S"
command = "true"

[[items]]
label = "West"
icon = "W"
command = "true"
"""

CSS = """
.backdrop { background-color: transparent; }
.shadow { display: none; }
.item { background-color: #ff0000; border-radius: 0; }
.item:active { background-color: #ff0000; }
.item:hover { background-color: #00ff00; }
.item-stroke { border-color: #00ffff; border-width: 4px; }
.item:hover .item-stroke { border-color: #00ffff; }
.icon { color: #ffffff; font-family: "DejaVu Sans"; font-size: 30pt; }
.item:hover .icon { color: #ffffff; }
.label { color: #ff8800; font-family: "DejaVu Sans"; font-size: 14pt; }
.item:hover .label { color: #ff8800; }
.pill {
    background-color: #0000ff;
    color: #ffff00;
    border-width: 0;
    height: %(pill_h)dpx;
    padding: 20px;
    margin-top: %(margin)dpx;
    border-radius: 0;
    font-family: "%(pill_font)s";
    font-size: %(pill_size)dpt;
    --module-gap: 40px;
    --separator-color: %(sep)s;
}
%(extra)s
"""

RED, GREEN, BLUE = (255, 0, 0), (0, 255, 0), (0, 0, 255)
CYAN, WHITE, ORANGE = (0, 255, 255), (255, 255, 255), (255, 136, 0)
YELLOW, MAGENTA = (255, 255, 0), (255, 0, 255)

failures = 0


def check(cond, what):
    global failures
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        failures += 1


def css(sep="#ff00ff", pill_font="DejaVu Sans", pill_size=14, extra=""):
    return CSS % dict(
        pill_h=PILL_H, margin=PILL_MARGIN, sep=sep, pill_font=pill_font,
        pill_size=pill_size, extra=extra,
    )


def read_png(path):
    """Decode an 8-bit RGBA, non-interlaced PNG (what cairo writes)."""
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    pos, idat, w = 8, b"", 0
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            assert (depth, ctype, interlace) == (8, 6, 0), (depth, ctype, interlace)
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride, bpp = w * 4, 4
    rows, prev, i = [], bytearray(stride), 0
    for _ in range(h):
        f = raw[i]
        line = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        rows.append(bytes(line))
        prev = line
    return w, h, rows


class Image:
    def __init__(self, path):
        self.w, self.h, self.rows = read_png(path)

    def px(self, x, y):
        o = x * 4
        return tuple(self.rows[y][o:o + 4])

    def near(self, x, y, rgb, tol=12):
        r, g, b, a = self.px(x, y)
        return a > 240 and all(abs(p - q) <= tol for p, q in zip((r, g, b), rgb))

    def where(self, rgb, box=None, tol=12):
        x0, y0, x1, y1 = box or (0, 0, self.w, self.h)
        return [(x, y) for y in range(y0, y1) for x in range(x0, x1) if self.near(x, y, rgb, tol)]

    def bbox(self, rgb):
        pts = self.where(rgb, tol=4)
        if not pts:
            return None
        xs, ys = [p[0] for p in pts], [p[1] for p in pts]
        return min(xs), min(ys), max(xs), max(ys)


def setup(root, cfgdir, toml=TOML, css_text=None):
    os.makedirs(cfgdir, exist_ok=True)
    with open(os.path.join(cfgdir, "hyprcircl.toml"), "w") as f:
        f.write(toml)
    with open(os.path.join(cfgdir, "hyprcircl.css"), "w") as f:
        f.write(css() if css_text is None else css_text)


def env_for(root, xdg):
    env = {k: v for k, v in os.environ.items() if k != "HYPRCIRCL_CONFIG"}
    env["HOME"] = os.path.join(root, "home")
    # Keep fonts installed under the real home visible to fontconfig.
    env["XDG_DATA_HOME"] = os.path.expanduser("~/.local/share")
    if xdg:
        env["XDG_CONFIG_HOME"] = os.path.join(root, "xdg")
    else:
        env.pop("XDG_CONFIG_HOME", None)
    return env


def render(root, env, name, hover=0):
    out = os.path.join(root, name + ".png")
    res = subprocess.run(
        [BIN, "preview", out, "--size", f"{W}x{H}", "--center", f"{CX},{CY}", "--hover", str(hover)],
        cwd=os.path.join(root, "cwd"), env=env, capture_output=True, text=True, timeout=60,
    )
    if res.returncode != 0:
        print(res.stdout, res.stderr)
        raise SystemExit(f"preview failed for {name}")
    if OUT:
        shutil.copy(out, os.path.join(OUT, "check-" + name + ".png"))
    return Image(out), res.stdout + res.stderr


def main():
    if OUT:
        os.makedirs(OUT, exist_ok=True)
    root = tempfile.mkdtemp(prefix="hyprcircl-check-")
    for d in ("home", "cwd"):
        os.makedirs(os.path.join(root, d))
    xdg_dir = os.path.join(root, "xdg", "hyprcircl")
    home_dir = os.path.join(root, "home", ".config", "hyprcircl")

    # --- 1. the config dir is what a real binary loads --------------------
    for label, cfgdir, xdg in (("$XDG_CONFIG_HOME", xdg_dir, True), ("~/.config", home_dir, False)):
        setup(root, cfgdir)
        env = env_for(root, xdg)
        toml_path = os.path.join(cfgdir, "hyprcircl.toml")
        css_path = os.path.join(cfgdir, "hyprcircl.css")
        res = subprocess.run([BIN, "config", "path"], cwd=os.path.join(root, "cwd"),
                             env=env, capture_output=True, text=True, timeout=30)
        check(toml_path in res.stdout, f"{label}: `config path` names {toml_path}  [{res.stdout.strip()}]")
        _, log = render(root, env, "load-" + ("xdg" if xdg else "home"))
        check(f"[CONFIG] Loaded {toml_path}" in log, f"{label}: preview loaded {toml_path}")
        check(f"[CSS] Loading {css_path}" in log, f"{label}: preview loaded {css_path}")
        if not xdg:
            shutil.rmtree(cfgdir)
    env = env_for(root, True)

    # --- 2. the user's CSS and TOML drive the render -----------------------
    img, _ = render(root, env, "base")
    check(img.px(5, 5)[3] == 0, "backdrop: transparent leaves the screen untouched")
    check(img.px(CX, CY)[3] == 0 and img.px(CX + 15, CY)[3] == 0,
          f"geometry: hole is empty (inner_radius {INNER})")
    check(img.near(CX + 45, CY, RED), "geometry: wedge starts just outside inner_radius 30 (default 60)")
    check(img.near(CX + 185, CY, RED), "colour: .item:active / .item fill is #ff0000")
    check(img.px(CX + 215, CY)[3] == 0, f"geometry: nothing past outer_radius {OUTER}")
    check(img.near(CX, CY - 185, GREEN), "colour: hovered wedge (item 0, 12 o'clock) is .item:hover #00ff00")
    ring = (CX - OUTER - 5, CY - OUTER - 5, CX + OUTER + 5, CY + OUTER + 5)
    check(len(img.where(CYAN, ring)) > 200, ".item-stroke border-color/width draws a cyan outline")
    check(len(img.where(WHITE, ring)) > 30, ".icon color #ffffff")
    check(len(img.where(ORANGE, ring)) > 30, ".label color #ff8800")

    pill = img.bbox(BLUE)
    check(pill is not None, ".pill background-color #0000ff is drawn")
    if pill:
        x0, y0, x1, y1 = pill
        top = CY - OUTER - PILL_MARGIN - PILL_H
        check(abs((y1 - y0 + 1) - PILL_H) <= 1, f".pill height {PILL_H}px (got {y1 - y0 + 1})")
        check(abs(y0 - top) <= 1, f".pill margin-top {PILL_MARGIN}px puts its top at y={top} (got {y0})")
        box = (x0, y0, x1 + 1, y1 + 1)
        check(len(img.where(YELLOW, box)) > 30, ".pill color #ffff00 colours the module text")
        check(len(img.where(MAGENTA, box)) > 10, ".pill --separator-color #ff00ff draws a separator")
        base_w = x1 - x0 + 1

    setup(root, xdg_dir, css_text=css(sep="transparent"))
    img2, _ = render(root, env, "separator-transparent")
    pill2 = img2.bbox(BLUE)
    check(pill2 is not None and not img2.where(MAGENTA, (pill2[0], pill2[1], pill2[2] + 1, pill2[3] + 1)),
          ".pill --separator-color: transparent hides the separator")

    setup(root, xdg_dir, css_text=css(pill_size=28))
    img3, _ = render(root, env, "pill-font-size")
    p3 = img3.bbox(BLUE)
    check(pill and p3 and (p3[2] - p3[0] + 1) > base_w + 40,
          f".pill font-size 28pt widens the pill ({base_w} -> {p3 and p3[2] - p3[0] + 1})")

    setup(root, xdg_dir, css_text=css(pill_font="DejaVu Sans Mono"))
    img4, _ = render(root, env, "pill-font-family")
    p4 = img4.bbox(BLUE)
    check(pill and p4 and (p4[2] - p4[0] + 1) != base_w,
          f".pill font-family changes the text ({base_w} -> {p4 and p4[2] - p4[0] + 1})")

    setup(root, xdg_dir, css_text=css(extra=".label { display: none; }"))
    img5, _ = render(root, env, "labels-hidden")
    check(not img5.where(ORANGE, ring), ".label display: none hides labels")

    setup(root, xdg_dir, toml=TOML.replace("labels = true", "labels = false"))
    img6, _ = render(root, env, "toml-labels-off")
    check(not img6.where(ORANGE, ring), "TOML [menu] labels = false hides labels")

    setup(root, xdg_dir, css_text=css(extra=".item:hover { background-color: #0000ff; }"))
    img7, _ = render(root, env, "hover-colour")
    check(img7.near(CX, CY - 185, BLUE), "a later .item:hover rule overrides an earlier one")

    # --- 3. the daemon itself, on a headless sway output --------------------
    if shutil.which("sway"):
        setup(root, xdg_dir)
        run = tempfile.mkdtemp(prefix="hyprcircl-sway-")
        os.chmod(run, 0o700)
        with open(os.path.join(run, "sway.conf"), "w") as f:
            f.write("output HEADLESS-1 resolution 1280x800\n")
        senv = dict(env, XDG_RUNTIME_DIR=run, WLR_BACKENDS="headless", WLR_RENDERER="pixman",
                    WLR_LIBINPUT_NO_DEVICES="1")
        sway = subprocess.Popen(["sway", "-c", os.path.join(run, "sway.conf")], env=senv,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        sock = None
        for _ in range(100):
            socks = [s for s in os.listdir(run) if s.startswith("wayland-") and not s.endswith(".lock")]
            if socks:
                sock = socks[0]
                break
            time.sleep(0.1)
        if not sock:
            sway.kill()
            raise SystemExit("headless sway did not start")
        logf = open(os.path.join(run, "hyprcircl.log"), "w+")
        denv = dict(senv, WAYLAND_DISPLAY=sock, GDK_BACKEND="wayland")
        daemon = subprocess.Popen([BIN], cwd=os.path.join(root, "cwd"), env=denv,
                                  stdout=logf, stderr=subprocess.STDOUT)
        time.sleep(3)
        with open(os.path.join(xdg_dir, "hyprcircl.css"), "a") as f:
            f.write("\n.pill { background-color: #00ff00; }\n")
        time.sleep(2)
        daemon.terminate()
        sway.terminate()
        daemon.wait(10)
        sway.wait(10)
        logf.seek(0)
        log = logf.read()
        print("--- daemon log ---\n" + "\n".join(
            l for l in log.splitlines() if any(k in l for k in ("CONFIG", "CSS", "THEME", "rror")))[:4000])
        check(f"[CONFIG] Loaded {os.path.join(xdg_dir, 'hyprcircl.toml')}" in log,
              "daemon: loaded hyprcircl.toml from $XDG_CONFIG_HOME/hyprcircl")
        check(f"[CSS] Loading {os.path.join(xdg_dir, 'hyprcircl.css')}" in log,
              "daemon: loaded hyprcircl.css from $XDG_CONFIG_HOME/hyprcircl")
        check("[THEME] Reloaded" in log, "daemon: an edit to hyprcircl.css reloads live")
        shutil.rmtree(run, ignore_errors=True)
    else:
        print("skip daemon checks: sway not installed")

    shutil.rmtree(root, ignore_errors=True)
    print(f"\n{failures} failed check(s)" if failures else "\nall checks passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
