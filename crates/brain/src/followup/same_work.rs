use nixa_store::claims::{FileHolder, clean_claim_path, paths_overlap};
use nixa_store::tasks::{Task, TaskStatus};
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("same work pattern")
}

pub static ANYWAY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:anyway|anyways|even so|still send|send it still|i know it is held|i know it's held)\b"));
static TASK_MENTION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\btasks?\s*#?\s*(\d{1,6})\b"));
static NOT_THIS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:do not|don't|dont|never|avoid|skip|leave|except|not touch|not work on|not start|stay off)\b[^.,;!?\n]{0,25}$"));
static ONLY_SEEN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:like|as in|same as|similar to|see|based on|copy of|the way)\s+(?:in\s+|for\s+|of\s+)?(?:the\s+)?$"));
static FILE_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[A-Za-z0-9_@.+-]+/)*[A-Za-z0-9_@+-][A-Za-z0-9_@.+-]*\.[A-Za-z][A-Za-z0-9]{0,7}$"));
static VERSION: LazyLock<Regex> = LazyLock::new(|| re(r"^\d+(?:\.\d+)+$"));
static WEB: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:https?|www)\b"));

pub const COMMON_NAMES: [&str; 12] = [
    "index.ts",
    "index.js",
    "index.tsx",
    "main.ts",
    "types.ts",
    "package.json",
    "readme.md",
    "tsconfig.json",
    "changelog.md",
    "lessons.md",
    "prd.md",
    "commands.md",
];

pub const SAME_WORK_MS: u64 = 10 * 60 * 1000;
pub const SAME_HELD_KEEP_MS: u64 = 24 * 60 * 60 * 1000;

const LEAD: &str = " \t\r\n\"'`([{,:;";
const TRAIL: &str = " \t\r\n\"'`)]},:;!?";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SameWorkTask {
    pub id: i64,
    pub title: String,
    pub status: TaskStatus,
    pub terminal: String,
    pub claimed_by: String,
}

pub fn task_of(task: &Task) -> SameWorkTask {
    SameWorkTask { id: task.id, title: task.title.clone(), status: task.status, terminal: task.terminal_id.clone(), claimed_by: task.claimed_by.clone() }
}

pub struct SameWorkInput<'a> {
    pub text: &'a str,
    pub target: Option<&'a str>,
    pub tasks: &'a [SameWorkTask],
    pub files: &'a [FileHolder],
    pub live: &'a HashSet<String>,
    pub working: &'a HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SameWorkHit {
    Task { task: i64, title: String, status: TaskStatus, terminal: String, name: String },
    File { task: i64, file: String, terminal: String, name: String },
}

impl SameWorkHit {
    pub fn terminal(&self) -> &str {
        match self {
            SameWorkHit::Task { terminal, .. } | SameWorkHit::File { terminal, .. } => terminal,
        }
    }

    pub fn task(&self) -> i64 {
        match self {
            SameWorkHit::Task { task, .. } | SameWorkHit::File { task, .. } => *task,
        }
    }
}

fn before(text: &str, at: usize) -> &str {
    let head = &text[..at];
    let start = head.rfind(['.', '\n', '!', '?']).map(|i| i + 1).unwrap_or(0);
    &head[start..]
}

pub fn tasks_named(text: &str) -> Vec<i64> {
    let mut found = Vec::new();
    for caps in TASK_MENTION.captures_iter(text) {
        let Ok(id) = caps[1].parse::<i64>() else { continue };
        if id <= 0 || found.contains(&id) {
            continue;
        }
        let at = caps.get(0).map(|m| m.start()).unwrap_or(0);
        let lead = before(text, at);
        if NOT_THIS.is_match(lead) || ONLY_SEEN.is_match(lead) {
            continue;
        }
        found.push(id);
    }
    found
}

fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_@.+-/".contains(c)
}

fn mentions(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut last: Option<char> = None;
    while let Some((start, c)) = chars.next() {
        if !path_char(c) {
            last = Some(c);
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(i, n)) = chars.peek() {
            if !path_char(n) {
                break;
            }
            end = i + n.len_utf8();
            chars.next();
        }
        let next = text[end..].chars().next();
        let lead_ok = last.is_none_or(|l| LEAD.contains(l));
        last = text[..end].chars().next_back();
        if !lead_ok {
            continue;
        }
        let run = &text[start..end];
        let word = match run.strip_suffix('.') {
            Some(cut) if next.is_none_or(char::is_whitespace) => cut,
            Some(_) => continue,
            None if next.is_none_or(|n| TRAIL.contains(n)) => run,
            None => continue,
        };
        if FILE_NAME.is_match(word) {
            let at = if start == 0 { 0 } else { start - text[..start].chars().next_back().map(char::len_utf8).unwrap_or(0) };
            out.push((at, word));
        }
    }
    out
}

