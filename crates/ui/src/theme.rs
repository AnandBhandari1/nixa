use gpui::{App, Global, Hsla, Rgba, SharedString, rgb, rgba};
use nixa_protocol::{Status, paths};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TermColors {
    pub background: Hsla,
    pub foreground: Hsla,
    pub selection: Hsla,
    pub cursor: Hsla,
    pub ansi: [Hsla; 16],
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: SharedString,
    pub label: SharedString,
    pub canvas: Hsla,
    pub chrome: Hsla,
    pub background: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub popover: Hsla,
    pub input: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub foreground: Hsla,
    pub bright: Hsla,
    pub muted: Hsla,
    pub dim: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_foreground: Hsla,
    pub solid: Hsla,
    pub solid_foreground: Hsla,
    pub ring: Hsla,
    pub brand: Hsla,
    pub red: Hsla,
    pub green: Hsla,
    pub yellow: Hsla,
    pub blue: Hsla,
    pub magenta: Hsla,
    pub cyan: Hsla,
    pub orange: Hsla,
    pub working: Hsla,
    pub blocked: Hsla,
    pub done: Hsla,
    pub idle: Hsla,
    pub error: Hsla,
    pub unknown: Hsla,
    pub scrim: Hsla,
    pub shadow: Hsla,
    pub terminal: TermColors,
}

impl Global for Theme {}

#[derive(Clone, Debug)]
pub struct Themes {
    pub dir: PathBuf,
    pub list: Vec<Theme>,
}

impl Global for Themes {}

pub const DEFAULT_THEME: &str = "atom-green";
pub const CHOICE_FILE: &str = "theme.json";
pub const USER_DIR: &str = "themes";

const BUILTIN: &[(&str, &str)] = &[
    ("one-dark-pro", include_str!("../themes/one-dark-pro.json")),
    ("atom-one-dark", include_str!("../themes/atom-one-dark.json")),
    ("atom-green", include_str!("../themes/atom-green.json")),
    ("graphite", include_str!("../themes/graphite.json")),
    ("ink", include_str!("../themes/ink.json")),
    ("moss", include_str!("../themes/moss.json")),
    ("matrix", include_str!("../themes/matrix.json")),
];

const BRAND: u32 = 0x54d1e3;

