// =========================================================================
// CSS-driven theme for every visual property of the radial menu.
//
// Users drop a `hyprcircl.css` next to their config and style the menu with
// plain CSS selectors:
//
//     .pill         { background-color: ...; color: ...; font-family: ...;
//                     font-size: ...; height: ...; padding: ...;
//                     margin-top: ...; border-radius: ...; }
//     .item         { background-color: ...; border-radius: ...; }
//     .item:hover   { background-color: ...; }
//     .item:active  { background-color: ...; }
//     .item:selected{ background-color: ...; }
//     .item-stroke  { border-color: ...; border-width: ...; }
//     .icon         { color: ...; font-size: ...; font-family: ...; font-weight: ...; }
//     .label        { color: ...; font-size: ...; font-family: ...; display: none; }
//
// A small self-contained CSS parser (no external crates) maps the subset of
// CSS we care about onto a typed `Theme`. Everything not mentioned in CSS
// keeps its default value, so an empty/missing file is a valid theme.
// =========================================================================

use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Default font family for all menu text (Nerd Font for icon glyphs).
pub const DEFAULT_FONT: &str = "JetBrainsMono Nerd Font";

/// Normalized RGBA color, each channel in 0.0..1.0.
pub type Rgba = (f64, f64, f64, f64);

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Top pill (Waybar-style bar) background / text.
    pub pill_bg: Rgba,
    pub pill_fg: Rgba,
    pub pill_font: String,
    /// Pango point size for pill text.
    pub pill_font_size: f64,
    pub pill_font_weight: i32,
    pub pill_height: f64,
    pub pill_padding_x: f64,
    /// Distance from the menu ring to the pill's bottom edge.
    pub pill_offset_y: f64,
    pub pill_corner: f64,
    /// Horizontal gap between pill modules.
    pub pill_gap: f64,

    /// Radial item fills by state.
    pub item_default: Rgba,
    pub item_hover: Rgba,
    pub item_active: Rgba,
    pub item_selected: Rgba,

    /// Item outline (stroke).
    pub item_stroke: Rgba,
    pub item_stroke_width: f64,
    pub item_corner: f64,

    /// Item icon glyphs.
    pub icon_color: Rgba,
    pub icon_font: String,
    pub icon_font_size: f64,
    pub icon_font_weight: i32,

    /// Item text labels.
    pub label_color: Rgba,
    pub label_font: String,
    pub label_font_size: f64,
    pub label_font_weight: i32,
    /// False when CSS sets `.label { display: none; }`.
    pub label_visible: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            pill_bg: (0.15, 0.15, 0.2, 0.9),
            pill_fg: (0.9, 0.9, 0.95, 1.0),
            pill_font: DEFAULT_FONT.into(),
            pill_font_size: 13.0,
            pill_font_weight: 700,
            pill_height: 28.0,
            pill_padding_x: 14.0,
            pill_offset_y: 24.0,
            pill_corner: 14.0,
            pill_gap: 18.0,
            item_default: (0.85, 0.85, 0.88, 0.9),
            item_hover: (0.48, 0.63, 0.96, 0.95),
            item_active: (0.85, 0.85, 0.88, 0.9),
            item_selected: (0.88, 0.29, 0.29, 0.95),
            item_stroke: (0.7, 0.7, 0.75, 1.0),
            item_stroke_width: 1.5,
            item_corner: 8.0,
            icon_color: (0.05, 0.05, 0.1, 1.0),
            icon_font: DEFAULT_FONT.into(),
            icon_font_size: 20.0,
            icon_font_weight: 700,
            label_color: (0.15, 0.15, 0.2, 0.85),
            label_font: DEFAULT_FONT.into(),
            label_font_size: 8.0,
            label_font_weight: 700,
            label_visible: true,
        }
    }
}

impl Theme {
    /// Build a theme from a CSS string, falling back to defaults for
    /// properties the CSS does not mention.
    pub fn from_css(css: &str) -> Theme {
        let mut theme = Theme::default();
        theme.merge_css(css);
        theme
    }

