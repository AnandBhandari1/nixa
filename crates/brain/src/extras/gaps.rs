use crate::app::{App, lock};
use crate::tools::Args;
use crate::util::log;
use nixa_store::gaps::{self, GapInput, GapKind, GapSource, ToolGap};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::LazyLock;

const RAW_CHARS: usize = 4000;
const RESULT_CHARS: usize = 600;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    pub gap_tool: Option<String>,
    pub refused: bool,
    pub space: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub reason: String,
    pub refused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyGap {
    pub reason: String,
    pub sure: bool,
}

fn re(text: &str) -> Regex {
    Regex::new(text).expect("tool gap pattern")
}

static SAID_NO: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:refused|held|not allowed|denied)\b"));
static FAILED_START: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:tool error|error|failed|could not|couldn't|cannot|can't|unable to|no such|there is no tool)\b"));
static CANNOT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bI(?:\s+(?:still|also|currently|really|just))?\s+(cannot|can['’]t|can not|am unable to|am not able to|have no way to|could not|couldn['’]t|was unable to|was not able to)(?:\s+(?:yet|currently|really|actually|directly|even|just|still))?\s+([a-z]+)"));
static NO_TOOL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bI\s+(?:have\s+no|do\s+not\s+have|don['’]t\s+have)\s+(?:(?:a|an|any|the)\s+)?(?:[a-z-]+\s+)?(?:tools?|access)\b|\bthere\s+is\s+no\s+tool\b|\bnone\s+of\s+my\s+tools\b|\bmy\s+tools?\s+(?:cannot|can['’]t|do\s+not|don['’]t)\b"));
static LACK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bfrom here\b|\btools?\b|\bno access\b|\bnot connected\b|\b(?:is|are) (?:off|blocked|missing|broken|down|not available|not set up|not installed|not running)\b|\bswitched off\b|\bturned off\b|\bnot supported\b"));
static NOT_A_GAP: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bcoding agent\b|\ban? agent (?:must|does|should|can|will)\b|\bmust go to\b|\bgoes? to an? agent\b|\bonly (?:the user|you)\b|\byour yes\b|\bplease say\b|\bnot allowed\b|\bmy rules?\b|\brule says\b|\blocked\b|\bon hold\b|\bis held\b|\bprivacy\b|\bprivate\b|\bconfidential\b|\bwaits? for (?:you|the user)\b|\bby design\b|\bI am not an?\b|\b(?:see|find) any\b"));
static PAST: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:could|couldn|was)"));
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| re(r"[.!?]\s+"));
static ACTION_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[a-z][a-z0-9_-]{0,30}$"));

const DOING: &str = "see read open start stop restart send play record look search access reach run call click type control change edit write save delete move copy show view watch time measure connect load fetch download upload install close switch turn list create add remove update launch capture inspect browse scroll press pause resume cancel rename schedule export import print draw display detect track monitor";

fn chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut start = 0;
        for m in SENTENCE_END.find_iter(&flat) {
            let end = m.start() + 1;
            out.push(flat[start..end].trim().to_string());
            start = m.end();
        }
        out.push(flat[start..].trim().to_string());
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