#[derive(Clone, Debug, PartialEq)]
enum Json {
    Obj(Vec<(String, Json)>),
    Arr(Vec<Json>),
    Str(String),
    Other,
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn hex_digits(b: &[u8], i: &mut usize) -> Option<u32> {
    let digits = std::str::from_utf8(b.get(*i..*i + 4)?).ok()?;
    *i += 4;
    u32::from_str_radix(digits, 16).ok()
}

fn json_string(b: &[u8], i: &mut usize) -> Option<String> {
    *i += 1;
    let mut out = Vec::new();
    while *i < b.len() {
        let c = b[*i];
        *i += 1;
        match c {
            b'"' => return String::from_utf8(out).ok(),
            b'\\' => {
                let e = *b.get(*i)?;
                *i += 1;
                let ch = match e {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'u' => char::from_u32(hex_digits(b, i)?).unwrap_or('\u{fffd}'),
                    other => other as char,
                };
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            _ => out.push(c),
        }
    }
    None
}

fn json_value(b: &[u8], i: &mut usize, depth: usize) -> Option<Json> {
    if depth > 32 {
        return None;
    }
    skip_ws(b, i);
    match *b.get(*i)? {
        b'{' => {
            *i += 1;
            let mut fields = Vec::new();
            loop {
                skip_ws(b, i);
                match *b.get(*i)? {
                    b'}' => {
                        *i += 1;
                        return Some(Json::Obj(fields));
                    }
                    b',' => *i += 1,
                    b'"' => {
                        let key = json_string(b, i)?;
                        skip_ws(b, i);
                        if *b.get(*i)? != b':' {
                            return None;
                        }
                        *i += 1;
                        fields.push((key, json_value(b, i, depth + 1)?));
                    }
                    _ => return None,
                }
            }
        }
        b'[' => {
            *i += 1;
            let mut items = Vec::new();
            loop {
                skip_ws(b, i);
                match *b.get(*i)? {
                    b']' => {
                        *i += 1;
                        return Some(Json::Arr(items));
                    }
                    b',' => *i += 1,
                    _ => items.push(json_value(b, i, depth + 1)?),
                }
            }
        }
        b'"' => json_string(b, i).map(Json::Str),
        _ => {
            let start = *i;
            while *i < b.len() && !matches!(b[*i], b',' | b'}' | b']') && !b[*i].is_ascii_whitespace() {
                *i += 1;
            }
            (*i > start).then_some(Json::Other)
        }
    }
}

fn parse_json(text: &str) -> Option<Json> {
    let b = text.as_bytes();
    let mut i = 0;
    let value = json_value(b, &mut i, 0)?;
    skip_ws(b, &mut i);
    (i == b.len()).then_some(value)
}

fn field<'a>(json: &'a Json, key: &str) -> Option<&'a Json> {
    match json {
        Json::Obj(fields) => fields.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn text_of(json: Option<&Json>) -> Option<&str> {
    match json {
        Some(Json::Str(s)) => Some(s),
        _ => None,
    }
}

pub fn parse_hex(text: &str) -> Option<Hsla> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        3 => {
            let (r, g, b) = ((value >> 8) & 0xf, (value >> 4) & 0xf, value & 0xf);
            Some(rgb(((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11)).into())
        }
        6 => Some(rgb(value).into()),
        8 => Some(rgba(value).into()),
        _ => None,
    }
}

fn color(json: &Json, key: &str) -> Option<Hsla> {
    text_of(field(json, key)).and_then(parse_hex)
}

fn shade(c: Hsla, delta: f32) -> Hsla {
    Hsla {
        l: (c.l + delta).clamp(0.0, 1.0),
        ..c
    }
}

pub fn mix(a: Hsla, b: Hsla, amount: f32) -> Hsla {
    let (a, b): (Rgba, Rgba) = (a.into(), b.into());
    let k = amount.clamp(0.0, 1.0);
    let lerp = |x: f32, y: f32| x + (y - x) * k;
    Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
    .into()
}

fn default_ansi() -> [Hsla; 16] {
    [
        0x3e4451, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf, 0x5c6370,
        0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
    ]
    .map(|v| rgb(v).into())
}

fn ansi_of(json: &Json, fallback: [Hsla; 16]) -> [Hsla; 16] {
    let mut ansi = fallback;
    if let Some(Json::Arr(items)) = field(json, "ansi") {
        for (slot, item) in ansi.iter_mut().zip(items) {
            if let Some(c) = text_of(Some(item)).and_then(parse_hex) {
                *slot = c;
            }
        }
    }
    ansi
}

fn title_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn derive(name: &str, label: &str, terminal: TermColors, bg: Hsla, fg: Hsla, accent: Hsla) -> Theme {
    let k = if bg.l < 0.5 { 1.0 } else { -1.0 };
    let up = |d: f32| shade(bg, k * d);
    let down = |d: f32| shade(bg, -k * d);
    let far = if bg.l < 0.5 { gpui::white() } else { gpui::black() };
    let canvas = down(0.055);
    let accent_foreground = if accent.l > 0.55 { canvas } else { gpui::white() };
    let dim = mix(fg, bg, 0.58);
    let [_, red, green, yellow, blue, magenta, cyan, ..] = terminal.ansi;
    Theme {
        name: SharedString::from(name.to_string()),
        label: SharedString::from(label.to_string()),
        canvas,
        chrome: down(0.03),
        background: bg,
        surface: up(0.02),
        raised: up(0.05),
        popover: up(0.012),
        input: down(0.044),
        border: up(0.065),
        border_strong: up(0.15),
        hover: up(0.04),
        selected: up(0.08),
        foreground: fg,
        bright: mix(fg, far, 0.55),
        muted: mix(fg, bg, 0.25),
        dim,
        accent,
        accent_hover: shade(accent, k * 0.06),
        accent_foreground,
        solid: accent,
        solid_foreground: accent_foreground,
        ring: accent,
        brand: rgb(BRAND).into(),
        red,
        green,
        yellow,
        blue,
        magenta,
        cyan,
        orange: Hsla { h: 0.06, ..red },
        working: blue,
        blocked: yellow,
        done: green,
        idle: dim,
        error: red,
        unknown: dim,
        scrim: gpui::black().opacity(0.55),
        shadow: gpui::black().opacity(0.45),
        terminal,
    }
}

fn slot<'a>(t: &'a mut Theme, key: &str) -> Option<&'a mut Hsla> {
    Some(match key {
        "canvas" => &mut t.canvas,
        "chrome" => &mut t.chrome,
        "background" => &mut t.background,
        "surface" => &mut t.surface,
        "raised" => &mut t.raised,
        "popover" => &mut t.popover,
        "input" => &mut t.input,
        "border" => &mut t.border,
        "border_strong" => &mut t.border_strong,
        "hover" => &mut t.hover,
        "selected" => &mut t.selected,
        "foreground" => &mut t.foreground,
        "bright" => &mut t.bright,
        "muted" => &mut t.muted,
        "dim" => &mut t.dim,
        "accent" => &mut t.accent,
        "accent_hover" => &mut t.accent_hover,
        "accent_foreground" => &mut t.accent_foreground,
        "solid" => &mut t.solid,
        "solid_foreground" => &mut t.solid_foreground,
        "ring" => &mut t.ring,
        "brand" => &mut t.brand,
        "red" => &mut t.red,
        "green" => &mut t.green,
        "yellow" => &mut t.yellow,
        "blue" => &mut t.blue,
        "magenta" => &mut t.magenta,
        "cyan" => &mut t.cyan,
        "orange" => &mut t.orange,
        "working" => &mut t.working,
        "blocked" => &mut t.blocked,
        "done" => &mut t.done,
        "idle" => &mut t.idle,
        "error" => &mut t.error,
        "unknown" => &mut t.unknown,
        "scrim" => &mut t.scrim,
        "shadow" => &mut t.shadow,
        _ => return None,
    })
}

