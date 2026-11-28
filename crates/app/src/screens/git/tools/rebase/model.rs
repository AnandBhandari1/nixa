use std::collections::{HashMap, HashSet};

pub const MAX_COMMITS: usize = 200;
pub const MAX_TODO: usize = 400_000;
pub const MAX_LISTED: usize = 10;
pub const MAX_DEEPER: usize = 50;
pub const REBASE_FLAGS: [&str; 5] = [
    "--autostash",
    "--no-autosquash",
    "--no-update-refs",
    "--no-rebase-merges",
    "--empty=keep",
];
pub const AMEND_COMMAND: &str =
    "git commit --amend --only --no-verify --allow-empty --cleanup=whitespace -F -";
pub const CHANGED_MARK: &str = "nixa-git: the branch changed while the rebase was starting";
pub const LOG_FORMAT: &str = "%H%x00%h%x00%P%x00%T%x00%an%x00%ae%x00%at%x00%s%x00%b%x1e";
pub const CHECK_AWK: &str = r#"BEGIN { n = split(want, w, " ") } $1 == "pick" || $1 == "p" { i++; s = tolower($2); if (i > n || length(s) < 4 || s !~ /^[0-9a-f]+$/ || index(w[i], s) != 1) bad = 1 } END { if (bad || i != n) exit 1; exit 0 }"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

impl Action {
    pub fn word(self) -> &'static str {
        match self {
            Action::Pick => "pick",
            Action::Reword => "reword",
            Action::Squash => "squash",
            Action::Fixup => "fixup",
            Action::Drop => "drop",
        }
    }

    pub fn joins(self) -> bool {
        matches!(self, Action::Squash | Action::Fixup)
    }

    pub fn kept(self) -> bool {
        matches!(self, Action::Pick | Action::Reword)
    }

    pub fn chip(self) -> (&'static str, &'static str) {
        match self {
            Action::Pick => ("Keep", "Keep this commit as it is"),
            Action::Reword => ("Edit message", "Keep this commit and change its message"),
            Action::Squash => (
                "Squash into the one above",
                "Join this commit into the one above and keep both messages",
            ),
            Action::Fixup => (
                "Fixup (squash, drop this message)",
                "Join this commit into the one above and keep only the message above",
            ),
            Action::Drop => ("Drop", "Remove this commit and its changes"),
        }
    }
}

