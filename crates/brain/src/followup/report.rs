use crate::prompts::screen::{ScreenState, read_screen};
use crate::util::short_hash;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const CHECKED_MAX: usize = 16;
pub const AFTER_INPUT_MAX: usize = 3;
pub const WORK_LOOK_BACK: usize = 8;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("follow-up pattern")
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| re(r"\x1b\[[0-?]*[ -/]*[@-~]"));
static INPUT: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:\d+>|(?:\d+)?[❯›])\s*\p{Co}?\s*$"));
static RING_INPUT: LazyLock<Regex> = LazyLock::new(|| re(r"^❯(?:\s+.*)?$"));
static RING_FOOTER: LazyLock<Regex> = LazyLock::new(|| re(r"^☤\s+.+\s·\s"));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"^─{3,}"));
static WORKING: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)esc to (?:interrupt|cancel)|ctrl[+-]c to (?:interrupt|cancel)|[⠀-⣿◐◓◑◒◴◵◶◷].*\d+(?:\.\d+)?\s*(?:ms|s|m)\b|^\s*(?:[✻✶✳✢✽·∙●*]\s*)?(?:thinking|working|running|compacting|opening websocket|reconnecting|waiting for (?:background agents?|workflows?))\b")
});
static FRAME: LazyLock<Regex> = LazyLock::new(|| re("[│┃╭╮╰╯─━┌┐└┘├┤┬┴┼═║]"));
static SPEED: LazyLock<Regex> = LazyLock::new(|| re(r"^\d+(?:\.\d+)?(?:ms|s|m|h)\b.*\btps\b"));
static NUMBERED_TURN: LazyLock<Regex> = LazyLock::new(|| re(r"^\d+›\s+\S"));
static RING_TURN: LazyLock<Regex> = LazyLock::new(|| re(r"^❯\s+\S"));
static PROMPT_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^[❯›].*$"));

fn lines_of(screen: &str, frames: bool) -> Vec<String> {
    let plain = ANSI.replace_all(screen, "");
    let plain = if frames { FRAME.replace_all(&plain, " ").into_owned() } else { plain.into_owned() };
    plain.split('\n').map(|l| l.trim().to_string()).collect()
}

fn at(lines: &[String], index: isize) -> &str {
    usize::try_from(index).ok().and_then(|i| lines.get(i)).map(String::as_str).unwrap_or("")
}

fn ring_input(lines: &[String]) -> Option<usize> {
    (0..lines.len()).rev().find(|&i| {
        let i = i as isize;
        RING_INPUT.is_match(at(lines, i))
            && RULE.is_match(at(lines, i - 1))
            && (RING_FOOTER.is_match(at(lines, i + 1)) || RULE.is_match(at(lines, i + 1)) && RING_FOOTER.is_match(at(lines, i + 2)))
    })
}

fn filled_after(lines: &[String], input: usize) -> usize {
    lines[input + 1..].iter().filter(|l| !l.is_empty()).count()
}

pub fn follow_up_report(screen: &str) -> Option<String> {
    let lines = lines_of(screen, false);
    let ring = ring_input(&lines);
    let input = ring.or_else(|| lines.iter().rposition(|l| INPUT.is_match(l)))?;
    if input < 1 || filled_after(&lines, input) > AFTER_INPUT_MAX {
        return None;
    }
    let start = lines[..input]
        .iter()
        .rposition(|l| NUMBERED_TURN.is_match(l) || ring.is_some() && RING_TURN.is_match(l))
        .map(|i| i + 1)
        .unwrap_or(0);
    let turn = &lines[start..input];
    let after: Vec<&String> = lines[input + 1..].iter().filter(|l| !(ring.is_some() && RING_FOOTER.is_match(l))).collect();
    if after.iter().any(|l| WORKING.is_match(l)) || turn.iter().any(|l| WORKING.is_match(l)) {
        return None;
    }
    let kept: Vec<&str> = turn.iter().filter(|l| !SPEED.is_match(l)).map(String::as_str).collect();
    let report = FRAME.replace_all(&kept.join("\n"), " ").trim().to_string();
    let reading = read_screen(&report)?;
    if reading.state == ScreenState::Waiting || PROMPT_LINE.replace_all(&report, "").trim().is_empty() {
        return None;
    }
    Some(report)
}