pub fn parse_theme(name: &str, text: &str) -> Option<Theme> {
    let json = parse_json(text)?;
    let background = color(&json, "background")?;
    let foreground = color(&json, "foreground")?;
    let ansi = ansi_of(&json, default_ansi());
    let terminal = TermColors {
        background,
        foreground,
        selection: color(&json, "selection").unwrap_or_else(|| mix(background, foreground, 0.22)),
        cursor: color(&json, "cursor").unwrap_or(foreground),
        ansi,
    };
    let empty = Json::Obj(Vec::new());
    let colors = field(&json, "colors").unwrap_or(&empty);
    let label = text_of(field(&json, "label"))
        .map(str::to_string)
        .unwrap_or_else(|| title_case(name));
    let mut theme = derive(
        name,
        &label,
        terminal,
        color(colors, "background").unwrap_or(background),
        color(colors, "foreground").unwrap_or(foreground),
        color(colors, "accent").unwrap_or(ansi[4]),
    );
    if let Json::Obj(fields) = colors {
        for (key, value) in fields {
            let parsed = text_of(Some(value)).and_then(parse_hex);
            if let (Some(c), Some(target)) = (parsed, slot(&mut theme, key)) {
                *target = c;
            }
        }
    }
    let follows = [
        ("solid", theme.accent),
        ("solid_foreground", theme.accent_foreground),
        ("ring", theme.accent),
        ("working", theme.blue),
        ("blocked", theme.yellow),
        ("done", theme.green),
        ("idle", theme.dim),
        ("error", theme.red),
        ("unknown", theme.dim),
    ];
    for (key, source) in follows {
        if color(colors, key).is_none()
            && let Some(target) = slot(&mut theme, key)
        {
            *target = source;
        }
    }
    Some(theme)
}

pub fn builtin() -> Vec<Theme> {
    BUILTIN
        .iter()
        .filter_map(|(name, text)| parse_theme(name, text))
        .collect()
}