pub const ACTIONS: [Action; 5] = [
    Action::Pick,
    Action::Reword,
    Action::Squash,
    Action::Fixup,
    Action::Drop,
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Message {
    pub summary: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Step {
    pub sha: String,
    pub action: Action,
    pub message: Option<Message>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub parents: Vec<String>,
    pub tree: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub subject: String,
    pub body: String,
    pub is_merge: bool,
    pub pushed: bool,
    pub empty: bool,
}

impl Commit {
    pub fn own(&self) -> Message {
        Message {
            summary: self.subject.clone(),
            body: self.body.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepProblem {
    pub index: Option<usize>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub problems: Vec<StepProblem>,
}

impl Check {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn problem(&self) -> Option<String> {
        self.problems.first().map(|p| p.text.clone())
    }
}

pub fn is_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

pub fn one_line(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in text.chars() {
        if (c as u32) < 0x20 || c as u32 == 0x7f {
            gap = true;
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(c);
    }
    if gap {
        out.push(' ');
    }
    out.trim().to_string()
}

pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn compose(m: &Message) -> String {
    let head = m
        .summary
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let body = m.body.replace("\r\n", "\n");
    let mut lines: Vec<&str> = body.split('\n').collect();
    while lines.len() > 1 && lines[0].trim().is_empty() {
        lines.remove(0);
    }
    if lines.len() == 1 && lines[0].trim().is_empty() {
        lines.clear();
    }
    let rest = lines.join("\n").trim_end().to_string();
    if head.is_empty() && rest.is_empty() {
        return String::new();
    }
    if rest.is_empty() {
        format!("{head}\n")
    } else {
        format!("{head}\n\n{rest}\n")
    }
}

pub fn split_message(text: &str) -> Message {
    let norm = text.replace("\r\n", "\n");
    let mut lines: Vec<&str> = norm.trim_end().split('\n').collect();
    let summary = if lines.is_empty() {
        String::new()
    } else {
        lines.remove(0).trim().to_string()
    };
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    Message {
        summary,
        body: lines.join("\n"),
    }
}

pub fn same_message(a: &Message, b: &Message) -> bool {
    compose(a).trim() == compose(b).trim()
}

pub fn amend_line(m: &Message) -> String {
    let text: String = compose(m)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| {
            let n = *c as u32;
            !(n < 0x20 && *c != '\t' && *c != '\n') && n != 0x7f
        })
        .collect();
    let text = text.trim_end_matches('\n');
    let parts: Vec<String> = text.split('\n').map(shell_quote).collect();
    format!("exec printf '%s\\n' {} | {AMEND_COMMAND}", parts.join(" "))
}

pub fn by_sha(commits: &[Commit]) -> HashMap<&str, &Commit> {
    commits.iter().map(|c| (c.sha.as_str(), c)).collect()
}

pub fn move_step<T: Clone>(steps: &[T], index: usize, up: bool) -> Vec<T> {
    let mut next = steps.to_vec();
    let to = if up {
        index.checked_sub(1)
    } else {
        Some(index + 1)
    };
    match to {
        Some(to) if index < next.len() && to < next.len() => {
            let item = next.remove(index);
            next.insert(to, item);
            next
        }
        _ => next,
    }
}

pub fn join_target(actions: &[Action], index: usize) -> Option<usize> {
    (0..index).rev().find(|&i| actions[i].kept())
}

pub fn joined_message(steps: &[Step], commits: &[Commit], index: usize) -> Message {
    let map = by_sha(commits);
    let own = |s: &Step| map.get(s.sha.as_str()).map(|c| c.own()).unwrap_or_default();
    let Some(step) = steps.get(index) else {
        return Message::default();
    };
    let actions: Vec<Action> = steps.iter().map(|s| s.action).collect();
    let Some(head) = join_target(&actions, index) else {
        return own(step);
    };
    let first = &steps[head];
    let first_msg = match (&first.action, &first.message) {
        (Action::Reword, Some(m)) => m.clone(),
        _ => own(first),
    };
    let mut text = compose(&first_msg).trim().to_string();
    for (j, s) in steps.iter().enumerate().take(index + 1).skip(head + 1) {
        if s.action != Action::Squash {
            continue;
        }
        match (&s.message, j != index) {
            (Some(m), true) => text = compose(m).trim().to_string(),
            _ => {
                let more = compose(&own(s)).trim().to_string();
                text = [text, more]
                    .into_iter()
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
            }
        }
    }
    split_message(&text)
}

pub fn validate(steps: &[Step], commits: Option<&[Commit]>) -> Check {
    let mut problems = Vec::new();
    if steps.is_empty() {
        problems.push(StepProblem {
            index: None,
            text: "There are no commits to change.".into(),
        });
        return Check { problems };
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for (i, step) in steps.iter().enumerate() {
        if !is_sha(&step.sha) {
            problems.push(StepProblem {
                index: Some(i),
                text: "A commit id is missing or wrong.".into(),
            });
        } else if !seen.insert(step.sha.as_str()) {
            problems.push(StepProblem {
                index: Some(i),
                text: format!("Commit {} is in the list twice.", short(&step.sha)),
            });
        }
        let needs = step.action == Action::Reword
            || (step.action == Action::Squash && step.message.is_some());
        let empty = step
            .message
            .as_ref()
            .is_none_or(|m| m.summary.trim().is_empty());
        if needs && empty {
            problems.push(StepProblem {
                index: Some(i),
                text: format!("Write a summary for {}.", short(&step.sha)),
            });
        }
    }
    if let Some(commits) = commits {
        let want: HashSet<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
        if want.len() != seen.len() || seen.iter().any(|s| !want.contains(s)) {
            problems.push(StepProblem {
                index: None,
                text: "The list does not match the branch any more. Load it again.".into(),
            });
        }
    }
    match steps.iter().position(|s| s.action != Action::Drop) {
        None => problems.push(StepProblem {
            index: None,
            text: "Keep at least one commit.".into(),
        }),
        Some(first) if steps[first].action.joins() => problems.push(StepProblem {
            index: Some(first),
            text:
                "The first kept commit cannot be joined. There is no commit above it to join into."
                    .into(),
        }),
        Some(_) => {}
    }
    Check { problems }
}

pub fn build_todo(steps: &[Step], commits: &[Commit]) -> Result<String, String> {
    let check = validate(steps, None);
    if let Some(problem) = check.problem() {
        return Err(problem);
    }
    let map = by_sha(commits);
    let mut out: Vec<String> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let subject = map
            .get(step.sha.as_str())
            .map(|c| one_line(&c.subject))
            .unwrap_or_default();
        let tail = if subject.is_empty() {
            String::new()
        } else {
            format!(" {subject}")
        };
        if step.action == Action::Reword {
            out.push(format!("pick {}{tail}", step.sha));
            out.push(amend_line(&step.message.clone().unwrap_or_default()));
            continue;
        }
        out.push(format!("{} {}{tail}", step.action.word(), step.sha));
        if step.action != Action::Squash {
            continue;
        }
        if let Some(m) = &step.message {
            out.push(amend_line(m));
            continue;
        }
        if !commits.is_empty() {
            let joined = joined_message(steps, commits, i);
            if !joined.summary.trim().is_empty() {
                out.push(amend_line(&joined));
            }
        }
    }
    Ok(format!("{}\n", out.join("\n")))
}

fn same_step(step: Option<&Step>, commit: Option<&Commit>) -> bool {
    let (Some(step), Some(commit)) = (step, commit) else {
        return false;
    };
    if step.sha != commit.sha {
        return false;
    }
    match step.action {
        Action::Pick => true,
        Action::Reword => step
            .message
            .as_ref()
            .is_some_and(|m| same_message(m, &commit.own())),
        _ => false,
    }
}

pub fn first_change(steps: &[Step], commits: &[Commit]) -> Option<usize> {
    (0..steps.len().max(commits.len())).find(|&i| !same_step(steps.get(i), commits.get(i)))
}

pub fn is_unchanged(steps: &[Step], commits: &[Commit]) -> bool {
    steps.len() == commits.len() && first_change(steps, commits).is_none()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub total: usize,
    pub kept: usize,
    pub dropped: usize,
    pub squashed: usize,
    pub reworded: usize,
    pub moved: bool,
    pub first_change: Option<usize>,
    pub pushed_changed: usize,
    pub text: String,
}

pub fn summarize(steps: &[Step], commits: &[Commit]) -> Summary {
    let count = |a: Action| steps.iter().filter(|s| s.action == a).count();
    let map = by_sha(commits);
    let reworded = steps
        .iter()
        .filter(|s| match (s.action, &s.message) {
            (Action::Reword, Some(m)) => map
                .get(s.sha.as_str())
                .is_none_or(|c| !same_message(m, &c.own())),
            (Action::Squash, Some(_)) => true,
            _ => false,
        })
        .count();
    let moved =
        commits.len() == steps.len() && commits.iter().zip(steps).any(|(c, s)| c.sha != s.sha);
    let first = first_change(steps, commits);
    let mut out = Summary {
        total: steps.len(),
        kept: count(Action::Pick) + count(Action::Reword),
        dropped: count(Action::Drop),
        squashed: count(Action::Squash) + count(Action::Fixup),
        reworded,
        moved,
        first_change: first,
        pushed_changed: first.map_or(0, |f| commits.iter().skip(f).filter(|c| c.pushed).count()),
        text: String::new(),
    };
    let mut parts = vec![
        format!("{} kept", out.kept),
        format!("{} dropped", out.dropped),
        format!("{} squashed", out.squashed),
    ];
    if reworded > 0 {
        parts.push(plural(reworded, "new message"));
    }
    out.text = format!(
        "{}{}.",
        parts.join(", "),
        if moved { ". New order" } else { "" }
    );
    out
}

pub fn parse_log(text: &str) -> Vec<Commit> {
    text.split('\u{1e}')
        .filter_map(|record| {
            let body = record.strip_prefix('\n').unwrap_or(record);
            if body.trim().is_empty() {
                return None;
            }
            let fields: Vec<&str> = body.split('\0').collect();
            let sha = fields.first().copied().unwrap_or_default();
            if !is_sha(sha) {
                return None;
            }
            let get = |i: usize| fields.get(i).copied().unwrap_or_default().to_string();
            let parents: Vec<String> = get(2)
                .split(' ')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect();
            Some(Commit {
                sha: sha.to_string(),
                short: get(1),
                is_merge: parents.len() > 1,
                parents,
                tree: get(3),
                author: get(4),
                email: get(5),
                at: get(6).parse().unwrap_or(0),
                subject: get(7),
                body: fields
                    .get(8..)
                    .map(|r| r.join("\0"))
                    .unwrap_or_default()
                    .trim_end()
                    .to_string(),
                pushed: false,
                empty: false,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub x: char,
    pub y: char,
    pub path: String,
}

pub fn parse_status_z(text: &str) -> Vec<Entry> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let entry = parts[i];
        i += 1;
        let mut chars = entry.chars();
        let (Some(x), Some(y), Some(_)) = (chars.next(), chars.next(), chars.next()) else {
            continue;
        };
        let path: String = chars.collect();
        if path.is_empty() {
            continue;
        }
        out.push(Entry { x, y, path });
        if x == 'R' || x == 'C' {
            i += 1;
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Local {
    pub dirty: bool,
    pub staged: bool,
    pub mixed: Vec<String>,
    pub unmerged: Vec<String>,
}

fn unmerged(e: &Entry) -> bool {
    e.x == 'U' || e.y == 'U' || (e.x == 'A' && e.y == 'A') || (e.x == 'D' && e.y == 'D')
}

fn changed(c: char) -> bool {
    !matches!(c, ' ' | '?' | '!')
}

pub fn local_changes(entries: &[Entry]) -> Local {
    Local {
        dirty: !entries.is_empty(),
        staged: entries.iter().any(|e| !unmerged(e) && changed(e.x)),
        mixed: entries
            .iter()
            .filter(|e| !unmerged(e) && changed(e.x) && changed(e.y))
            .map(|e| e.path.clone())
            .collect(),
        unmerged: entries
            .iter()
            .filter(|e| unmerged(e))
            .map(|e| e.path.clone())
            .collect(),
    }
}

fn parents_of(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..parts.len()).map(|i| parts[..i].join("/")).collect()
}

pub fn ignored_at_risk(ignored: &[String], touched: &[String]) -> (Vec<String>, Vec<String>) {
    let files: HashSet<&str> = touched.iter().map(String::as_str).collect();
    let dirs: HashSet<String> = touched.iter().flat_map(|t| parents_of(t)).collect();
    let mut risky = Vec::new();
    let mut deeper = Vec::new();
    for entry in ignored {
        let name = entry.trim_end_matches('/');
        if name.is_empty() {
            continue;
        }
        if files.contains(name) || parents_of(name).iter().any(|p| files.contains(p.as_str())) {
            risky.push(entry.clone());
        } else if dirs.contains(name) {
            if entry.ends_with('/') {
                deeper.push(entry.clone());
            } else {
                risky.push(entry.clone());
            }
        }
    }
    (risky, deeper)
}

pub fn list_text(paths: &[String]) -> String {
    let shown = paths
        .iter()
        .take(MAX_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > MAX_LISTED {
        format!("{shown} and {} more", paths.len() - MAX_LISTED)
    } else {
        shown
    }
}

pub fn check_script(want: &[String], head: &str, head_ref: Option<&str>) -> String {
    [
        format!("want={}", shell_quote(&want.join(" "))),
        format!("head={}", shell_quote(head)),
        format!("name={}", shell_quote(head_ref.unwrap_or("detached HEAD"))),
        "d=$(git rev-parse --git-path rebase-merge) || exit 1".into(),
        "ok=1".into(),
        r#"[ "$(cat "$d/orig-head" 2>/dev/null)" = "$head" ] || ok=0"#.into(),
        r#"[ "$(cat "$d/head-name" 2>/dev/null)" = "$name" ] || ok=0"#.into(),
        format!(
            r#"[ "$ok" = 1 ] && awk -v want="$want" {} "$2" || ok=0"#,
            shell_quote(CHECK_AWK)
        ),
        format!(
            r#"if [ "$ok" != 1 ]; then echo {} >&2; exit 1; fi"#,
            shell_quote(CHANGED_MARK)
        ),
        r#"cp "$1/todo" "$2""#.into(),
        String::new(),
    ]
    .join("\n")
}

pub fn conflict_files(text: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for line in text.split('\n').skip(1) {
        if line.trim().is_empty() {
            break;
        }
        if !files.iter().any(|f| f == line) {
            files.push(line.to_string());
        }
    }
    files
}

pub fn force_hint(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => "1 changed commit was pushed before. Push with force (safe, with lease) to update the remote.".into(),
        n => format!("{n} changed commits were pushed before. Push with force (safe, with lease) to update the remote."),
    }
}