pub fn files_named(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (at, word) in mentions(text) {
        let raw = clean_claim_path(word);
        if raw.is_empty() || VERSION.is_match(&raw) || WEB.is_match(&raw) || raw.contains("..") {
            continue;
        }
        if !raw.contains('/') && COMMON_NAMES.contains(&raw.to_lowercase().as_str()) {
            continue;
        }
        if NOT_THIS.is_match(before(text, at)) {
            continue;
        }
        if !found.contains(&raw) {
            found.push(raw);
        }
    }
    found
}

fn same_file(named: &str, held: &str) -> bool {
    if named.contains('/') {
        return paths_overlap(named, held) || held.ends_with(&format!("/{named}")) || named.ends_with(&format!("/{held}"));
    }
    held == named || held.ends_with(&format!("/{named}"))
}

pub fn same_work(input: &SameWorkInput) -> Option<SameWorkHit> {
    let others = |terminal: &str| !terminal.is_empty() && Some(terminal) != input.target && input.live.contains(terminal);
    let holds = |task: &SameWorkTask| matches!(task.status, TaskStatus::Doing | TaskStatus::Blocked) || task.status == TaskStatus::Review && input.working.contains(&task.terminal);
    for id in tasks_named(input.text) {
        let Some(task) = input.tasks.iter().find(|t| t.id == id) else { continue };
        if !holds(task) || !others(&task.terminal) {
            continue;
        }
        let name = if task.claimed_by.is_empty() { task.terminal.clone() } else { task.claimed_by.clone() };
        return Some(SameWorkHit::Task { task: task.id, title: task.title.clone(), status: task.status, terminal: task.terminal.clone(), name });
    }
    let files = files_named(input.text);
    if files.is_empty() {
        return None;
    }
    for holder in input.files {
        if !others(&holder.terminal) {
            continue;
        }
        if let Some(file) = files.iter().find(|named| holder.files.iter().any(|held| same_file(named, held))) {
            let name = if holder.name.is_empty() { holder.terminal.clone() } else { holder.name.clone() };
            return Some(SameWorkHit::File { task: holder.task, file: file.clone(), terminal: holder.terminal.clone(), name });
        }
    }
    None
}

pub fn same_work_line(hit: &SameWorkHit, number: Option<u32>) -> String {
    let (name, terminal) = match hit {
        SameWorkHit::Task { name, terminal, .. } | SameWorkHit::File { name, terminal, .. } => (name, terminal),
    };
    let mut place = name.clone();
    if name != terminal {
        place.push_str(&format!(" in {terminal}"));
    }
    if let Some(n) = number {
        place.push_str(&format!(" (terminal {n})"));
    }
    match hit {
        SameWorkHit::Task { task, status, .. } => format!("Task #{task} is held by {place} ({}). Send to {terminal} or wait.", status.as_str()),
        SameWorkHit::File { task, file, .. } => format!("{file} is held by {place} for task #{task}. Send to {terminal} or wait."),
    }
}

pub fn same_work_key(hit: &SameWorkHit) -> String {
    format!("{}:{}", hit.task(), hit.terminal())
}

pub fn auto_pass(working: bool, first: Option<u64>, now: u64) -> Option<&'static str> {
    if !working {
        return Some("the holder is idle");
    }
    let first = first?;
    (now.saturating_sub(first) >= SAME_WORK_MS).then_some("the holder stayed busy past the time limit")
}

pub fn same_work_reason(line: &str) -> String {
    let minutes = SAME_WORK_MS / 60_000;
    format!("{line} Nothing was sent or started yet, because the holder is busy with it. Do not ask the user: send it to the holder now, or wait for the holder with nixa_term_wait. When the holder is idle, or still busy after {minutes} minutes, send the same work again and Nixa lets it through.")
}