pub fn default_theme() -> Theme {
    BUILTIN
        .iter()
        .find(|(name, _)| *name == DEFAULT_THEME)
        .and_then(|(name, text)| parse_theme(name, text))
        .unwrap_or_else(|| {
            let ansi = default_ansi();
            let bg: Hsla = rgb(0x21252b).into();
            let fg: Hsla = rgb(0x98c379).into();
            let terminal = TermColors {
                background: bg,
                foreground: fg,
                selection: rgb(0x3e4451).into(),
                cursor: fg,
                ansi,
            };
            derive(DEFAULT_THEME, "Atom Green", terminal, bg, fg, rgb(0x56b6c2).into())
        })
}

pub fn nixa_dark() -> Theme {
    default_theme()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}

pub fn user_themes(dir: &Path) -> Vec<Theme> {
    let Ok(entries) = fs::read_dir(dir.join(USER_DIR)) else {
        return Vec::new();
    };
    let mut found: Vec<Theme> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json") && p.is_file())
        .filter_map(|p| {
            let name = p.file_stem()?.to_str()?.to_string();
            if !valid_name(&name) {
                return None;
            }
            let meta = fs::metadata(&p).ok()?;
            if meta.len() > 256 * 1024 {
                return None;
            }
            parse_theme(&name, &fs::read_to_string(&p).ok()?)
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

pub fn all_themes(dir: &Path) -> Vec<Theme> {
    let mut list = builtin();
    for theme in user_themes(dir) {
        match list.iter_mut().find(|t| t.name == theme.name) {
            Some(old) => *old = theme,
            None => list.push(theme),
        }
    }
    list
}

pub fn find<'a>(list: &'a [Theme], name: &str) -> Option<&'a Theme> {
    list.iter().find(|t| t.name.as_ref() == name)
}

pub fn saved_choice(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(CHOICE_FILE)).ok()?;
    let json = parse_json(&text)?;
    text_of(field(&json, "theme"))
        .filter(|n| valid_name(n))
        .map(str::to_string)
}

pub fn save_choice(dir: &Path, name: &str) -> io::Result<()> {
    if !valid_name(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad theme name"));
    }
    paths::ensure_private_dir(dir)?;
    let path = dir.join(CHOICE_FILE);
    static SAVES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SAVES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp = dir.join(format!("{CHOICE_FILE}.tmp-{}-{n}", std::process::id()));
    fs::write(&tmp, format!("{{\n  \"theme\": \"{name}\"\n}}\n"))?;
    fs::rename(&tmp, &path)
}

pub fn pick(list: &[Theme], saved: Option<&str>) -> Theme {
    saved
        .and_then(|name| find(list, name))
        .or_else(|| find(list, DEFAULT_THEME))
        .or_else(|| list.first())
        .cloned()
        .unwrap_or_else(default_theme)
}

pub fn install(cx: &mut App) {
    install_from(cx, paths::data_dir());
}

