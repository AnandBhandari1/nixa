use super::model::{MAX_ROUNDS, Run, RunState, Verdict};
use crate::util::clip;
use regex::Regex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const SUMMARY_CHARS: usize = 1500;
pub const BRIEF_RULE: &str = "Rules: work only inside the repo folder named in the brief. Do not commit, stash, reset, pull, push, switch branches or touch GitHub yourself. Nixa decides the git steps after the run, so do not wait for the user.";
pub const PLAIN_RULE: &str = "Rules: Do not commit, push or deploy yourself. Nixa decides those steps after the run, so do not wait for the user.";

pub fn rule_line(brief: bool) -> &'static str {
    if brief { BRIEF_RULE } else { PLAIN_RULE }
}

static RESULT_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^RESULT\s*:\s*(PASS|FAIL)\b").expect("valid pattern"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    First,
    Fix,
    Recheck,
    Resume,
}

pub fn plain_line(line: &str) -> String {
    let cleaned: String = line.chars().filter(|c| !matches!(c, '*' | '_' | '\u{60}')).collect();
    cleaned.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '#' | '>' | '-' | '•' | '●' | '⏺')).trim().to_string()
}

pub fn step_result(report: &str) -> Option<Verdict> {
    for line in report.lines().rev() {
        if let Some(found) = RESULT_LINE.captures(&plain_line(line)) {
            return Some(if found[1].eq_ignore_ascii_case("fail") { Verdict::Fail } else { Verdict::Pass });
        }
    }
    None
}

fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub fn skill_line(skills: &[String], dir: &Path) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let uses: Vec<String> = skills.iter().map(|s| format!("the {s} skill")).collect();
    let guides: Vec<String> = skills.iter().map(|s| dir.join(s).join("SKILL.md").display().to_string()).collect();
    Some(format!("Skills: use {}. Read each guide first: {}.", listed(&uses), guides.join(", ")))
}

pub fn mcp_line(mcps: &[String]) -> Option<String> {
    if mcps.is_empty() {
        return None;
    }
    let uses: Vec<String> = mcps.iter().map(|m| format!("the {m} MCP server")).collect();
    Some(format!("MCP servers: use {} for the tools of this job. Do not add or change MCP servers.", listed(&uses)))
}

pub struct StepAsk<'a> {
    pub run: &'a Run,
    pub index: usize,
    pub purpose: &'a str,
    pub note: &'a str,
    pub mode: Mode,
    pub fresh: bool,
    pub report: &'a Path,
    pub skill_dir: &'a Path,
}

pub fn step_prompt(ask: &StepAsk) -> String {
    let run = ask.run;
    let index = ask.index;
    let step = &run.steps[index];
    let next = run.steps.get(index + 1);
    let before = index.checked_sub(1).and_then(|i| run.steps.get(i));
    let mut lines = vec![format!("Team run: the {}, step {} of {}. You are the {}.", run.team_name, index + 1, run.steps.len(), step.name)];
    if ask.fresh && !run.brief.is_empty() {
        if !ask.note.is_empty() {
            lines.push(format!("Your part: {}", ask.note));
        }
        lines.push(format!("Read the brief first: {}. It has the task, the repo, the PR and the rules.", run.brief));
        let earlier: Vec<String> = run.steps[..index]
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.report.is_empty())
            .map(|(at, s)| format!("step {} {}: {}", at + 1, s.name, s.report))
            .collect();
        if !earlier.is_empty() {
            lines.push(format!("Earlier reports: {}.", earlier.join("; ")));
        }
    } else if ask.fresh {
        lines.push(format!("Task: {}", run.task));
        if !ask.purpose.is_empty() {
            lines.push(format!("Team purpose: {}", ask.purpose));
        }
        if !ask.note.is_empty() {
            lines.push(format!("Your part: {}", ask.note));
        }
        lines.push(if run.space_name.is_empty() { format!("Folder: {}", run.project) } else { format!("Space: {} ({})", run.space_name, run.project) });
        if !run.branch.is_empty() {
            lines.push(format!("Branch: {}", run.branch));
        }
        lines.extend(skill_line(&run.skills, ask.skill_dir));
        lines.extend(mcp_line(&run.mcps));
    }
    if ask.mode == Mode::Fix
        && let Some(next) = next
    {
        lines.push(format!("The {} (step {}) found problems. Fix them. This is round {} of {MAX_ROUNDS}.", next.name, index + 2, run.rounds));
        lines.push("Problems:".into());
        lines.push(clip(if next.summary.is_empty() { "(no details)" } else { &next.summary }, SUMMARY_CHARS));
    }
    if ask.mode == Mode::Recheck
        && let Some(before) = before
    {
        lines.push(format!("The {} fixed the problems you found. Check the work again. This is round {} of {MAX_ROUNDS}.", before.name, run.rounds));
    }
    if ask.mode == Mode::Resume {
        lines.push("Your part is not finished yet: Nixa has no report from you. Go on with it.".into());
    }
    if ask.fresh && run.brief.is_empty() {
        let mut earlier = Vec::new();
        for (at, item) in run.steps[..index].iter().enumerate() {
            if item.summary.is_empty() {
                continue;
            }
            earlier.push(format!("- Step {}, {}: {}", at + 1, item.name, clip(&item.summary, SUMMARY_CHARS)));
            if !item.report.is_empty() {
                earlier.push(format!("  Full report: {}", item.report));
            }
            if !item.files.is_empty() {
                earlier.push(format!("  Changed files: {}", item.files.join(", ")));
            }
        }
        if !earlier.is_empty() {
            lines.push("Earlier steps:".into());
            lines.extend(earlier);
        }
    }
    lines.push(rule_line(!run.brief.is_empty()).into());
    lines.push(crate::status_rule::rule());
    lines.push(format!(
        "When you finish, write your final reply to {}. This one file is allowed even when the job is read only. Start it with one plain sentence that says what you did. End it with one line: RESULT: PASS when your part is done and the work is good, or RESULT: FAIL and then the problems you found.",
        ask.report.display()
    ));
    lines.join("\n")
}