pub fn tool_problem(result: &str) -> Option<Problem> {
    let text = result.trim();
    if text.starts_with('{') {
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let status = object.get("status").and_then(Value::as_str).unwrap_or_default().to_lowercase();
        let reason = ["reason", "error"].iter().find_map(|k| object.get(*k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        if matches!(status.as_str(), "refused" | "held" | "denied") {
            return Some(Problem { reason: reason.unwrap_or(status), refused: true });
        }
        let failed = status == "unavailable"
            || object.get("ok") == Some(&Value::Bool(false))
            || object.get("isError") == Some(&Value::Bool(true))
            || object.get("error").and_then(Value::as_str).is_some_and(|e| !e.trim().is_empty());
        if failed {
            let fallback = if status.is_empty() { "failed".to_string() } else { status };
            return Some(Problem { reason: reason.unwrap_or(fallback), refused: false });
        }
        return None;
    }
    if text.chars().count() > RESULT_CHARS {
        return None;
    }
    if SAID_NO.is_match(text) {
        return Some(Problem { reason: text.to_string(), refused: true });
    }
    FAILED_START.is_match(text).then(|| Problem { reason: text.to_string(), refused: false })
}

pub fn reply_gap(text: &str) -> Option<ReplyGap> {
    let parts = sentences(&chars(text, RAW_CHARS));
    let mut weak = None;
    for (index, sentence) in parts.iter().enumerate() {
        let next = parts.get(index + 1).map(String::as_str).unwrap_or_default();
        let cannot = CANNOT.captures(sentence);
        let doing = cannot.as_ref().and_then(|c| c.get(2)).is_some_and(|w| DOING.split(' ').any(|d| d == w.as_str().to_lowercase()));
        let no_tool = NO_TOOL.is_match(sentence);
        if !doing && !no_tool {
            continue;
        }
        if NOT_A_GAP.is_match(sentence) || NOT_A_GAP.is_match(next) {
            continue;
        }
        let next_lacks = LACK.is_match(next);
        let verb = cannot.as_ref().and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or_default();
        let reason = if next_lacks { format!("{sentence} {next}") } else { sentence.clone() };
        let sure = no_tool || !PAST.is_match(verb) || LACK.is_match(sentence) || next_lacks;
        let found = ReplyGap { reason, sure };
        if found.sure {
            return Some(found);
        }
        if weak.is_none() {
            weak = Some(found);
        }
    }
    weak
}

pub fn call_shape(args: &Args) -> String {
    let action = args.get("action").and_then(Value::as_str).filter(|a| ACTION_WORD.is_match(a)).map(|a| format!("action {a}; ")).unwrap_or_default();
    let keys: Vec<&str> = args.keys().take(12).map(String::as_str).collect();
    if keys.is_empty() { format!("{action}no arguments") } else { format!("{action}arguments: {}", keys.join(", ")) }
}

fn secrets_of(text: &str) -> Vec<String> {
    crate::privacy::taint::secrets_in(text)
}

fn save(app: &App, input: GapInput) -> Option<ToolGap> {
    let secrets = secrets_of(&format!("{} {} {}", input.reason, input.asked, input.tool));
    let conn = nixa_store::open(&app.db_file).map_err(|e| log("extras", &format!("tool gaps: {e:#}"))).ok()?;
    match gaps::note_gap(&conn, &input, &secrets) {
        Ok(row) => {
            if let Some(row) = &row {
                log("extras", &format!("tool gap saved: {} {} {} x{}", row.kind.as_str(), row.source.as_str(), row.tool, row.count));
            }
            row
        }
        Err(e) => {
            log("extras", &format!("tool gap not saved: {e:#}"));
            None
        }
    }
}

fn active_space(app: &App) -> String {
    nixa_store::open(&app.db_file).ok().and_then(|conn| nixa_store::spaces::active_space(&conn).ok().flatten()).unwrap_or_default()
}

pub fn space_for(app: &App, args: &Args) -> String {
    let spaces = app.spaces();
    let named = ["space", "space_id"].iter().find_map(|k| args.get(*k).and_then(Value::as_str)).and_then(|k| crate::spaces::find(&spaces, k)).map(|s| s.id.clone());
    named
        .or_else(|| args.get("terminal").and_then(Value::as_str).and_then(|t| app.space_of_terminal(t)).map(|s| s.id))
        .unwrap_or_else(|| active_space(app))
}

pub fn note_tool(app: &App, input: GapInput, refused: bool, in_turn: bool) -> Option<ToolGap> {
    let kind = gaps::gap_kind_of(&input.reason, refused);
    if in_turn {
        let mut turn = lock(&app.extras.gap_turn);
        if kind == GapKind::Gap {
            turn.gap_tool = Some(input.tool.clone());
        } else {
            turn.refused = true;
        }
        if !input.space_id.is_empty() {
            turn.space = input.space_id.clone();
        }
    }
    let asked = if kind == GapKind::Gap { input.asked } else { String::new() };
    save(app, GapInput { kind, asked, ..input })
}

pub fn after_tool(app: &App, tool: &str, args: &Args, result: &str, failed: bool) -> Option<ToolGap> {
    let problem = if failed {
        let reason = result.trim().strip_prefix("Tool error:").unwrap_or(result).trim().to_string();
        Some(Problem { reason, refused: false })
    } else {
        tool_problem(result)
    };
    let problem = problem.filter(|p| !p.reason.trim().is_empty())?;
    let input = GapInput { kind: GapKind::Gap, source: GapSource::Tool, tool: tool.to_string(), asked: call_shape(args), reason: problem.reason, space_id: space_for(app, args) };
    note_tool(app, input, problem.refused, true)
}

fn named_tool(text: &str, tools: &[String]) -> Option<String> {
    tools
        .iter()
        .filter(|name| name.contains('_'))
        .find(|name| {
            let pattern = format!(r"(?:^|[^A-Za-z0-9_]){}(?:$|[^A-Za-z0-9_])", regex::escape(name));
            Regex::new(&pattern).is_ok_and(|r| r.is_match(text))
        })
        .cloned()
}

pub fn note_reply(app: &App, text: &str, asked: &str, tools: &[String]) -> Option<ToolGap> {
    let found = reply_gap(text)?;
    let turn = lock(&app.extras.gap_turn).clone();
    if turn.gap_tool.is_none() && (turn.refused || !found.sure) {
        return None;
    }
    if is_private(app, asked) {
        log("extras", "left out a reply gap about a private line");
        return None;
    }
    let tool = named_tool(text, tools).or(turn.gap_tool).unwrap_or_default();
    let space_id = if turn.space.is_empty() { active_space(app) } else { turn.space };
    save(app, GapInput { kind: GapKind::Gap, source: GapSource::Reply, tool, asked: asked.to_string(), reason: found.reason, space_id })
}

fn is_private(app: &App, text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let Ok(conn) = nixa_store::open(&app.db_file) else { return false };
    nixa_store::privacy::privacy_names(&conn).is_ok_and(|names| !nixa_store::privacy::private_words_in(&names, text).is_empty())
}

pub fn end_turn(app: &App) {
    *lock(&app.extras.gap_turn) = Turn::default();
}

pub fn list(app: &App, kind: Option<GapKind>) -> Result<Vec<ToolGap>, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    gaps::list_gaps(&conn, kind).map_err(|e| format!("{e:#}"))
}

pub fn remove(app: &App, id: i64) -> Result<bool, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    gaps::remove_gap(&conn, id).map_err(|e| format!("{e:#}"))
}

