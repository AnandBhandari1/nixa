#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Bug,
    Risk,
    Nit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Success,
    Danger,
    Attention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Bullet,
    Paragraph,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
    pub severity: Option<Severity>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub label: Option<String>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub text: String,
    pub tone: Tone,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub sections: Vec<Section>,
    pub structured: bool,
    pub verdict: Option<Verdict>,
}

const DANGER: &[&str] = &["needs changes", "not safe", "do not merge", "don't merge", "blocker", "must fix"];
const SAFE: &[&str] = &["looks safe", "safe to merge", "looks good", "good to go", "lgtm", "no issues"];

pub fn verdict_tone(text: &str) -> Tone {
    let lower = text.to_lowercase();
    if DANGER.iter().any(|w| lower.contains(w)) {
        Tone::Danger
    } else if SAFE.iter().any(|w| lower.contains(w)) {
        Tone::Success
    } else {
        Tone::Attention
    }
}

pub fn label_of(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("**")?;
    let end = rest.find(['*', '\n'])?;
    let label = &rest[..end];
    let tail = rest[end..].strip_prefix("**")?;
    let tail = tail.strip_prefix(':').unwrap_or(tail);
    (!label.is_empty() && tail.trim().is_empty()).then(|| label.trim().to_string())
}

fn bullet_of(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let mut chars = rest.chars();
    let first = chars.next()?;
    if !matches!(first, '-' | '*' | '•') {
        return None;
    }
    let after = chars.as_str();
    after.starts_with(char::is_whitespace).then(|| after.trim_start())
}

fn severity_of(body: &str) -> Option<(Severity, &str)> {
    for (word, kind) in [("bug", Severity::Bug), ("risk", Severity::Risk), ("nit", Severity::Nit)] {
        let Some(head) = body.get(..word.len()) else {
            continue;
        };
        if !head.eq_ignore_ascii_case(word) {
            continue;
        }
        let rest = &body[word.len()..];
        if rest.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix([':', '|', '—', '–', '-'])
            .unwrap_or(rest)
            .trim_start();
        return Some((kind, rest));
    }
    None
}

pub fn parse_report(text: &str) -> Report {
    let mut sections: Vec<Section> = Vec::new();
    let mut current = Section::default();
    let mut after_blank = true;
    for raw in text.split('\n') {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            after_blank = true;
            continue;
        }
        if let Some(label) = label_of(line) {
            if !current.blocks.is_empty() || current.label.is_some() {
                sections.push(std::mem::take(&mut current));
            }
            current.label = Some(label);
            after_blank = true;
            continue;
        }
        if let Some(body) = bullet_of(line) {
            let body = body.trim();
            let (text, severity) = match severity_of(body) {
                Some((kind, rest)) if !rest.trim().is_empty() => (rest.trim().to_string(), Some(kind)),
                Some((kind, _)) => (body.to_string(), Some(kind)),
                None => (body.to_string(), None),
            };
            current.blocks.push(Block {
                kind: BlockKind::Bullet,
                text,
                severity,
            });
        } else if let Some(last) = current.blocks.last_mut().filter(|_| !after_blank) {
            let joiner = if last.kind == BlockKind::Bullet { " " } else { "\n" };
            last.text = format!("{}{joiner}{}", last.text, line.trim());
        } else {
            current.blocks.push(Block {
                kind: BlockKind::Paragraph,
                text: line.trim().to_string(),
                severity: None,
            });
        }
        after_blank = false;
    }
    if !current.blocks.is_empty() || current.label.is_some() {
        sections.push(current);
    }
    let structured = sections.iter().any(|s| s.label.is_some());
    let verdict_text = sections
        .iter()
        .find(|s| s.label.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("verdict")))
        .map(|s| s.blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join(" ").trim().to_string())
        .unwrap_or_default();
    Report {
        sections: sections.into_iter().filter(|s| !s.blocks.is_empty()).collect(),
        structured,
        verdict: (!verdict_text.is_empty()).then(|| Verdict {
            tone: verdict_tone(&verdict_text),
            text: verdict_text,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REVIEW: &str = "**Summary**\nAdds a login check.\nIt also logs.\n\n**Findings**\n- Bug: `if x = 1` assigns\n  instead of compares.\n- Risk | slow loop\n- nit\n- Plain point\n**Verdict**:\nNeeds changes\n";

    #[test]
    fn parses_sections_bullets_and_verdict() {
        let r = parse_report(REVIEW);
        assert!(r.structured);
        assert_eq!(r.sections.len(), 3);
        assert_eq!(r.sections[0].label.as_deref(), Some("Summary"));
        assert_eq!(r.sections[0].blocks[0].text, "Adds a login check.\nIt also logs.");
        let f = &r.sections[1].blocks;
        assert_eq!(f[0].severity, Some(Severity::Bug));
        assert_eq!(f[0].text, "`if x = 1` assigns instead of compares.");
        assert_eq!(f[1].severity, Some(Severity::Risk));
        assert_eq!(f[1].text, "slow loop");
        assert_eq!(f[2].text, "nit");
        assert_eq!(f[2].severity, Some(Severity::Nit));
        assert_eq!(f[3].severity, None);
        let v = r.verdict.unwrap();
        assert_eq!(v.text, "Needs changes");
        assert_eq!(v.tone, Tone::Danger);
    }

    #[test]
    fn plain_text_is_not_structured() {
        let r = parse_report("Just one answer.\nSecond line.\n\nNew para.");
        assert!(!r.structured);
        assert_eq!(r.sections[0].blocks.len(), 2);
        assert_eq!(r.verdict, None);
    }

    #[test]
    fn labels_need_the_whole_line() {
        assert_eq!(label_of("**Verdict**:").as_deref(), Some("Verdict"));
        assert_eq!(label_of("  **A b**  ").as_deref(), Some("A b"));
        assert_eq!(label_of("**A** text"), None);
        assert_eq!(label_of("****"), None);
        assert_eq!(severity_of("bugfix here"), None);
    }

    #[test]
    fn tones() {
        assert_eq!(verdict_tone("Looks safe"), Tone::Success);
        assert_eq!(verdict_tone("LGTM but must fix tests"), Tone::Danger);
        assert_eq!(verdict_tone("Needs a closer look at auth"), Tone::Attention);
    }
}