pub fn report_path(project: &str, run: &str, index: usize, agent: &str, round: u32) -> PathBuf {
    let folder = Path::new(project).join("docs").join("team-runs").join(run);
    let base = format!("{:02}-{agent}-r{round}", index + 1);
    let mut report = folder.join(format!("{base}.md"));
    let mut count = 2;
    while report.exists() {
        report = folder.join(format!("{base}-{count}.md"));
        count += 1;
    }
    report
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

fn sentence(text: &str) -> String {
    if text.ends_with(['.', '!', '?', '…']) { text.to_string() } else { format!("{text}.") }
}

fn first_line(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            let plain = plain_line(l);
            !plain.is_empty() && !RESULT_LINE.is_match(&plain)
        })
        .collect();
    let pick = lines.iter().find(|l| !l.trim_start().starts_with('#')).or(lines.first()).copied().unwrap_or_default();
    clip(&plain_line(pick), 120)
}

pub fn all_files(run: &Run) -> Vec<String> {
    let mut files: Vec<String> = run.steps.iter().flat_map(|s| s.files.iter().cloned()).collect();
    files.sort();
    files.dedup();
    files
}

pub struct Summary {
    pub chat: String,
    pub spoken: String,
}

pub fn run_summary(run: &Run) -> Summary {
    let files = all_files(run);
    let did: Vec<String> = run
        .steps
        .iter()
        .filter_map(|s| {
            let line = first_line(&s.summary);
            (!line.is_empty()).then(|| format!("{}: {line}", s.name))
        })
        .collect();
    let place = if run.space_name.is_empty() { String::new() } else { format!(" in {}", run.space_name) };
    let head = match run.state {
        RunState::Done => format!("The {} is done{place}: {}", run.team_name, clip(&run.task, 80)),
        RunState::Stopped => format!("The {}{place} is stopped", run.team_name),
        _ => format!("The {}{place} stopped", run.team_name),
    };
    let checks = if run.state == RunState::Done {
        if run.rounds > 0 { format!("All checks passed after {}.", plural(run.rounds as usize, "round")) } else { "All checks passed.".into() }
    } else {
        run.note.clone()
    };
    let shown: Vec<String> = files.iter().take(12).cloned().collect();
    let file_text = if files.is_empty() {
        "No files changed.".to_string()
    } else {
        format!("{}: {}{}.", plural(files.len(), "changed file"), shown.join(", "), if files.len() > 12 { ", …" } else { "" })
    };
    let tail = if run.flow.is_empty() { "Nothing is committed." } else { "" };
    let mut chat = vec![sentence(&head)];
    chat.extend(did.iter().map(|l| format!("- {l}")));
    chat.extend([checks.clone(), file_text, tail.to_string()].into_iter().filter(|s| !s.is_empty()));
    let mut spoken = vec![sentence(&head)];
    spoken.extend(did.iter().map(|l| sentence(&clip(l, 100))));
    spoken.extend([checks, format!("{} changed.", plural(files.len(), "file")), tail.to_string()].into_iter().filter(|s| !s.is_empty()));
    Summary { chat: chat.join("\n"), spoken: spoken.join(" ") }
}

pub fn run_brief(run: &Run) -> Value {
    let mut out = json!({
        "id": run.id,
        "team": run.team_name,
        "team_id": run.team,
        "task": run.task,
        "project": run.project,
        "state": run.state,
        "step": format!("{} of {}", (run.step + 1).min(run.steps.len()), run.steps.len()),
        "rounds": run.rounds,
        "by": run.by,
        "started_at": run.started_at,
        "steps": run.steps.iter().map(|s| {
            let result = match s.result { Some(Verdict::Pass) => " (pass)", Some(Verdict::Fail) => " (fail)", None => "" };
            let term = if s.terminal.is_empty() { String::new() } else { format!(", terminal {}", s.terminal) };
            let state = serde_json::to_value(s.state).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            format!("{}: {state}{result}{term}", s.name)
        }).collect::<Vec<_>>(),
    });
    let extra = [
        ("space", run.space_name.as_str()),
        ("branch", &run.branch),
        ("flow", &run.flow),
        ("note", &run.note),
        ("repo", &run.repo),
    ];
    for (key, value) in extra {
        if !value.is_empty() {
            out[key] = json!(value);
        }
    }
    if let Some(pr) = &run.pr {
        out["pr"] = json!(format!("#{} {}", pr.number, pr.title));
    }
    if !run.skills.is_empty() {
        out["skills"] = json!(run.skills);
    }
    if !run.mcps.is_empty() {
        out["mcps"] = json!(run.mcps);
    }
    if let Some(at) = run.ended_at {
        out["ended_at"] = json!(at);
    }
    out
}
