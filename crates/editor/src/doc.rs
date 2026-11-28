pub const BOM: char = '\u{feff}';
const SCAN_LINES: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    Crlf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub disk: Option<String>,
    pub from_save: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub disk: Option<String>,
    pub base: String,
    pub eol: Eol,
    pub bom: bool,
    pub indent: Indent,
    pub conflict: Option<Conflict>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskAction {
    None,
    Settled,
    Adopt,
    Reload,
    Conflict,
    Deleted,
}

impl Eol {
    pub fn label(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::Crlf => "CRLF",
        }
    }
}

impl Indent {
    pub fn unit(self) -> String {
        match self {
            Indent::Tabs => "\t".into(),
            Indent::Spaces(n) => " ".repeat(n.max(1)),
        }
    }

    pub fn label(self) -> String {
        match self {
            Indent::Tabs => "Tabs".into(),
            Indent::Spaces(1) => "1 space".into(),
            Indent::Spaces(n) => format!("{n} spaces"),
        }
    }
}

pub fn detect_eol(text: &str) -> Eol {
    let bytes = text.as_bytes();
    let (mut crlf, mut lf) = (0usize, 0usize);
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
    }
    if crlf > lf { Eol::Crlf } else { Eol::Lf }
}

pub fn to_lf(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

pub fn with_eol(text: &str, eol: Eol) -> String {
    match eol {
        Eol::Lf => text.to_string(),
        Eol::Crlf => text.replace('\n', "\r\n"),
    }
}

pub fn detect_indent(text: &str, fallback: Indent) -> Indent {
    let (mut tabs, mut spaced) = (0usize, 0usize);
    let mut steps = [0usize; 9];
    let mut last = 0usize;
    for line in text.split('\n').take(SCAN_LINES) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let width = line.len() - line.trim_start_matches(' ').len();
        if width > 0 {
            spaced += 1;
        }
        if width > last && (2..=8).contains(&(width - last)) {
            steps[width - last] += 1;
        }
        last = width;
    }
    if tabs > spaced {
        return Indent::Tabs;
    }
    if spaced == 0 {
        return fallback;
    }
    let mut best = 0usize;
    for step in 2..=8 {
        if steps[step] > steps[best] {
            best = step;
        }
    }
    if best == 0 { fallback } else { Indent::Spaces(best) }
}

pub fn open(raw: &str) -> Doc {
    let bom = raw.starts_with(BOM);
    let body = if bom { &raw[BOM.len_utf8()..] } else { raw };
    Doc {
        disk: Some(raw.to_string()),
        base: to_lf(body),
        eol: detect_eol(body),
        bom,
        indent: detect_indent(body, Indent::Spaces(2)),
        conflict: None,
    }
}

pub fn disk_text(doc: &Doc, value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 4);
    if doc.bom {
        out.push(BOM);
    }
    out.push_str(&with_eol(&to_lf(value), doc.eol));
    out
}

pub fn is_missing(doc: &Doc) -> bool {
    doc.disk.is_none()
}

pub fn is_dirty(doc: &Doc, value: &str) -> bool {
    is_missing(doc) || value != doc.base
}

pub fn on_disk(doc: &Doc, raw: Option<&str>, value: &str) -> (Doc, DiskAction) {
    if raw == doc.disk.as_deref() {
        if doc.conflict.is_some() {
            return (Doc { conflict: None, ..doc.clone() }, DiskAction::Settled);
        }
        return (doc.clone(), DiskAction::None);
    }
    if let Some(raw) = raw {
        let fresh = open(raw);
        if fresh.base == value {
            return (Doc { eol: doc.eol, bom: doc.bom, ..fresh }, DiskAction::Adopt);
        }
        if !is_dirty(doc, value) {
            return (fresh, DiskAction::Reload);
        }
    }
    let conflict = Conflict { disk: raw.map(str::to_string), from_save: false };
    let action = if raw.is_none() { DiskAction::Deleted } else { DiskAction::Conflict };
    (Doc { conflict: Some(conflict), ..doc.clone() }, action)
}

pub fn reload(doc: &Doc) -> Option<Doc> {
    doc.conflict.as_ref()?.disk.as_deref().map(open)
}