pub fn install_from(cx: &mut App, dir: PathBuf) {
    let list = all_themes(&dir);
    let chosen = pick(&list, saved_choice(&dir).as_deref());
    cx.set_global(chosen);
    cx.set_global(Themes { dir, list });
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

pub fn themes(cx: &App) -> Vec<(SharedString, SharedString)> {
    cx.try_global::<Themes>()
        .map(|g| {
            g.list
                .iter()
                .map(|t| (t.name.clone(), t.label.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn reload(cx: &mut App) {
    let Some(dir) = cx.try_global::<Themes>().map(|g| g.dir.clone()) else {
        return;
    };
    let current = theme(cx).name.to_string();
    let list = all_themes(&dir);
    let chosen = pick(&list, Some(&current));
    cx.set_global(chosen);
    cx.set_global(Themes { dir, list });
    cx.refresh_windows();
}

pub fn apply(cx: &mut App, name: &str) -> bool {
    let Some(found) = cx
        .try_global::<Themes>()
        .and_then(|g| find(&g.list, name).cloned())
    else {
        return false;
    };
    cx.set_global(found);
    cx.refresh_windows();
    true
}

pub fn choose(cx: &mut App, name: &str) -> io::Result<bool> {
    if !apply(cx, name) {
        return Ok(false);
    }
    match cx.try_global::<Themes>().map(|g| g.dir.clone()) {
        Some(dir) => save_choice(&dir, name).map(|_| true),
        None => Ok(true),
    }
}

pub fn next_name(list: &[(SharedString, SharedString)], current: &str, step: isize) -> Option<SharedString> {
    if list.is_empty() {
        return None;
    }
    let len = list.len() as isize;
    let Some(at) = list.iter().position(|(n, _)| n.as_ref() == current) else {
        return Some(list[0].0.clone());
    };
    Some(list[(at as isize + step).rem_euclid(len) as usize].0.clone())
}

pub fn tint(color: Hsla, alpha: f32) -> Hsla {
    color.opacity(alpha)
}

pub fn status_color(t: &Theme, status: Status) -> Hsla {
    match status {
        Status::Working => t.working,
        Status::Blocked => t.blocked,
        Status::Done => t.done,
        Status::Idle => t.idle,
        Status::Error => t.error,
    }
}

pub fn status_label(status: Status) -> &'static str {
    match status {
        Status::Working => "Working",
        Status::Blocked => "Needs you",
        Status::Done => "Done",
        Status::Idle => "Idle",
        Status::Error => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nixa-theme-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn hx(v: u32) -> Hsla {
        rgb(v).into()
    }

    #[test]
    fn every_builtin_theme_parses_with_a_unique_name() {
        let list = builtin();
        assert_eq!(list.len(), BUILTIN.len());
        let mut names: Vec<&str> = list.iter().map(|t| t.name.as_ref()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BUILTIN.len());
        for name in ["one-dark-pro", "atom-one-dark", "atom-green", "graphite", "ink", "moss", "matrix"] {
            assert!(find(&list, name).is_some(), "{name}");
        }
        for t in &list {
            assert!(!t.label.is_empty());
            assert!(t.terminal.ansi.iter().all(|c| c.a > 0.0));
        }
    }

    #[test]
    fn default_theme_is_atom_green() {
        assert_eq!(DEFAULT_THEME, "atom-green");
        let t = default_theme();
        assert_eq!(t.name.as_ref(), "atom-green");
        assert_eq!(t.label.as_ref(), "Atom Green");
        assert_eq!(t.foreground, hx(0x98c379));
        assert_eq!(t.accent, hx(0x56b6c2));
        assert_eq!(t.terminal.background, hx(0x21252b));
        assert_eq!(nixa_dark().name.as_ref(), "atom-green");
    }

    #[test]
    fn one_dark_pro_keeps_its_look() {
        let list = builtin();
        let t = find(&list, "one-dark-pro").unwrap();
        assert_eq!(t.label.as_ref(), "One Dark Pro");
        let want = [
            (t.canvas, 0x1b1e24),
            (t.chrome, 0x21252b),
            (t.background, 0x282c34),
            (t.surface, 0x2c313a),
            (t.raised, 0x323844),
            (t.popover, 0x2a2f38),
            (t.input, 0x1e2228),
            (t.border, 0x363c47),
            (t.border_strong, 0x4a5160),
            (t.hover, 0x2f3540),
            (t.selected, 0x37404f),
            (t.foreground, 0xabb2bf),
            (t.bright, 0xdcdfe4),
            (t.muted, 0x8b929e),
            (t.dim, 0x5f6672),
            (t.accent, 0x61afef),
            (t.accent_hover, 0x7dbdf2),
            (t.accent_foreground, 0x1b1e24),
            (t.brand, 0x54d1e3),
            (t.red, 0xe06c75),
            (t.green, 0x98c379),
            (t.yellow, 0xe5c07b),
            (t.blue, 0x61afef),
            (t.magenta, 0xc678dd),
            (t.cyan, 0x56b6c2),
            (t.orange, 0xe3814f),
        ];
        for (i, (got, hex)) in want.into_iter().enumerate() {
            assert_eq!(got, hx(hex), "field {i}");
        }
        assert_eq!(t.terminal.selection, hx(0x334659));
        assert_eq!(t.terminal.cursor, hx(0x61afef));
        assert_eq!(t.terminal.ansi[15], hx(0xffffff));
        assert_eq!(t.working, t.blue);
        assert_eq!(t.blocked, t.yellow);
        assert_eq!(t.done, t.green);
        assert_eq!(t.idle, t.dim);
        assert_eq!(t.error, t.red);
    }

    #[test]
    fn atom_green_keeps_its_own_colors() {
        let list = builtin();
        let t = find(&list, "atom-green").unwrap();
        assert_eq!(t.foreground, hx(0x98c379));
        assert_eq!(t.muted, hx(0x61afef));
        assert_eq!(t.dim, hx(0xc678dd));
        assert_eq!(t.accent, hx(0x56b6c2));
        assert_eq!(t.solid, hx(0x98c379));
        assert_eq!(t.solid_foreground, hx(0x21252b));
        assert_eq!(t.ring, hx(0x61afef));
        assert_eq!(status_color(t, Status::Working), hx(0xe5c07b));
        assert_eq!(status_color(t, Status::Blocked), hx(0xe06c75));
        assert_eq!(status_color(t, Status::Done), hx(0x61afef));
        assert_eq!(status_color(t, Status::Idle), hx(0x56b6c2));
        assert_eq!(t.unknown, hx(0x767676));
        assert_eq!(t.terminal.background, hx(0x21252b));
        assert_eq!(t.terminal.ansi[8], hx(0x767676));
    }

    #[test]
    fn matrix_is_black_with_green_accent() {
        let list = builtin();
        let t = find(&list, "matrix").unwrap();
        assert_eq!(t.background, hx(0x000000));
        assert_eq!(t.canvas, hx(0x000000));
        assert_eq!(t.accent, hx(0x00ff41));
        assert_eq!(t.solid, t.accent);
        assert_eq!(t.terminal.cursor, hx(0x00ff41));
        assert!(t.raised.l > t.background.l);
        assert!(t.border.l > t.raised.l);
    }

    #[test]
    fn derived_theme_steps_away_from_the_background() {
        let dark = parse_theme("d", r##"{"background":"#202020","foreground":"#d0d0d0"}"##).unwrap();
        assert!(dark.canvas.l < dark.background.l);
        assert!(dark.raised.l > dark.background.l);
        assert!(dark.muted.l < dark.foreground.l);
        assert_eq!(dark.label.as_ref(), "D");
        let light = parse_theme("paper-white", r##"{"background":"#f4f4f4","foreground":"#202020"}"##).unwrap();
        assert!(light.canvas.l > light.background.l);
        assert!(light.raised.l < light.background.l);
        assert!(light.bright.l < light.foreground.l);
        assert_eq!(light.label.as_ref(), "Paper White");
        assert_eq!(light.terminal.ansi, default_ansi());
    }

    #[test]
    fn hex_forms() {
        assert_eq!(parse_hex("#abc"), Some(hx(0xaabbcc)));
        assert_eq!(parse_hex("#aabbcc"), Some(hx(0xaabbcc)));
        assert_eq!(parse_hex(" #aabbcc80 "), Some(rgba(0xaabbcc80).into()));
        assert_eq!(parse_hex("aabbcc"), None);
        assert_eq!(parse_hex("#ggg"), None);
        assert_eq!(parse_hex("#+12345"), None);
        assert_eq!(parse_hex("#12345"), None);
    }

    #[test]
    fn json_reader_handles_escapes_and_rejects_junk() {
        let j = parse_json(r#"{"a": "x\"y\u0041", "b": [1, true, null, "z"], "c": {"d": -1.5e3}}"#).unwrap();
        assert_eq!(text_of(field(&j, "a")), Some("x\"yA"));
        assert_eq!(field(&j, "b").map(|b| matches!(b, Json::Arr(v) if v.len() == 4)), Some(true));
        assert_eq!(field(field(&j, "c").unwrap(), "d"), Some(&Json::Other));
        assert!(parse_json(r#"{"a": "b"} x"#).is_none());
        assert!(parse_json(r#"{"a" "b"}"#).is_none());
        assert!(parse_json(r#"{"a": "b""#).is_none());
        assert!(parse_json(&"[".repeat(100)).is_none());
        assert!(parse_theme("x", r##"{"foreground":"#fff"}"##).is_none());
        assert!(parse_theme("x", "not json").is_none());
    }

    #[test]
    fn user_themes_add_and_replace_builtins() {
        let dir = temp_dir();
        let user = dir.join(USER_DIR);
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("matrix.json"), r##"{"background":"#010101","foreground":"#eeeeee"}"##).unwrap();
        fs::write(user.join("sunset.json"), r##"{"label":"Sunset","background":"#2a1b1b","foreground":"#f0d0c0","colors":{"accent":"#ff8844"}}"##).unwrap();
        fs::write(user.join("broken.json"), "{").unwrap();
        fs::write(user.join("bad name.json"), r##"{"background":"#000","foreground":"#fff"}"##).unwrap();
        fs::write(user.join("notes.txt"), "x").unwrap();
        let list = all_themes(&dir);
        assert_eq!(list.len(), BUILTIN.len() + 1);
        assert_eq!(find(&list, "matrix").unwrap().background, hx(0x010101));
        let sunset = find(&list, "sunset").unwrap();
        assert_eq!(sunset.label.as_ref(), "Sunset");
        assert_eq!(sunset.accent, hx(0xff8844));
        assert!(find(&list, "broken").is_none());
        assert!(find(&list, "bad name").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn choice_is_saved_and_read_back() {
        let dir = temp_dir();
        assert_eq!(saved_choice(&dir), None);
        save_choice(&dir, "moss").unwrap();
        assert_eq!(saved_choice(&dir).as_deref(), Some("moss"));
        save_choice(&dir, "ink").unwrap();
        assert_eq!(saved_choice(&dir).as_deref(), Some("ink"));
        assert!(save_choice(&dir, "../evil").is_err());
        assert!(save_choice(&dir, "").is_err());
        let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from(CHOICE_FILE)]);
        fs::write(dir.join(CHOICE_FILE), r#"{"theme": "../x"}"#).unwrap();
        assert_eq!(saved_choice(&dir), None);
        let nested = dir.join("a").join("b");
        save_choice(&nested, "graphite").unwrap();
        assert_eq!(saved_choice(&nested).as_deref(), Some("graphite"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saves_from_many_threads_leave_one_good_file() {
        let dir = temp_dir();
        let names = ["ink", "moss", "graphite", "matrix"];
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || save_choice(&dir, names[i % names.len()]))
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
        let got = saved_choice(&dir).unwrap();
        assert!(names.contains(&got.as_str()));
        let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from(CHOICE_FILE)]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pick_falls_back_to_the_default() {
        let list = builtin();
        assert_eq!(pick(&list, Some("ink")).name.as_ref(), "ink");
        assert_eq!(pick(&list, Some("one-dark-pro")).name.as_ref(), "one-dark-pro");
        assert_eq!(pick(&list, Some("gone")).name.as_ref(), DEFAULT_THEME);
        assert_eq!(pick(&list, None).name.as_ref(), DEFAULT_THEME);
        assert_eq!(pick(&[], Some("ink")).name.as_ref(), DEFAULT_THEME);
    }

    #[test]
    fn next_name_wraps_both_ways() {
        let list: Vec<(SharedString, SharedString)> = builtin().iter().map(|t| (t.name.clone(), t.label.clone())).collect();
        let first = list[0].0.as_ref();
        let last = list[list.len() - 1].0.as_ref();
        let name = |n: Option<SharedString>| n.map(|n| n.to_string());
        assert_eq!(name(next_name(&list, last, 1)).as_deref(), Some(first));
        assert_eq!(name(next_name(&list, first, -1)).as_deref(), Some(last));
        assert_eq!(name(next_name(&list, "gone", 1)).as_deref(), Some(first));
        assert_eq!(name(next_name(&list, "gone", -1)).as_deref(), Some(first));
        assert_eq!(next_name(&[], "x", 1), None);
    }

    #[test]
    fn every_status_has_a_color_and_label() {
        for t in builtin() {
            for status in Status::ALL {
                assert!(status_color(&t, status).a > 0.0);
                assert!(!status_label(status).is_empty());
            }
        }
    }

    #[test]
    fn tint_keeps_hue() {
        let t = nixa_dark();
        let c = tint(t.yellow, 0.15);
        assert_eq!(c.h, t.yellow.h);
        assert!((c.a - 0.15).abs() < 1e-6);
    }

    #[test]
    fn mix_ends() {
        let a = hx(0x000000);
        let b = hx(0xffffff);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }
}