    /// Overlay the given CSS onto the current theme.
    pub fn merge_css(&mut self, css: &str) {
        for rule in parse_rules(&tokenize(css)) {
            let Some(target) = match_selector(&rule.selectors) else {
                continue;
            };
            for (prop, vals) in &rule.decls {
                apply_decl(self, &target, prop, vals);
            }
        }
    }
}

// =========================================================================
// CSS tokenizer (subset: selectors, blocks, declarations, values we use)
// =========================================================================

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Lbrace,
    Rbrace,
    Lparen,
    Rparen,
    Colon,
    Semicolon,
    Comma,
    Dot,
    Ident(String),
    Hash(String),
    Number(f64),
    Dimension(f64, Unit),
    Percentage(f64),
    Str(String),
    /// `name(` — the following `(` is consumed; args end at Rparen.
    Function(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Px,
    Pt,
    Em,
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '-'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn tokenize(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        match c {
            '{' => {
                out.push(Tok::Lbrace);
                i += 1;
            }
            '}' => {
                out.push(Tok::Rbrace);
                i += 1;
            }
            '(' => {
                out.push(Tok::Lparen);
                i += 1;
            }
            ')' => {
                out.push(Tok::Rparen);
                i += 1;
            }
            ':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            ';' => {
                out.push(Tok::Semicolon);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            // A `.` followed by a digit is a leading-dot decimal (`.5px`) and
            // must fall through to the number scanner below, not become a
            // selector Dot token.
            '.' if !(i + 1 < b.len() && (b[i + 1] as char).is_ascii_digit()) => {
                out.push(Tok::Dot);
                i += 1;
            }
            '/' if i + 1 < b.len() && b[i + 1] as char == '*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] as char == '*' && b[i + 1] as char == '/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            '#' => {
                let mut j = i + 1;
                while j < b.len() && is_name_char(b[j] as char) {
                    j += 1;
                }
                out.push(Tok::Hash(s[i + 1..j].to_string()));
                i = j;
            }
            '"' | '\'' => {
                let q = c;
                i += 1;
                let mut val = String::new();
                while i < b.len() {
                    // Decode one full UTF-8 scalar so non-ASCII font names
                    // survive instead of being mangled byte-by-byte.
                    let ch = s[i..].chars().next().unwrap();
                    if ch == '\\' && i + 1 < b.len() {
                        let esc = s[i + 1..].chars().next().unwrap();
                        val.push(esc);
                        i += 1 + esc.len_utf8();
                    } else if ch == q {
                        i += 1;
                        break;
                    } else {
                        val.push(ch);
                        i += ch.len_utf8();
                    }
                }
                out.push(Tok::Str(val));
            }
            c if c.is_ascii_digit()
                // `-5`, `-.5`: sign must be followed by a number start.
                || (c == '-'
                    && i + 1 < b.len()
                    && ((b[i + 1] as char).is_ascii_digit()
                        || (b[i + 1] as char == '.'
                            && i + 2 < b.len()
                            && (b[i + 2] as char).is_ascii_digit())))
                || (c == '.' && i + 1 < b.len() && (b[i + 1] as char).is_ascii_digit()) =>
            {
                let start = i;
                if c == '-' {
                    i += 1;
                }
                while i < b.len() && (b[i] as char).is_ascii_digit() {
                    i += 1;
                }
                if i < b.len() && b[i] as char == '.' {
                    i += 1;
                    while i < b.len() && (b[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < b.len() && (b[i] as char == 'e' || b[i] as char == 'E') {
                    let mut j = i + 1;
                    if j < b.len() && (b[j] as char == '+' || b[j] as char == '-') {
                        j += 1;
                    }
                    if j < b.len() && (b[j] as char).is_ascii_digit() {
                        i = j;
                        while i < b.len() && (b[i] as char).is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let num: f64 = s[start..i].parse().unwrap_or(0.0);
                let mut j = i;
                while j < b.len() && is_name_char(b[j] as char) {
                    j += 1;
                }
                if j > i {
                    let unit = &s[i..j];
                    let u = match unit {
                        "px" => Unit::Px,
                        "pt" => Unit::Pt,
                        "em" | "rem" => Unit::Em,
                        _ => Unit::Px,
                    };
                    out.push(Tok::Dimension(num, u));
                    i = j;
                } else if i < b.len() && b[i] as char == '%' {
                    out.push(Tok::Percentage(num));
                    i += 1;
                } else {
                    out.push(Tok::Number(num));
                }
            }
            c if is_name_start(c) => {
                let start = i;
                i += 1;
                while i < b.len() && is_name_char(b[i] as char) {
                    i += 1;
                }
                let name: String = s[start..i].to_string();
                if i < b.len() && b[i] as char == '(' {
                    out.push(Tok::Function(name));
                    i += 1;
                } else {
                    out.push(Tok::Ident(name));
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    out
}

// =========================================================================
// Rule + declaration parser
// =========================================================================

struct Rule {
    selectors: Vec<Tok>,
    decls: Vec<(String, Vec<Tok>)>,
}

fn parse_rules(toks: &[Tok]) -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        // Selector tokens up to `{`.
        let mut selectors = Vec::new();
        while i < toks.len() && toks[i] != Tok::Lbrace {
            if toks[i] == Tok::Rbrace || toks[i] == Tok::Semicolon {
                // Stray closing token with no block: skip it and start fresh.
                i += 1;
                continue;
            }
            selectors.push(toks[i].clone());
            i += 1;
        }
        if i >= toks.len() {
            break;
        }
        i += 1; // consume `{`

        // Declarations up to `}`.
        let mut decls = Vec::new();
        let mut prop: Option<String> = None;
        let mut vals: Vec<Tok> = Vec::new();
        while i < toks.len() && toks[i] != Tok::Rbrace {
            match &toks[i] {
                Tok::Semicolon => {
                    if let Some(p) = prop.take() {
                        decls.push((p, std::mem::take(&mut vals)));
                    }
                    i += 1;
                }
                Tok::Ident(name)
                    if prop.is_none()
                        && vals.is_empty()
                        && matches!(toks.get(i + 1), Some(Tok::Colon)) =>
                {
                    prop = Some(name.clone());
                    i += 1;
                }
                _ => {
                    vals.push(toks[i].clone());
                    i += 1;
                }
            }
        }
        if let Some(p) = prop.take() {
            decls.push((p, vals));
        }
        if i < toks.len() {
            i += 1; // consume `}`
        }

        if !selectors.is_empty() {
            rules.push(Rule { selectors, decls });
        }
    }
    rules
}

// =========================================================================
// Selector matching → (element, state)
// =========================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
enum Element {
    Pill,
    Item,
    Stroke,
    Icon,
    Label,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Base,
    Hover,
    Active,
    Selected,
}

struct Target {
    element: Element,
    state: State,
}

fn match_selector(sel: &[Tok]) -> Option<Target> {
    let mut classes = Vec::new(); // Ident after `.` or any Hash (`#name`)
    let mut pseudos = Vec::new(); // Ident after `:`
    let mut bare = Vec::new(); // other idents (element names)
    let mut prev: Option<Tok> = None;
    for t in sel {
        match t {
            Tok::Hash(name) => classes.push(name.clone()),
            Tok::Ident(name) => match &prev {
                Some(Tok::Dot) => classes.push(name.clone()),
                Some(Tok::Colon) => pseudos.push(name.clone()),
                _ => bare.push(name.clone()),
            },
            _ => {}
        }
        prev = Some(t.clone());
    }

    let element = if classes.iter().any(|c| c == "item-stroke") {
        Element::Stroke
    } else if classes.iter().any(|c| c == "pill") {
        Element::Pill
    } else if classes.iter().any(|c| c == "icon") {
        Element::Icon
    } else if classes.iter().any(|c| c == "label") {
        Element::Label
    } else if classes.iter().any(|c| c == "item") {
        Element::Item
    } else if bare.iter().any(|s| s == "window" || s == "body") {
        return None; // window-level styling is handled by GTK's CSS engine
    } else {
        return None;
    };

    let state = if pseudos.iter().any(|p| p == "hover" || p == "focus") {
        State::Hover
    } else if pseudos.iter().any(|p| p == "active") {
        State::Active
    } else if pseudos.iter().any(|p| p == "selected") {
        State::Selected
    } else {
        State::Base
    };

    Some(Target { element, state })
}

// =========================================================================
// Declaration application
// =========================================================================

fn apply_decl(theme: &mut Theme, target: &Target, prop: &str, vals: &[Tok]) {
    let color = parse_color(vals);
    match (target.element, target.state) {
        (Element::Pill, _) => match prop {
            "background-color" | "background" => {
                if let Some(c) = color {
                    theme.pill_bg = c;
                }
            }
            "color" => {
                if let Some(c) = color {
                    theme.pill_fg = c;
                }
            }
            "font-family" => {
                if let Some(f) = font_family(vals) {
                    theme.pill_font = f;
                }
            }
            "font-size" => {
                if let Some(n) = number(vals) {
                    theme.pill_font_size = n;
                }
            }
            "font-weight" => {
                if let Some(w) = weight(vals) {
                    theme.pill_font_weight = w;
                }
            }
            "height" => {
                if let Some(n) = number(vals) {
                    theme.pill_height = n;
                }
            }
            "padding" | "padding-x" => {
                if let Some(n) = number(vals) {
                    theme.pill_padding_x = n;
                }
            }
            "margin-top" | "offset-y" => {
                if let Some(n) = number(vals) {
                    theme.pill_offset_y = n;
                }
            }
            "border-radius" => {
                if let Some(n) = number(vals) {
                    theme.pill_corner = n;
                }
            }
            "--module-gap" | "gap" => {
                if let Some(n) = number(vals) {
                    theme.pill_gap = n;
                }
            }
            _ => {}
        },
        (Element::Item, state) => {
            if prop == "background-color" || prop == "background" {
                if let Some(c) = color {
                    match state {
                        State::Base => theme.item_default = c,
                        State::Hover => theme.item_hover = c,
                        State::Active => theme.item_active = c,
                        State::Selected => theme.item_selected = c,
                    }
                }
            } else if prop == "border-radius" && state == State::Base {
                if let Some(n) = number(vals) {
                    theme.item_corner = n;
                }
            }
        }
        (Element::Stroke, _) => match prop {
            "border-color" | "color" => {
                if let Some(c) = color {
                    theme.item_stroke = c;
                }
            }
            "border-width" => {
                if let Some(n) = number(vals) {
                    theme.item_stroke_width = n;
                }
            }
            _ => {}
        },
        (Element::Icon, _) => match prop {
            "color" => {
                if let Some(c) = color {
                    theme.icon_color = c;
                }
            }
            "font-family" => {
                if let Some(f) = font_family(vals) {
                    theme.icon_font = f;
                }
            }
            "font-size" => {
                if let Some(n) = number(vals) {
                    theme.icon_font_size = n;
                }
            }
            "font-weight" => {
                if let Some(w) = weight(vals) {
                    theme.icon_font_weight = w;
                }
            }
            _ => {}
        },
        (Element::Label, _) => match prop {
            "color" => {
                if let Some(c) = color {
                    theme.label_color = c;
                }
            }
            "font-family" => {
                if let Some(f) = font_family(vals) {
                    theme.label_font = f;
                }
            }
            "font-size" => {
                if let Some(n) = number(vals) {
                    theme.label_font_size = n;
                }
            }
            "font-weight" => {
                if let Some(w) = weight(vals) {
                    theme.label_font_weight = w;
                }
            }
            "display" if display_none(vals) => theme.label_visible = false,
            _ => {}
        },
    }
}

// =========================================================================
// Value helpers
// =========================================================================

/// First numeric value in a declaration (px/pt/em/bare all read as a raw
/// number — pango sizes are points, geometry is canvas pixels).
fn number(vals: &[Tok]) -> Option<f64> {
    for t in vals {
        match t {
            Tok::Number(v) | Tok::Dimension(v, _) | Tok::Percentage(v) => return Some(*v),
            _ => {}
        }
    }
    None
}

/// First font-family value: a quoted string is used verbatim; unquoted
/// multi-word families ("JetBrains Mono") are joined until the list ends
/// or the next comma-separated family starts.
fn font_family(vals: &[Tok]) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for t in vals {
        match t {
            Tok::Str(s) => return Some(s.clone()),
            Tok::Ident(s) => parts.push(s.as_str()),
            Tok::Comma | Tok::Number(_) | Tok::Dimension(..) | Tok::Percentage(_) => break,
            _ => {}
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn weight(vals: &[Tok]) -> Option<i32> {
    for t in vals {
        match t {
            Tok::Number(v) => return Some(*v as i32),
            Tok::Ident(s) => match s.to_ascii_lowercase().as_str() {
                "normal" => return Some(400),
                "bold" => return Some(700),
                "bolder" => return Some(800),
                "lighter" => return Some(300),
                _ => {}
            },
            _ => {}
        }
    }
    None
}

fn display_none(vals: &[Tok]) -> bool {
    vals.iter()
        .any(|t| matches!(t, Tok::Ident(s) if s.eq_ignore_ascii_case("none")))
}

/// Parse a color out of a declaration's tokens (hex, rgb/rgba, hsl/hsla,
/// or a named color).
fn parse_color(vals: &[Tok]) -> Option<Rgba> {
    for t in vals {
        match t {
            Tok::Hash(h) => return parse_hex(h),
            Tok::Ident(name) => {
                if name.eq_ignore_ascii_case("transparent") {
                    return Some((0.0, 0.0, 0.0, 0.0));
                }
                if let Some(c) = named_color(name) {
                    return Some(c);
                }
            }
            Tok::Function(name) => {
                let upper = name.to_ascii_uppercase();
                let nums = value_components(vals);
                // Alpha: percentage or 0..1 float (CSS never uses 0-255 here).
                let alpha = |nums: &[(f64, bool)]| -> f64 {
                    match nums.get(3) {
                        Some((v, true)) => (*v / 100.0).clamp(0.0, 1.0),
                        Some((v, false)) => v.clamp(0.0, 1.0),
                        None => 1.0,
                    }
                };
                return match upper.as_str() {
                    "RGB" | "RGBA" if nums.len() >= 3 => {
                        // Channels honour percentages; plain numbers accept
                        // both the 0-255 CSS range and the 0..1 float range.
                        let chan = |i: usize| -> f64 {
                            let (v, pct) = nums[i];
                            if pct {
                                (v / 100.0).clamp(0.0, 1.0)
                            } else {
                                channel(v)
                            }
                        };
                        Some((chan(0), chan(1), chan(2), alpha(&nums)))
                    }
                    "HSL" | "HSLA" if nums.len() >= 3 => {
                        let (r, g, b) = hsl_to_rgb(nums[0].0, nums[1].0, nums[2].0);
                        Some((r, g, b, alpha(&nums)))
                    }
                    _ => None,
                };
            }
            _ => {}
        }
    }
    None
}

/// Every numeric component in a declaration, paired with whether it was
/// written as a percentage (used for function arguments).
fn value_components(vals: &[Tok]) -> Vec<(f64, bool)> {
    vals.iter()
        .filter_map(|t| match t {
            Tok::Number(v) => Some((*v, false)),
            Tok::Dimension(v, _) => Some((*v, false)),
            Tok::Percentage(v) => Some((*v, true)),
            _ => None,
        })
        .collect()
}

/// Normalize an RGB channel. Accepts both standard CSS 0–255 numbers
/// (`rgb(200, 150, 100)`) and the 0..1 float convention used by the
/// shipped theme (`rgba(0.85, 0.85, 0.88, 0.90)`): values ≤ 1 pass
/// through, larger values are scaled down from the 255 range.
fn channel(v: f64) -> f64 {
    if v > 1.0 {
        (v / 255.0).clamp(0.0, 1.0)
    } else {
        v.clamp(0.0, 1.0)
    }
}

fn parse_hex(s: &str) -> Option<Rgba> {
    let b = s.as_bytes();
    let nibble = |c: u8| -> Option<f64> { (c as char).to_digit(16).map(|d| d as f64 / 15.0) };
    let byte = |pair: &[u8]| -> Option<f64> {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        Some((hi * 16 + lo) as f64 / 255.0)
    };
    match b.len() {
        3 => Some((nibble(b[0])?, nibble(b[1])?, nibble(b[2])?, 1.0)),
        4 => Some((nibble(b[0])?, nibble(b[1])?, nibble(b[2])?, nibble(b[3])?)),
        6 => Some((byte(&b[0..2])?, byte(&b[2..4])?, byte(&b[4..6])?, 1.0)),
        8 => Some((
            byte(&b[0..2])?,
            byte(&b[2..4])?,
            byte(&b[4..6])?,
            byte(&b[6..8])?,
        )),
        _ => None,
    }
}

fn named_color(name: &str) -> Option<Rgba> {
    let c: &str = match name.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "ffffff",
        "red" => "ff0000",
        "green" => "008000",
        "blue" => "0000ff",
        "yellow" => "ffff00",
        "cyan" | "aqua" => "00ffff",
        "magenta" | "fuchsia" => "ff00ff",
        "orange" => "ffa500",
        "purple" => "800080",
        "pink" => "ffc0cb",
        "gray" | "grey" => "808080",
        "brown" => "a52a2a",
        "lime" => "00ff00",
        "olive" => "808000",
        "navy" => "000080",
        "teal" => "008080",
        "maroon" => "800000",
        "silver" => "c0c0c0",
        "gold" => "ffd700",
        _ => return None,
    };
    parse_hex(c)
}

/// HSL → RGB (h in degrees, s/l as 0..1 or 0..100).
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let h = h.rem_euclid(360.0) / 360.0;
    let s = if s > 1.0 { s / 100.0 } else { s }.clamp(0.0, 1.0);
    let l = if l > 1.0 { l / 100.0 } else { l }.clamp(0.0, 1.0);
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

// =========================================================================
// Live reload
// =========================================================================

/// Background thread: re-parse the CSS file whenever it changes and swap the
/// new theme into the shared `RwLock`, so CSS edits apply without restarting.
pub fn watch_theme(css_path: String, theme: Arc<RwLock<Theme>>) {
    std::thread::spawn(move || {
        let mut last: Option<String> = None;
        loop {
            let contents = std::fs::read_to_string(&css_path).ok();
            if contents.is_some() && contents != last {
                last = contents.clone();
                let new = Theme::from_css(contents.as_deref().unwrap_or(""));
                if let Ok(mut w) = theme.write() {
                    *w = new;
                }
                println!("[THEME] Reloaded {css_path}");
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
}

// =========================================================================
// Unit tests for CSS parsing and theme merging
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Rgba, b: Rgba) -> bool {
        let eps = 0.001;
        (a.0 - b.0).abs() < eps
            && (a.1 - b.1).abs() < eps
            && (a.2 - b.2).abs() < eps
            && (a.3 - b.3).abs() < eps
    }

    #[test]
    fn defaults_are_stable() {
        let t = Theme::default();
        assert_eq!(t.pill_font, DEFAULT_FONT);
        assert!(t.label_visible);
        assert!(t.pill_height > 0.0);
        assert!(t.pill_font_size > 0.0);
        assert!(t.item_corner > 0.0);
        assert!(t.item_stroke_width > 0.0);
    }

    #[test]
    fn empty_css_changes_nothing() {
        let t = Theme::from_css("");
        assert_eq!(t, Theme::default());
        let t2 = Theme::from_css("/* only a comment */ \n {}");
        assert_eq!(t2, Theme::default());
    }

    #[test]
    fn item_colors_and_pseudo_classes() {
        let css = r#"
            .item { background: #abcdef; border-radius: 10px; }
            .item:hover { background: red; }
            .item:active { background: rgb(50%, 0, 0); }
            .item:selected { background: hsla(200, 100%, 50%, 0.5); }
        "#;
        let t = Theme::from_css(css);
        assert!(approx(
            t.item_default,
            (
                0xab as f64 / 255.0,
                0xcd as f64 / 255.0,
                0xef as f64 / 255.0,
                1.0
            )
        ));
        assert_eq!(t.item_corner, 10.0);
        assert!(approx(t.item_hover, (1.0, 0.0, 0.0, 1.0)));
        assert!(approx(t.item_active, (0.5, 0.0, 0.0, 1.0)));
        assert!((t.item_selected.3 - 0.5).abs() < 0.001);
        assert!((t.item_selected.2 - 1.0).abs() < 0.001);
    }

    #[test]
    fn stroke_and_icon_label() {
        let css = r#"
            .item-stroke { border-color: #222222; border-width: 3px; }
            .icon { color: #0d0d1a; font-size: 22px; font-family: "Fira Code"; font-weight: 600; }
            .label { color: #262633; font-size: 9px; }
            .label { display: none; }
        "#;
        let t = Theme::from_css(css);
        assert!(approx(
            t.item_stroke,
            (
                0x22 as f64 / 255.0,
                0x22 as f64 / 255.0,
                0x22 as f64 / 255.0,
                1.0
            )
        ));
        assert_eq!(t.item_stroke_width, 3.0);
        assert_eq!(t.icon_font, "Fira Code");
        assert_eq!(t.icon_font_size, 22.0);
        assert_eq!(t.icon_font_weight, 600);
        assert_eq!(t.label_font_size, 9.0);
        assert!(!t.label_visible);
    }

    #[test]
    fn pill_properties() {
        let css = r#"
            .pill {
                background-color: rgba(0.1, 0.1, 0.15, 0.95);
                color: white;
                font-family: 'Mono Sans';
                font-size: 15px;
                font-weight: bold;
                height: 32px;
                padding: 12px;
                margin-top: 30px;
                border-radius: 16px;
                --module-gap: 20px;
            }
        "#;
        let t = Theme::from_css(css);
        assert!(approx(t.pill_bg, (0.1, 0.1, 0.15, 0.95)));
        assert!(approx(t.pill_fg, (1.0, 1.0, 1.0, 1.0)));
        assert_eq!(t.pill_font, "Mono Sans");
        assert_eq!(t.pill_font_size, 15.0);
        assert_eq!(t.pill_font_weight, 700);
        assert_eq!(t.pill_height, 32.0);
        assert_eq!(t.pill_padding_x, 12.0);
        assert_eq!(t.pill_offset_y, 30.0);
        assert_eq!(t.pill_corner, 16.0);
        assert_eq!(t.pill_gap, 20.0);
    }

    #[test]
    fn hex_formats() {
        assert!(approx(parse_hex("fff").unwrap(), (1.0, 1.0, 1.0, 1.0)));
        assert!(approx(
            parse_hex("f008").unwrap(),
            (1.0, 0.0, 0.0, 0x88 as f64 / 255.0)
        ));
        assert!(approx(parse_hex("ff0000").unwrap(), (1.0, 0.0, 0.0, 1.0)));
        assert!(approx(
            parse_hex("ff000080").unwrap(),
            (1.0, 0.0, 0.0, 0x80 as f64 / 255.0)
        ));
        // 5-digit sequences are not valid hex lengths.
        assert_eq!(parse_hex("fffff"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn named_colors_and_transparent() {
        assert!(approx(
            parse_color(&[Tok::Ident("reD".into())]).unwrap(),
            (1.0, 0.0, 0.0, 1.0)
        ));
        assert!(approx(
            parse_color(&[Tok::Ident("transparent".into())]).unwrap(),
            (0.0, 0.0, 0.0, 0.0)
        ));
        assert!(parse_color(&[Tok::Ident("nope".into())]).is_none());
    }

    #[test]
    fn comments_and_malformed_input_are_tolerated() {
        let css = r#"
            /* stray block with no selector */
            { color: red; }
            .icon { color: /* inline */ #0a0a0a; }
            ;; ;
            .label { color: lime; }
        "#;
        let t = Theme::from_css(css);
        assert!(approx(
            t.icon_color,
            (
                0x0a as f64 / 255.0,
                0x0a as f64 / 255.0,
                0x0a as f64 / 255.0,
                1.0
            )
        ));
        assert!(approx(t.label_color, (0.0, 1.0, 0.0, 1.0)));
    }

    #[test]
    fn window_selector_is_ignored_by_theme() {
        // Window-level styling is owned by GTK's CSS engine, not the theme.
        let css = r#"window { background-color: rgba(1, 0, 0, 1); } body { color: blue; }"#;
        let t = Theme::from_css(css);
        assert_eq!(t, Theme::default());
    }

    #[test]
    fn font_weight_keywords() {
        let css = ".icon { font-weight: normal; } .label { font-weight: bold; }";
        let t = Theme::from_css(css);
        assert_eq!(t.icon_font_weight, 400);
        assert_eq!(t.label_font_weight, 700);
    }

    #[test]
    fn rgb_accepts_0_255_and_0_1_channels() {
        // Standard CSS 8-bit channels must not saturate.
        assert!(approx(
            parse_color(&[
                Tok::Function("rgb".into()),
                Tok::Number(200.0),
                Tok::Comma,
                Tok::Number(150.0),
                Tok::Comma,
                Tok::Number(100.0),
            ])
            .unwrap(),
            (200.0 / 255.0, 150.0 / 255.0, 100.0 / 255.0, 1.0)
        ));
        // The shipped theme's float convention keeps passing through.
        assert!(approx(
            parse_color(&[
                Tok::Function("rgba".into()),
                Tok::Number(0.85),
                Tok::Comma,
                Tok::Number(0.85),
                Tok::Comma,
                Tok::Number(0.88),
                Tok::Comma,
                Tok::Number(0.90),
            ])
            .unwrap(),
            (0.85, 0.85, 0.88, 0.90)
        ));
        // Percentages still work.
        assert!(approx(
            parse_color(&[
                Tok::Function("rgb".into()),
                Tok::Percentage(50.0),
                Tok::Comma,
                Tok::Number(0.0),
                Tok::Comma,
                Tok::Number(0.0),
            ])
            .unwrap(),
            (0.5, 0.0, 0.0, 1.0)
        ));
    }

    #[test]
    fn hex_rejects_invalid_digits() {
        assert_eq!(parse_hex("ggg"), None);
        assert_eq!(parse_hex("ff00zz"), None);
    }

    #[test]
    fn leading_dot_decimals_parse_at_fractional_value() {
        // `.5px` is valid CSS; it must not become `5px` (10x error).
        let t = Theme::from_css(".item { border-radius: .5px; }");
        assert!((t.item_corner - 0.5).abs() < 1e-9);
        let t = Theme::from_css(".item-stroke { border-width: -.25px; }");
        assert!((t.item_stroke_width + 0.25).abs() < 1e-9);
    }

    #[test]
    fn unquoted_multiword_font_family_is_joined() {
        let t = Theme::from_css(".icon { font-family: JetBrains Mono; }");
        assert_eq!(t.icon_font, "JetBrains Mono");
        // First family of a comma list wins.
        let t = Theme::from_css(".pill { font-family: JetBrains Mono, sans-serif; }");
        assert_eq!(t.pill_font, "JetBrains Mono");
    }
}