pub fn keep_mine(doc: &Doc) -> Doc {
    match doc.conflict.as_ref().and_then(|c| c.disk.clone()) {
        None => Doc { disk: None, conflict: None, ..doc.clone() },
        Some(disk) => Doc { base: open(&disk).base, disk: Some(disk), conflict: None, ..doc.clone() },
    }
}

pub fn saved(doc: &Doc, text: String, value: &str) -> Doc {
    Doc { disk: Some(text), base: value.to_string(), conflict: None, ..doc.clone() }
}

pub fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if f < KB * KB {
        if f < 10.0 * KB { format!("{:.1} KB", f / KB) } else { format!("{:.0} KB", f / KB) }
    } else {
        format!("{:.1} MB", f / KB / KB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eol_counts_both_kinds() {
        assert_eq!(detect_eol("a\r\nb\r\nc\n"), Eol::Crlf);
        assert_eq!(detect_eol("a\nb\r\nc\n"), Eol::Lf);
        assert_eq!(detect_eol("one line"), Eol::Lf);
    }

    #[test]
    fn lf_and_back() {
        assert_eq!(to_lf("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(with_eol("a\nb", Eol::Crlf), "a\r\nb");
    }

    #[test]
    fn indent_guess() {
        assert_eq!(detect_indent("fn a() {\n    x;\n    if y {\n        z;\n    }\n}\n", Indent::Spaces(2)), Indent::Spaces(4));
        assert_eq!(detect_indent("a:\n  b:\n    c: 1\n", Indent::Spaces(4)), Indent::Spaces(2));
        assert_eq!(detect_indent("a\n\tb\n\tc\n", Indent::Spaces(2)), Indent::Tabs);
        assert_eq!(detect_indent("a\nb\n", Indent::Spaces(3)), Indent::Spaces(3));
        assert_eq!(Indent::Spaces(4).label(), "4 spaces");
        assert_eq!(Indent::Tabs.unit(), "\t");
    }

    #[test]
    fn bom_and_crlf_are_kept_on_save() {
        let raw = "\u{feff}a\r\nb\r\n";
        let doc = open(raw);
        assert!(doc.bom);
        assert_eq!(doc.base, "a\nb\n");
        assert_eq!(disk_text(&doc, "a\nb\nc\n"), "\u{feff}a\r\nb\r\nc\r\n");
        assert!(!is_dirty(&doc, "a\nb\n"));
        assert!(is_dirty(&doc, "a\n"));
    }

    #[test]
    fn disk_change_rules() {
        let doc = open("one\n");
        let (_, a) = on_disk(&doc, Some("one\n"), "one\n");
        assert_eq!(a, DiskAction::None);
        let (d, a) = on_disk(&doc, Some("two\n"), "one\n");
        assert_eq!((a, d.base.as_str()), (DiskAction::Reload, "two\n"));
        let (_, a) = on_disk(&doc, Some("mine\n"), "mine\n");
        assert_eq!(a, DiskAction::Adopt);
        let (d, a) = on_disk(&doc, Some("two\n"), "mine\n");
        assert_eq!(a, DiskAction::Conflict);
        assert_eq!(reload(&d).map(|d| d.base), Some("two\n".to_string()));
        let kept = keep_mine(&d);
        assert_eq!(kept.disk.as_deref(), Some("two\n"));
        assert!(is_dirty(&kept, "mine\n"));
        let (d, a) = on_disk(&doc, None, "one\n");
        assert_eq!(a, DiskAction::Deleted);
        let kept = keep_mine(&d);
        assert!(is_missing(&kept));
        assert!(is_dirty(&kept, "one\n"));
        let (back, a) = on_disk(&d, Some("one\n"), "one\n");
        assert_eq!(a, DiskAction::Settled);
        assert!(back.conflict.is_none());
    }

    #[test]
    fn adopt_keeps_line_endings() {
        let doc = open("a\r\n");
        let (d, a) = on_disk(&doc, Some("b\n"), "b\n");
        assert_eq!(a, DiskAction::Adopt);
        assert_eq!(d.eol, Eol::Crlf);
    }

    #[test]
    fn bytes_read_well() {
        assert_eq!(format_bytes(10), "10 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(20480), "20 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