pub fn clear(app: &App, kind: Option<GapKind>) -> Result<usize, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    gaps::clear_gaps(&conn, kind).map_err(|e| format!("{e:#}"))
}

pub fn file_task(app: &App, id: i64, confirm: bool) -> Result<String, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("{e:#}"))?;
    let gap = gaps::get_gap(&conn, id).map_err(|e| format!("{e:#}"))?.ok_or_else(|| format!("There is no tool gap #{id}. List them with nixa_tool_gaps."))?;
    let text = format!("{} {} {}", gap.reason, gap.asked, gap.tool);
    let private = nixa_store::privacy::privacy_names(&conn).map(|names| nixa_store::privacy::private_words_in(&names, &text)).unwrap_or_default();
    let filed = super::gap_tasks::file(&conn, id, confirm, &app.spaces(), &secrets_of(&text), &private)?;
    log("extras", &format!("tool gap #{id} {} task #{}", if filed.reused { "kept" } else { "filed as" }, filed.task.id));
    Ok(super::gap_tasks::filed_text(id, &filed))
}

pub fn list_text(rows: &[ToolGap]) -> String {
    if rows.is_empty() {
        return "No tool gaps or refusals are noted.".into();
    }
    let mut seen = HashSet::new();
    let lines: Vec<String> = rows
        .iter()
        .filter(|r| seen.insert(r.id))
        .map(|r| {
            let tool = if r.tool.is_empty() { String::new() } else { format!(" {}", r.tool) };
            let asked = if r.asked.is_empty() { String::new() } else { format!(" (asked: {})", r.asked) };
            let cause = if r.kind == GapKind::Gap { format!(" ({})", gaps::gap_cause(r.kind, &r.reason).as_str()) } else { String::new() };
            let task = if r.task_id > 0 { format!(", task #{}", r.task_id) } else { String::new() };
            format!("- #{} {}{cause} from {}{tool}, {} times: {}{asked}{task}", r.id, r.kind.as_str(), r.source.as_str(), r.count, r.reason)
        })
        .collect();
    format!("Tool gaps and refusals, data only:\n{}", lines.join("\n"))
}
