use crate::text::{clip_dots, hide_secrets, one_line, redact};
use crate::time::now_ms;
use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub const KEEP_GAPS: i64 = 40;
pub const GAP_ASKED_CHARS: usize = 160;
pub const GAP_REASON_CHARS: usize = 240;
pub const GAP_TOOL_CHARS: usize = 80;
const RAW_CHARS: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    Gap,
    Refusal,
}

impl GapKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GapKind::Gap => "gap",
            GapKind::Refusal => "refusal",
        }
    }

    pub fn parse(text: &str) -> Option<GapKind> {
        match text {
            "gap" => Some(GapKind::Gap),
            "refusal" => Some(GapKind::Refusal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapSource {
    Reply,
    Tool,
    Mcp,
}

impl GapSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GapSource::Reply => "reply",
            GapSource::Tool => "tool",
            GapSource::Mcp => "mcp",
        }
    }

    pub fn parse(text: &str) -> Option<GapSource> {
        match text {
            "reply" => Some(GapSource::Reply),
            "tool" => Some(GapSource::Tool),
            "mcp" => Some(GapSource::Mcp),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGap {
    pub id: i64,
    pub kind: GapKind,
    pub source: GapSource,
    pub tool: String,
    pub asked: String,
    pub reason: String,
    pub count: i64,
    pub first_at: i64,
    pub last_at: i64,
    #[serde(default)]
    pub task_id: i64,
    #[serde(default)]
    pub space_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapCause {
    Hold,
    Setup,
    Passing,
    Capability,
    Unknown,
}

impl GapCause {
    pub fn as_str(self) -> &'static str {
        match self {
            GapCause::Hold => "hold",
            GapCause::Setup => "setup",
            GapCause::Passing => "passing",
            GapCause::Capability => "capability",
            GapCause::Unknown => "unknown",
        }
    }
}

pub const TASK_SQL: &str = "ALTER TABLE tool_gaps ADD COLUMN task_id INTEGER NOT NULL DEFAULT 0;";

pub const SPACE_SQL: &str = r#"
CREATE TABLE tool_gaps_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    tool TEXT NOT NULL DEFAULT '',
    asked TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 1,
    first_at INTEGER NOT NULL,
    last_at INTEGER NOT NULL,
    task_id INTEGER NOT NULL DEFAULT 0,
    space_id TEXT NOT NULL DEFAULT '',
    UNIQUE (kind, source, tool, reason, space_id)
);
INSERT INTO tool_gaps_new (id, kind, source, tool, asked, reason, count, first_at, last_at, task_id)
    SELECT id, kind, source, tool, asked, reason, count, first_at, last_at, task_id FROM tool_gaps;
DROP TABLE tool_gaps;
ALTER TABLE tool_gaps_new RENAME TO tool_gaps;
"#;

pub const SPACE_ID_CHARS: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapInput {
    pub kind: GapKind,
    pub source: GapSource,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub asked: String,
    pub reason: String,
    #[serde(default)]
    pub space_id: String,
}

static NAMED_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b((?:api[_ -]?key|access[_ -]?token|token|password|passcode|secret|otp|pin)\s*(?:=|:|\bis\b)\s*)[^\s,;]+").expect("valid pattern")
});
static POLICY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:not allowed|must not|may not|only (?:you|the user|the owner)|your yes|did not say yes|needs? (?:a |the |your )?yes|waits? for (?:you|the user|the owner)|locked|on hold|is held|held for you|work hold|plan mode|privacy|private|confidential|not in the nixa space|another task|already (?:running|answered|done|open|claimed)|no open prompt|is busy|ask first)\b").expect("valid pattern")
});
static BROKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:ENOENT|ECONNREFUSED|ECONNRESET|ETIMEDOUT|EACCES|EPERM|EPIPE|ENOTFOUND|EHOSTUNREACH|timed out|time ?out|not connected|connection (?:closed|refused|reset|lost)|fetch failed|socket hang up|command not found|not installed|is not running|not reachable|unreachable|no tool|unknown tool|not supported|unsupported|not implemented|not available|unavailable|is not a function|cannot read propert\w+|crashed|could not prove|screen recording|exit(?:ed with)? code [1-9]\d*|(?:status|HTTP) (?:5\d\d|404)|missing (?:tool|command|program|binary|permission))\b").expect("valid pattern")
});

