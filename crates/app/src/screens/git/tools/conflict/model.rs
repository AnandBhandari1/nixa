use crate::screens::git::tools::state::{Op, RepoState};

pub const MAX_SIDE_LINES: usize = 300;
pub const MAX_AI_CHARS: usize = 12_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    Both,
    Base,
}

impl Choice {
    pub fn id(self) -> &'static str {
        match self {
            Choice::Ours => "ours",
            Choice::Theirs => "theirs",
            Choice::Both => "both",
            Choice::Base => "base",
        }
    }

    pub fn button(self) -> &'static str {
        match self {
            Choice::Ours => "Use current",
            Choice::Theirs => "Use incoming",
            Choice::Both => "Use both",
            Choice::Base => "Use base",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Choice::Ours => "Keep the Current (HEAD) lines",
            Choice::Theirs => "Keep the Incoming lines",
            Choice::Both => "Keep Current first, then Incoming",
            Choice::Base => "Keep the lines from before both changes",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Choice::Ours => "Using current",
            Choice::Theirs => "Using incoming",
            Choice::Both => "Using both",
            Choice::Base => "Using base",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conflict {
    pub ours_lines: Vec<String>,
    pub base_lines: Option<Vec<String>>,
    pub theirs_lines: Vec<String>,
    pub ours_label: String,
    pub base_label: Option<String>,
    pub theirs_label: String,
    pub open: String,
    pub base_marker: Option<String>,
    pub separator: String,
    pub close: String,
    pub crlf: bool,
    pub line: usize,
}

impl Conflict {
    pub fn ours(&self) -> String {
        join_clean(&self.ours_lines)
    }

    pub fn theirs(&self) -> String {
        join_clean(&self.theirs_lines)
    }

    pub fn choices(&self) -> Vec<Choice> {
        let mut list = vec![Choice::Ours, Choice::Theirs, Choice::Both];
        if self.base_lines.is_some() {
            list.push(Choice::Base);
        }
        list
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Text(Vec<String>),
    Conflict(Box<Conflict>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub parts: Vec<Part>,
}

fn rest_ok(rest: &str) -> bool {
    rest.is_empty() || rest == "\r" || rest.starts_with(' ')
}

fn marker(line: &str, ch: char) -> bool {
    let mark: String = std::iter::repeat_n(ch, 7).collect();
    line.strip_prefix(mark.as_str()).is_some_and(rest_ok)
}

pub fn is_open(line: &str) -> bool {
    marker(line, '<')
}

pub fn is_base(line: &str) -> bool {
    marker(line, '|')
}

pub fn is_close(line: &str) -> bool {
    marker(line, '>')
}

pub fn is_separator(line: &str) -> bool {
    line == "=======" || line == "=======\r"
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

pub fn join_clean(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| strip_cr(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn label_of(line: &str) -> String {
    line.get(7..).unwrap_or_default().trim().to_string()
}

#[derive(PartialEq)]
enum Section {
    Ours,
    Base,
    Theirs,
}

fn read_conflict(lines: &[&str], start: usize) -> (usize, Option<Conflict>) {
    let mut c = Conflict {
        open: lines[start].to_string(),
        line: start + 1,
        crlf: lines[start].ends_with('\r'),
        ours_label: label_of(lines[start]),
        ..Conflict::default()
    };
    let mut section = Section::Ours;
    let mut i = start + 1;
    while i < lines.len() {
        let line = lines[i];
        if section != Section::Theirs && is_open(line) {
            return (i, None);
        }
        if section != Section::Theirs && is_close(line) {
            return (i + 1, None);
        }
        if section == Section::Ours && is_base(line) {
            section = Section::Base;
            c.base_lines = Some(Vec::new());
            c.base_marker = Some(line.to_string());
            c.base_label = Some(label_of(line));
        } else if section != Section::Theirs && is_separator(line) {
            section = Section::Theirs;
            c.separator = line.to_string();
        } else if section == Section::Theirs && is_close(line) {
            c.close = line.to_string();
            c.theirs_label = label_of(line);
            return (i + 1, Some(c));
        } else {
            match section {
                Section::Ours => c.ours_lines.push(line.to_string()),
                Section::Base => c
                    .base_lines
                    .get_or_insert_with(Vec::new)
                    .push(line.to_string()),
                Section::Theirs => c.theirs_lines.push(line.to_string()),
            }
        }
        i += 1;
    }
    (i, None)
}

pub fn parse_conflicts(content: &str) -> Parsed {
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
    Parsed { parts }
}

pub fn conflicts_of(parsed: &Parsed) -> Vec<Conflict> {
    parsed
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Conflict(c) => Some((**c).clone()),
            Part::Text(_) => None,
        })
        .collect()
}

pub fn has_markers(text: &str) -> bool {
    text.split('\n').any(|l| is_open(l) || is_close(l))
}

pub fn side_starts(parsed: &Parsed) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let (mut ours, mut theirs) = (1, 1);
    for part in &parsed.parts {
        match part {
            Part::Text(lines) => {
                ours += lines.len();
                theirs += lines.len();
            }
            Part::Conflict(c) => {
                starts.push((ours, theirs));
                ours += c.ours_lines.len();
                theirs += c.theirs_lines.len();
            }
        }
    }
    starts
}

fn unresolved(c: &Conflict) -> Vec<String> {
    let mut out = vec![c.open.clone()];
    out.extend(c.ours_lines.iter().cloned());
    if let (Some(marker), Some(base)) = (&c.base_marker, &c.base_lines) {
        out.push(marker.clone());
        out.extend(base.iter().cloned());
    }
    out.push(c.separator.clone());
    out.extend(c.theirs_lines.iter().cloned());
    out.push(c.close.clone());
    out
}

pub fn resolved_lines(c: &Conflict, choice: Option<Choice>) -> Vec<String> {
    match choice {
        Some(Choice::Ours) => c.ours_lines.clone(),
        Some(Choice::Theirs) => c.theirs_lines.clone(),
        Some(Choice::Both) => c
            .ours_lines
            .iter()
            .chain(&c.theirs_lines)
            .cloned()
            .collect(),
        Some(Choice::Base) if c.base_lines.is_some() => c.base_lines.clone().unwrap_or_default(),
        _ => unresolved(c),
    }
}

pub fn is_chosen(c: &Conflict, choice: Option<Choice>) -> bool {
    match choice {
        Some(Choice::Base) => c.base_lines.is_some(),
        Some(_) => true,
        None => false,
    }
}

pub fn resolve_text(parsed: &Parsed, choices: &[Option<Choice>]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut index = 0;
    for part in &parsed.parts {
        match part {
            Part::Text(lines) => out.extend(lines.iter().cloned()),
            Part::Conflict(c) => {
                out.extend(resolved_lines(c, choices.get(index).copied().flatten()));
                index += 1;
            }
        }
    }
    out.join("\n")
}

pub fn for_area(text: &str) -> String {
    text.replace("\r\n", "\n")
        .trim_end_matches('\r')
        .to_string()
}

pub fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

pub fn toggle(choices: &mut [Option<Choice>], index: usize, choice: Choice) {
    if let Some(slot) = choices.get_mut(index) {
        *slot = if *slot == Some(choice) {
            None
        } else {
            Some(choice)
        };
    }
}

pub fn join_sides(conflicts: &[Conflict], ours: bool) -> String {
    let side = |c: &Conflict| if ours { c.ours() } else { c.theirs() };
    if conflicts.len() == 1 {
        return side(&conflicts[0]);
    }
    conflicts
        .iter()
        .enumerate()
        .map(|(i, c)| format!("--- conflict {} (line {}) ---\n{}", i + 1, c.line, side(c)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn op_title(state: Option<&RepoState>) -> Option<String> {
    let state = state?;
    Some(match state.op? {
        Op::Merge => "Merging".into(),
        Op::Rebase => match state.step {
            Some((n, total)) => format!("Rebasing step {n} of {total}"),
            None => "Rebasing".into(),
        },
        Op::CherryPick => "Cherry-picking".into(),
        Op::Revert => "Reverting".into(),
    })
}

pub fn side_hint(op: Option<Op>) -> &'static str {
    match op {
        Some(Op::Rebase) => {
            "In a rebase, Current (HEAD) is the branch you are rebasing onto. Incoming is your own commit being replayed on top."
        }
        Some(Op::CherryPick) => {
            "Current (HEAD) is your branch as it is now. Incoming is the commit being picked."
        }
        Some(Op::Revert) => {
            "Current (HEAD) is your branch as it is now. Incoming is what the revert wants to undo."
        }
        _ => "Current (HEAD) is the branch you are on. Incoming is the branch being merged in.",
    }
}

pub fn finish_hint(op: Option<Op>) -> String {
    match op {
        None => "Commit to finish.".into(),
        Some(op) => format!(
            "Press Continue in the Changes panel to finish the {}.",
            op.word()
        ),
    }
}

pub struct Status<'a> {
    pub loading: bool,
    pub done: bool,
    pub conflicts: usize,
    pub chosen: usize,
    pub original: &'a str,
    pub hand_edited: bool,
    pub result: &'a str,
}

pub fn result_status(s: &Status) -> String {
    if s.loading {
        return "Loading…".into();
    }
    if s.done {
        return "Saved.".into();
    }
    if s.conflicts == 0 {
        return if has_markers(s.original) {
            "Conflict markers look broken here. Edit the result by hand.".into()
        } else {
            "No conflict markers in this file. Save to keep it as it is.".into()
        };
    }
    if s.hand_edited {
        return if has_markers(s.result) {
            "Edited by hand. Some conflict markers are still in it.".into()
        } else {
            "Edited by hand. Ready to save.".into()
        };
    }
    if s.chosen == s.conflicts {
        "All conflicts chosen. Ready to save.".into()
    } else {
        format!(
            "{} of {} conflicts chosen. Pick a side for each one, or edit the result.",
            s.chosen, s.conflicts
        )
    }
}

pub fn files_word(n: usize) -> &'static str {
    if n == 1 { "file" } else { "files" }
}