pub fn input_empty(screen: &str) -> bool {
    let lines = lines_of(screen, true);
    let Some(input) = lines.iter().rposition(|l| INPUT.is_match(l) || l == "❯") else { return false };
    if filled_after(&lines, input) > AFTER_INPUT_MAX {
        return false;
    }
    !lines[input.saturating_sub(WORK_LOOK_BACK)..].iter().any(|l| WORKING.is_match(l))
}

fn key_of(text: &str) -> String {
    short_hash(text)
}

fn screen_key(screen: &str) -> String {
    key_of(&follow_up_report(screen).unwrap_or_else(|| screen.trim().to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobKey {
    pub terminal: String,
    pub at: u64,
    pub sent: String,
}

#[derive(Debug, Clone)]
struct Watch {
    at: u64,
    sent: String,
    baseline: Option<String>,
    checked: HashSet<String>,
}

impl Watch {
    fn fits(&self, job: &JobKey) -> bool {
        self.at == job.at && self.sent == job.sent
    }
}

#[derive(Debug, Default)]
pub struct JobFollowUp {
    watches: HashMap<String, Watch>,
    before_send: HashMap<String, String>,
}

impl JobFollowUp {
    pub fn prepare(&mut self, terminal: &str, screen: &str) {
        self.before_send.insert(terminal.to_string(), screen_key(screen));
    }

    pub fn start(&mut self, job: &JobKey) {
        let baseline = self.before_send.remove(&job.terminal);
        self.watches.insert(job.terminal.clone(), Watch { at: job.at, sent: job.sent.clone(), baseline, checked: HashSet::new() });
    }

    pub fn observe(&mut self, job: &JobKey, screen: &str) -> Option<String> {
        let report = follow_up_report(screen);
        let key = key_of(report.as_deref().unwrap_or(screen.trim()));
        let watch = self.watches.entry(job.terminal.clone()).or_insert_with(|| Watch { at: job.at, sent: job.sent.clone(), baseline: Some(key.clone()), checked: HashSet::new() });
        if !watch.fits(job) {
            *watch = Watch { at: job.at, sent: job.sent.clone(), baseline: Some(key.clone()), checked: HashSet::new() };
        }
        let baseline = watch.baseline.get_or_insert_with(|| key.clone());
        let report = report?;
        if &key == baseline || watch.checked.contains(&key) || watch.checked.len() >= CHECKED_MAX {
            return None;
        }
        Some(report)
    }

    pub fn checked(&mut self, job: &JobKey, screen: &str) {
        if let Some(report) = follow_up_report(screen) {
            self.mark(job, &report);
        }
    }

    pub fn mark(&mut self, job: &JobKey, report: &str) {
        if let Some(watch) = self.watches.get_mut(&job.terminal)
            && watch.fits(job)
        {
            watch.checked.insert(key_of(report));
        }
    }

    pub fn forget(&mut self, terminal: &str) {
        self.watches.remove(terminal);
        self.before_send.remove(terminal);
    }

    pub fn keep(&mut self, jobs: &HashSet<String>, live: &HashSet<String>) {
        self.watches.retain(|t, _| jobs.contains(t));
        self.before_send.retain(|t, _| live.contains(t));
    }

    pub fn clear(&mut self) {
        self.watches.clear();
        self.before_send.clear();
    }

    pub fn watching(&self, terminal: &str) -> bool {
        self.watches.contains_key(terminal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DONE: &str = "⏺ I fixed the parser and all 12 tests pass.\n\n╭──────────╮\n│ ❯        │\n╰──────────╯\n  ? for shortcuts\n";

    fn job(at: u64, sent: &str) -> JobKey {
        JobKey { terminal: "m4".into(), at, sent: sent.into() }
    }

    #[test]
    fn finds_a_report_at_an_empty_prompt() {
        let screen = "❯ fix the parser\nI fixed the parser.\nAll 12 tests pass.\n\n❯\n";
        assert_eq!(follow_up_report(screen).as_deref(), Some("❯ fix the parser\nI fixed the parser.\nAll 12 tests pass."));
    }

    #[test]
    fn a_framed_prompt_counts_after_frames_go() {
        let screen = "I fixed the parser.\n──────────\n❯ \n──────────\n";
        assert_eq!(follow_up_report(screen).as_deref(), Some("I fixed the parser."));
        assert!(input_empty(DONE));
    }

    #[test]
    fn no_report_while_working_or_with_no_prompt() {
        assert_eq!(follow_up_report("I am still on it\n✻ Thinking… (12s · esc to interrupt)\n❯\n"), None);
        assert_eq!(follow_up_report("just some output\nmore output\n"), None);
        assert_eq!(follow_up_report("❯\n"), None);
        assert_eq!(follow_up_report("text\n❯\na\nb\nc\nd\n"), None);
        assert_eq!(follow_up_report("Waiting for background agents to finish\n❯\n"), None);
    }

    #[test]
    fn numbered_turns_keep_only_the_last_turn_and_drop_speed_lines() {
        let screen = "1› first job\nold answer\n2› second job\nnew answer here\n3.2s 40 tps\n3›\n";
        assert_eq!(follow_up_report(screen).as_deref(), Some("new answer here"));
    }

    #[test]
    fn ring_composer_with_footer_is_found() {
        let screen = "❯ do it\nDone, the file is saved.\n───────\n❯ try: add tests\n☤ ring · model · 3m\n";
        assert_eq!(follow_up_report(screen).as_deref(), Some("Done, the file is saved."));
        let wide = "❯ do it\nDone again.\n───────\n❯\n───────\n☤ ring · model · ◷ 3m\n";
        assert_eq!(follow_up_report(wide).as_deref(), Some("Done again."));
    }

    #[test]
    fn input_empty_needs_a_prompt_and_no_spinner() {
        assert!(input_empty("output\n❯ \n"));
        assert!(!input_empty("output\n"));
        assert!(!input_empty("⠋ Working 3s\n❯\n"));
        assert!(!input_empty("output\n❯\na\nb\nc\nd\n"));
        assert!(input_empty("\x1b[32moutput\x1b[0m\n1>\n"));
    }

    #[test]
    fn observe_skips_the_baseline_and_checked_reports() {
        let mut follow = JobFollowUp::default();
        let before = "❯ old\nold reply here\n❯\n";
        follow.prepare("m4", before);
        let j = job(10, "fix it");
        follow.start(&j);
        assert!(follow.watching("m4"));
        assert_eq!(follow.observe(&j, before), None);
        let after = "❯ fix it\nFixed it, tests pass.\n❯\n";
        assert_eq!(follow.observe(&j, after).as_deref(), Some("❯ fix it\nFixed it, tests pass."));
        follow.checked(&j, after);
        assert_eq!(follow.observe(&j, after), None);
        let again = "❯ fix it\nFixed it, and the docs.\n❯\n";
        let seen = follow.observe(&j, again).expect("new report");
        follow.mark(&j, &seen);
        assert_eq!(follow.observe(&j, again), None);
        let other = job(20, "next");
        assert_eq!(follow.observe(&other, after), None);
        let newer = "❯ next\nNext part is done.\n❯\n";
        assert_eq!(follow.observe(&other, newer).as_deref(), Some("❯ next\nNext part is done."));
    }

    #[test]
    fn observe_without_start_sets_the_baseline_first() {
        let mut follow = JobFollowUp::default();
        let j = job(5, "go");
        let screen = "❯ go\nAll done.\n❯\n";
        assert_eq!(follow.observe(&j, screen), None);
        assert_eq!(follow.observe(&j, "❯ go\nAll done, and docs too.\n❯\n").as_deref(), Some("❯ go\nAll done, and docs too."));
    }

    #[test]
    fn keep_and_forget_drop_old_terminals() {
        let mut follow = JobFollowUp::default();
        follow.prepare("m1", "a\n❯\n");
        follow.start(&JobKey { terminal: "m1".into(), at: 1, sent: "x".into() });
        follow.prepare("m2", "b\n❯\n");
        let none = HashSet::new();
        let live: HashSet<String> = ["m2".to_string()].into();
        follow.keep(&none, &live);
        assert!(!follow.watching("m1"));
        follow.forget("m2");
        follow.start(&JobKey { terminal: "m2".into(), at: 2, sent: "y".into() });
        follow.clear();
        assert!(!follow.watching("m2"));
    }
}