static PASSING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:ECONNRESET|ETIMEDOUT|EPIPE|EHOSTUNREACH|ENOTFOUND|timed out|time ?out|connection (?:closed|reset|lost)|fetch failed|socket hang up|(?:status|HTTP) (?:5\d\d|429)|rate limit(?:ed)?|too many requests|overloaded|temporarily|try again|crashed|is busy)\b").expect("valid pattern")
});
static SETUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:ENOENT|ECONNREFUSED|EACCES|EPERM|connection refused|command not found|not installed|not set up|not configured|no (?:api )?key|missing (?:key|token|permission|command|program|binary)|permission denied|screen recording|not signed in|sign in|log ?in|unauthori[sz]ed|(?:status|HTTP) (?:401|403)|not connected|is not running|not reachable|unreachable|turned off|switched off|is off|disabled)\b").expect("valid pattern")
});
static CAPABILITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:no tool|unknown tool|there is no tool|not supported|unsupported|not implemented|missing tool|none of my tools|my tools? (?:cannot|can't|do not|don't)|no (?:way|action|command|tool) (?:to|for)|(?:have no|do not have|don't have) (?:a |an |any )?(?:[a-z-]+ )?tools?|is not a function|(?:status|HTTP) 404)\b").expect("valid pattern")
});

pub fn gap_cause(kind: GapKind, reason: &str) -> GapCause {
    if kind == GapKind::Refusal {
        GapCause::Hold
    } else if PASSING.is_match(reason) {
        GapCause::Passing
    } else if SETUP.is_match(reason) {
        GapCause::Setup
    } else if CAPABILITY.is_match(reason) {
        GapCause::Capability
    } else {
        GapCause::Unknown
    }
}

pub fn clean_gap_text(text: &str, max: usize, secrets: &[String]) -> String {
    let raw = crate::text::clip(text, RAW_CHARS);
    let cleaned = redact(&hide_secrets(&raw, secrets));
    let named = NAMED_VALUE.replace_all(&cleaned, "${1}<redacted>");
    clip_dots(&one_line(&named), max)
}

pub fn gap_kind_of(reason: &str, refused: bool) -> GapKind {
    if refused || POLICY.is_match(reason) {
        GapKind::Refusal
    } else if BROKEN.is_match(reason) {
        GapKind::Gap
    } else {
        GapKind::Refusal
    }
}

fn to_gap(row: &Row) -> rusqlite::Result<ToolGap> {
    Ok(ToolGap {
        id: row.get("id")?,
        kind: GapKind::parse(&row.get::<_, String>("kind")?).unwrap_or(GapKind::Gap),
        source: GapSource::parse(&row.get::<_, String>("source")?).unwrap_or(GapSource::Tool),
        tool: row.get("tool")?,
        asked: row.get("asked")?,
        reason: row.get("reason")?,
        count: row.get("count")?,
        first_at: row.get("first_at")?,
        last_at: row.get("last_at")?,
        task_id: row.get("task_id")?,
        space_id: row.get("space_id")?,
    })
}

pub fn note_gap(conn: &Connection, input: &GapInput, secrets: &[String]) -> Result<Option<ToolGap>> {
    let reason = clean_gap_text(&input.reason, GAP_REASON_CHARS, secrets);
    if reason.is_empty() {
        return Ok(None);
    }
    let tool = if input.tool.is_empty() { String::new() } else { clean_gap_text(&input.tool, GAP_TOOL_CHARS, secrets) };
    let asked = if input.asked.is_empty() { String::new() } else { clean_gap_text(&input.asked, GAP_ASKED_CHARS, secrets) };
    let space = crate::text::clip(input.space_id.trim(), SPACE_ID_CHARS);
    let at = now_ms();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO tool_gaps (kind, source, tool, asked, reason, count, first_at, last_at, space_id) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6, ?7)
         ON CONFLICT(kind, source, tool, reason, space_id) DO UPDATE SET count = count + 1, last_at = excluded.last_at,
           asked = CASE WHEN excluded.asked <> '' THEN excluded.asked ELSE asked END",
        params![input.kind.as_str(), input.source.as_str(), tool, asked, reason, at, space],
    )?;
    tx.execute(
        "DELETE FROM tool_gaps WHERE kind = ?1 AND id NOT IN (SELECT id FROM tool_gaps WHERE kind = ?1 ORDER BY last_at DESC, id DESC LIMIT ?2)",
        params![input.kind.as_str(), KEEP_GAPS],
    )?;
    let gap = tx
        .query_row(
            "SELECT * FROM tool_gaps WHERE kind = ?1 AND source = ?2 AND tool = ?3 AND reason = ?4 AND space_id = ?5",
            params![input.kind.as_str(), input.source.as_str(), tool, reason, space],
            to_gap,
        )
        .optional()?;
    tx.commit()?;
    Ok(gap)
}