pub fn anyway(text: &str) -> bool {
    ANYWAY.is_match(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: i64, status: TaskStatus, terminal: &str) -> SameWorkTask {
        SameWorkTask { id, title: format!("job {id}"), status, terminal: terminal.into(), claimed_by: "builder".into() }
    }

    fn holder(task: i64, terminal: &str, files: &[&str]) -> FileHolder {
        FileHolder { task, terminal: terminal.into(), name: String::new(), files: files.iter().map(|f| f.to_string()).collect(), at: 0 }
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_tasks_but_not_the_ones_to_avoid() {
        assert_eq!(tasks_named("Start task 12 and task #13, then tasks 12 again."), vec![12, 13]);
        assert_eq!(tasks_named("Do not touch task 9. Work on task 10."), vec![10]);
        assert_eq!(tasks_named("Build it like in task 4."), Vec::<i64>::new());
        assert_eq!(tasks_named("task 0 and task 1234567"), Vec::<i64>::new());
    }

    #[test]
    fn names_files_and_skips_noise() {
        assert_eq!(files_named("Fix src/app/main.rs and (crates/ui/lib.rs), then lib.rs."), vec!["src/app/main.rs", "crates/ui/lib.rs", "lib.rs"]);
        assert_eq!(files_named("Update package.json and README.md and v1.2.3"), Vec::<String>::new());
        assert_eq!(files_named("Read https://example.com/a.html and ../up/x.rs"), Vec::<String>::new());
        assert_eq!(files_named("Edit ./docs/plan.md but do not touch src/keep.rs"), vec!["docs/plan.md"]);
        assert_eq!(files_named("name:file.txt?"), vec!["file.txt"]);
        assert_eq!(files_named("x=bad.rs a.rs.bak"), vec!["a.rs.bak"]);
        assert_eq!(files_named("Open docs/a.md."), vec!["docs/a.md"]);
    }

    #[test]
    fn finds_a_held_task_in_another_live_terminal() {
        let tasks = vec![task(7, TaskStatus::Doing, "m2"), task(8, TaskStatus::Review, "m3"), task(9, TaskStatus::Todo, "m2")];
        let live = set(&["m1", "m2", "m3"]);
        let none = HashSet::new();
        let input = |text| SameWorkInput { text, target: Some("m1"), tasks: &tasks, files: &[], live: &live, working: &none };
        let hit = same_work(&input("Please do task 7")).expect("a hit");
        assert_eq!(hit.task(), 7);
        assert_eq!(hit.terminal(), "m2");
        assert_eq!(same_work_line(&hit, Some(2)), "Task #7 is held by builder in m2 (terminal 2) (doing). Send to m2 or wait.");
        assert_eq!(same_work(&input("Please do task 8")), None);
        assert_eq!(same_work(&input("Please do task 9")), None);
        let working = set(&["m3"]);
        let busy = SameWorkInput { text: "task 8 now", target: Some("m1"), tasks: &tasks, files: &[], live: &live, working: &working };
        assert_eq!(same_work(&busy).map(|h| h.task()), Some(8));
        let own = SameWorkInput { text: "task 7", target: Some("m2"), tasks: &tasks, files: &[], live: &live, working: &none };
        assert_eq!(same_work(&own), None);
    }

    #[test]
    fn finds_a_held_file() {
        let files = vec![holder(3, "m5", &["crates/app/src/main.rs"]), holder(4, "m6", &["crates/ui"])];
        let live = set(&["m5", "m6"]);
        let none = HashSet::new();
        let check = |text| same_work(&SameWorkInput { text, target: None, tasks: &[], files: &files, live: &live, working: &none });
        let hit = check("Change main.rs please").expect("file hit");
        assert_eq!(same_work_line(&hit, None), "main.rs is held by m5 for task #3. Send to m5 or wait.");
        assert_eq!(check("Edit crates/ui/src/button.rs").map(|h| h.task()), Some(4));
        assert_eq!(check("Edit other.rs"), None);
        let gone = set(&["m6"]);
        assert_eq!(same_work(&SameWorkInput { text: "main.rs", target: None, tasks: &[], files: &files, live: &gone, working: &none }), None);
    }

    #[test]
    fn auto_pass_lets_an_idle_holder_through_at_once_and_a_busy_one_after_the_limit() {
        assert_eq!(auto_pass(false, None, 5), Some("the holder is idle"));
        assert_eq!(auto_pass(true, None, 5), None);
        assert_eq!(auto_pass(true, Some(0), SAME_WORK_MS - 1), None);
        assert_eq!(auto_pass(false, Some(0), 1), Some("the holder is idle"));
        assert_eq!(auto_pass(true, Some(0), SAME_WORK_MS), Some("the holder stayed busy past the time limit"));
        let hit = SameWorkHit::File { task: 3, file: "a.rs".into(), terminal: "m5".into(), name: String::new() };
        assert_eq!(same_work_key(&hit), "3:m5");
    }

    #[test]
    fn anyway_words_and_reason_text() {
        assert!(anyway("send it anyway"));
        assert!(anyway("I know it's held, go"));
        assert!(!anyway("send it"));
        let auto = same_work_reason("x.");
        assert!(auto.contains("send it to the holder now") && !auto.contains("tell the user"));
        assert!(auto.contains("send the same work again and Nixa lets it through"));
        assert!(auto.starts_with("x. Nothing was sent") && !auto.contains("Held for you"));
    }
}
