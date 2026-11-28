use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Markers {
    pub open: String,
    pub base: Option<String>,
    pub separator: String,
    pub close: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub ours: String,
    pub base: Option<String>,
    pub theirs: String,
    pub ours_label: String,
    pub base_label: Option<String>,
    pub theirs_label: String,
    pub ours_lines: Vec<String>,
    pub base_lines: Option<Vec<String>>,
    pub theirs_lines: Vec<String>,
    pub markers: Markers,
    pub crlf: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    Text(Vec<String>),
    Conflict(Box<Conflict>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Choice {
    Ours,
    Theirs,
    Both,
    Base,
    Text(String),
}

fn marker(line: &str, sign: char) -> bool {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let head: String = line.chars().take(7).collect();
    if head.chars().count() != 7 || head.chars().any(|c| c != sign) {
        return false;
    }
    let rest = &line[7..];
    rest.is_empty() || rest.starts_with(' ')
}

fn is_open(line: &str) -> bool {
    marker(line, '<')
}

fn is_base(line: &str) -> bool {
    marker(line, '|')
}

fn is_close(line: &str) -> bool {
    marker(line, '>')
}

fn is_separator(line: &str) -> bool {
    line.strip_suffix('\r').unwrap_or(line) == "======="
}

fn strip_cr(line: &str) -> String {
    line.strip_suffix('\r').unwrap_or(line).to_string()
}

fn join_clean(lines: &[String]) -> String {
    lines.iter().map(|l| strip_cr(l)).collect::<Vec<_>>().join("\n")
}

fn label(line: &str) -> String {
    line.get(7..).unwrap_or("").trim().to_string()
}

fn read_conflict(lines: &[&str], start: usize) -> (usize, Option<Conflict>) {
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    let mut base: Option<Vec<String>> = None;
    let mut base_marker = None;
    let mut separator = String::new();
    let mut section = 0;
    let mut i = start + 1;
    while i < lines.len() {
        let line = lines[i];
        if section != 2 && is_open(line) {
            return (i, None);
        }
        if section != 2 && is_close(line) {
            return (i + 1, None);
        }
        if section == 0 && is_base(line) {
            section = 1;
            base = Some(Vec::new());
            base_marker = Some(line.to_string());
        } else if section != 2 && is_separator(line) {
            section = 2;
            separator = line.to_string();
        } else if section == 2 && is_close(line) {
            let open = lines[start].to_string();
            let conflict = Conflict {
                ours: join_clean(&ours),
                base: base.as_ref().map(|b| join_clean(b)),
                theirs: join_clean(&theirs),
                ours_label: label(&open),
                base_label: base_marker.as_deref().map(label),
                theirs_label: label(line),
                crlf: open.ends_with('\r'),
                markers: Markers { open, base: base_marker, separator, close: line.to_string() },
                ours_lines: ours,
                base_lines: base,
                theirs_lines: theirs,
                line: start + 1,
            };
            return (i + 1, Some(conflict));
        } else if section == 0 {
            ours.push(line.to_string());
        } else if section == 1 {
            if let Some(b) = base.as_mut() {
                b.push(line.to_string());
            }
        } else {
            theirs.push(line.to_string());
        }
        i += 1;
    }
    (i, None)
}

pub fn parse_conflicts(content: &str) -> Vec<Part> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut parts = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !is_open(lines[i]) {
            text.push(lines[i].to_string());
            i += 1;
            continue;
        }
        let (end, part) = read_conflict(&lines, i);
        match part {
            Some(c) => {
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Conflict(Box::new(c)));
            }
            None => text.extend(lines[i..end].iter().map(|l| l.to_string())),
        }
        i = end;
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    parts
}

pub fn conflicts_of(parts: &[Part]) -> Vec<&Conflict> {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::Conflict(c) => Some(c.as_ref()),
            Part::Text(_) => None,
        })
        .collect()
}

pub fn conflict_count(parts: &[Part]) -> usize {
    conflicts_of(parts).len()
}

pub fn has_markers(text: &str) -> bool {
    text.split('\n').any(|l| is_open(l) || is_close(l))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Starts {
    pub ours: usize,
    pub theirs: usize,
}

pub fn side_starts(parts: &[Part]) -> Vec<Starts> {
    let mut starts = Vec::new();
    let (mut ours, mut theirs) = (1, 1);
    for part in parts {
        match part {
            Part::Text(lines) => {
                ours += lines.len();
                theirs += lines.len();
            }
            Part::Conflict(c) => {
                starts.push(Starts { ours, theirs });
                ours += c.ours_lines.len();
                theirs += c.theirs_lines.len();
            }
        }
    }
    starts
}

fn text_lines(text: &str, crlf: bool) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(|l| if crlf { format!("{}\r", strip_cr(l)) } else { strip_cr(l) }).collect()
}