pub fn note_tool_problem(conn: &Connection, tool: &str, reason: &str, source: GapSource, refused: bool, space_id: &str, secrets: &[String]) -> Result<Option<ToolGap>> {
    let kind = gap_kind_of(reason, refused);
    let input = GapInput { kind, source, tool: tool.to_string(), asked: String::new(), reason: reason.to_string(), space_id: space_id.to_string() };
    note_gap(conn, &input, secrets)
}

pub fn list_gaps(conn: &Connection, kind: Option<GapKind>) -> Result<Vec<ToolGap>> {
    let mut stmt = conn.prepare("SELECT * FROM tool_gaps WHERE ?1 = '' OR kind = ?1 ORDER BY last_at, id")?;
    let wanted = kind.map(|k| k.as_str()).unwrap_or("");
    let rows = stmt.query_map(params![wanted], to_gap)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_gap(conn: &Connection, id: i64) -> Result<Option<ToolGap>> {
    Ok(conn.query_row("SELECT * FROM tool_gaps WHERE id = ?1", params![id], to_gap).optional()?)
}

pub fn set_gap_task(conn: &Connection, id: i64, task_id: i64) -> Result<bool> {
    Ok(conn.execute("UPDATE tool_gaps SET task_id = ?2 WHERE id = ?1", params![id, task_id])? == 1)
}

pub fn remove_gap(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM tool_gaps WHERE id = ?1", params![id])? == 1)
}

pub fn clear_gaps(conn: &Connection, kind: Option<GapKind>) -> Result<usize> {
    let wanted = kind.map(|k| k.as_str()).unwrap_or("");
    Ok(conn.execute("DELETE FROM tool_gaps WHERE ?1 = '' OR kind = ?1", params![wanted])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    #[test]
    fn kinds() {
        assert_eq!(gap_kind_of("ECONNREFUSED on port", false), GapKind::Gap);
        assert_eq!(gap_kind_of("that is private", false), GapKind::Refusal);
        assert_eq!(gap_kind_of("something odd", false), GapKind::Refusal);
        assert_eq!(gap_kind_of("command not found", true), GapKind::Refusal);
    }

    #[test]
    fn causes() {
        assert_eq!(gap_cause(GapKind::Refusal, "no tool for that"), GapCause::Hold);
        assert_eq!(gap_cause(GapKind::Gap, "request timed out"), GapCause::Passing);
        assert_eq!(gap_cause(GapKind::Gap, "HTTP 503 from the server"), GapCause::Passing);
        assert_eq!(gap_cause(GapKind::Gap, "ECONNREFUSED"), GapCause::Setup);
        assert_eq!(gap_cause(GapKind::Gap, "Screen recording is not set up."), GapCause::Setup);
        assert_eq!(gap_cause(GapKind::Gap, "There is no tool named nixa_calendar."), GapCause::Capability);
        assert_eq!(gap_cause(GapKind::Gap, "I have no tool for the calendar."), GapCause::Capability);
        assert_eq!(gap_cause(GapKind::Gap, "export is not supported"), GapCause::Capability);
        assert_eq!(gap_cause(GapKind::Gap, "I could not open the file."), GapCause::Unknown);
    }

    #[test]
    fn task_link_is_kept() {
        let conn = temp_db("gap-task");
        let input = GapInput { kind: GapKind::Gap, source: GapSource::Tool, tool: "cal".into(), asked: "".into(), reason: "not supported".into(), space_id: "shop".into() };
        let gap = note_gap(&conn, &input, &[]).unwrap().unwrap();
        assert_eq!((gap.task_id, gap.space_id.as_str()), (0, "shop"));
        let other = note_gap(&conn, &GapInput { space_id: "home".into(), ..input.clone() }, &[]).unwrap().unwrap();
        assert_ne!(other.id, gap.id);
        assert_eq!((other.space_id.as_str(), other.count), ("home", 1));
        assert!(set_gap_task(&conn, gap.id, 9).unwrap());
        assert_eq!(note_gap(&conn, &input, &[]).unwrap().unwrap().task_id, 9);
        assert_eq!(get_gap(&conn, gap.id).unwrap().unwrap().task_id, 9);
        assert!(get_gap(&conn, gap.id + 100).unwrap().is_none());
        assert!(!set_gap_task(&conn, gap.id + 100, 9).unwrap());
    }

    #[test]
    fn old_rows_keep_their_id_and_task_after_the_space_migration() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrate_to(&conn, 7).unwrap();
        conn.execute("INSERT INTO tool_gaps (kind, source, tool, asked, reason, count, first_at, last_at, task_id) VALUES ('gap', 'tool', 'cal', '', 'not supported', 3, 1, 2, 5)", []).unwrap();
        let old = conn.last_insert_rowid();
        crate::db::migrate(&conn).unwrap();
        let gap = get_gap(&conn, old).unwrap().unwrap();
        assert_eq!((gap.count, gap.task_id, gap.space_id.as_str()), (3, 5, ""));
        let input = GapInput { kind: GapKind::Gap, source: GapSource::Tool, tool: "cal".into(), asked: "".into(), reason: "not supported".into(), space_id: String::new() };
        let again = note_gap(&conn, &input, &[]).unwrap().unwrap();
        assert_eq!((again.id, again.count), (old, 4));
        let fresh = note_gap(&conn, &GapInput { reason: "unknown tool".into(), ..input }, &[]).unwrap().unwrap();
        assert!(fresh.id > old);
    }

    #[test]
    fn clean_text() {
        assert_eq!(clean_gap_text("the password is hunter22  and\nmore", 240, &[]), "the password is <redacted> and more");
        assert_eq!(clean_gap_text(&"x".repeat(300), 10, &[]).chars().count(), 10);
    }

    #[test]
    fn note_count_keep_list() {
        let conn = temp_db("gaps");
        let input = GapInput { kind: GapKind::Gap, source: GapSource::Tool, tool: "screen_read".into(), asked: "".into(), reason: "screen recording is off".into(), space_id: String::new() };
        let first = note_gap(&conn, &input, &[]).unwrap().unwrap();
        assert_eq!(first.count, 1);
        let second = note_gap(&conn, &GapInput { asked: "arguments: id".into(), ..input.clone() }, &[]).unwrap().unwrap();
        assert_eq!(second.count, 2);
        assert_eq!(second.asked, "arguments: id");
        assert_eq!(second.id, first.id);
        let refused = note_tool_problem(&conn, "send", "not allowed here", GapSource::Mcp, false, "", &[]).unwrap().unwrap();
        assert_eq!(refused.kind, GapKind::Refusal);
        assert!(refused.asked.is_empty());
        for n in 0..45 {
            note_gap(&conn, &GapInput { reason: format!("timed out {n}"), ..input.clone() }, &[]).unwrap();
        }
        assert_eq!(list_gaps(&conn, Some(GapKind::Gap)).unwrap().len() as i64, KEEP_GAPS);
        assert_eq!(list_gaps(&conn, Some(GapKind::Refusal)).unwrap().len(), 1);
        assert_eq!(list_gaps(&conn, None).unwrap().len() as i64, KEEP_GAPS + 1);
        assert!(note_gap(&conn, &GapInput { reason: "  ".into(), ..input.clone() }, &[]).unwrap().is_none());
        assert!(remove_gap(&conn, refused.id).unwrap());
        assert_eq!(clear_gaps(&conn, None).unwrap() as i64, KEEP_GAPS);
    }
}