fn unresolved_lines(c: &Conflict) -> Vec<String> {
    let mut out = vec![c.markers.open.clone()];
    out.extend(c.ours_lines.iter().cloned());
    if let Some(base) = &c.markers.base {
        out.push(base.clone());
        out.extend(c.base_lines.clone().unwrap_or_default());
    }
    out.push(c.markers.separator.clone());
    out.extend(c.theirs_lines.iter().cloned());
    out.push(c.markers.close.clone());
    out
}

pub fn resolved_lines(c: &Conflict, choice: Option<&Choice>) -> Vec<String> {
    match choice {
        Some(Choice::Text(text)) => text_lines(text, c.crlf),
        Some(Choice::Ours) => c.ours_lines.clone(),
        Some(Choice::Theirs) => c.theirs_lines.clone(),
        Some(Choice::Both) => c.ours_lines.iter().chain(c.theirs_lines.iter()).cloned().collect(),
        Some(Choice::Base) if c.base_lines.is_some() => c.base_lines.clone().unwrap_or_default(),
        _ => unresolved_lines(c),
    }
}

pub fn is_chosen(c: &Conflict, choice: Option<&Choice>) -> bool {
    match choice {
        Some(Choice::Base) => c.base_lines.is_some(),
        Some(_) => true,
        None => false,
    }
}

pub fn resolve_text(parts: &[Part], choices: &[Option<Choice>]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut index = 0;
    for part in parts {
        match part {
            Part::Text(lines) => out.extend(lines.iter().cloned()),
            Part::Conflict(c) => {
                out.extend(resolved_lines(c, choices.get(index).and_then(|x| x.as_ref())));
                index += 1;
            }
        }
    }
    out.join("\n")
}

pub fn all_chosen(parts: &[Part], choices: &[Option<Choice>]) -> bool {
    conflicts_of(parts).iter().enumerate().all(|(i, c)| is_chosen(c, choices.get(i).and_then(|x| x.as_ref())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "top\n<<<<<<< HEAD\nmine\n||||||| base\nold\n=======\ntheirs\n>>>>>>> side\nmid\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> side\nend\n";

    #[test]
    fn parses_two_conflicts_with_base() {
        let parts = parse_conflicts(FILE);
        assert_eq!(conflict_count(&parts), 2);
        let list = conflicts_of(&parts);
        assert_eq!(list[0].ours, "mine");
        assert_eq!(list[0].base.as_deref(), Some("old"));
        assert_eq!(list[0].ours_label, "HEAD");
        assert_eq!(list[0].theirs_label, "side");
        assert_eq!(list[0].line, 2);
        assert_eq!(list[1].base, None);
        assert_eq!(side_starts(&parts), vec![Starts { ours: 2, theirs: 2 }, Starts { ours: 4, theirs: 4 }]);
        assert!(has_markers(FILE));
        assert!(!has_markers("a\n======= \nb"));
    }

    #[test]
    fn resolves_by_choice() {
        let parts = parse_conflicts(FILE);
        let none = resolve_text(&parts, &[]);
        assert_eq!(none, FILE);
        let text = resolve_text(&parts, &[Some(Choice::Theirs), Some(Choice::Both)]);
        assert_eq!(text, "top\ntheirs\nmid\na\nb\nend\n");
        let base = resolve_text(&parts, &[Some(Choice::Base), Some(Choice::Text("x\ny".into()))]);
        assert_eq!(base, "top\nold\nmid\nx\ny\nend\n");
        assert!(!all_chosen(&parts, &[Some(Choice::Ours)]));
        assert!(!all_chosen(&parts, &[Some(Choice::Ours), Some(Choice::Base)]));
        assert!(all_chosen(&parts, &[Some(Choice::Ours), Some(Choice::Ours)]));
    }

    #[test]
    fn crlf_and_broken_markers() {
        let parts = parse_conflicts("<<<<<<< a\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\n");
        let c = conflicts_of(&parts)[0];
        assert!(c.crlf);
        assert_eq!(c.ours, "x");
        assert_eq!(resolve_text(&parts, &[Some(Choice::Text("z".into()))]), "z\r\n");
        let broken = parse_conflicts("<<<<<<< a\nx\n");
        assert_eq!(conflict_count(&broken), 0);
    }
}
