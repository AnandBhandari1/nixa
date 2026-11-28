use crate::agent_map::{self, AgentMap, AgentMapRow, DEFAULT_CRITERIA, FALLOVER_ORDER};
use crate::text::clean_line;
use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::LazyLock;

const STOP: [&str; 42] = [
    "about", "after", "agent", "agents", "also", "before", "being", "check", "does", "done", "each", "every", "from", "have", "into", "just", "keep", "make", "nixa",
    "more", "must", "only", "other", "same", "should", "show", "that", "their", "them", "then", "there", "these", "they", "this", "when", "where", "which", "while", "with",
    "without", "work", "your",
];
const EFFORTS: [&str; 7] = ["low", "medium", "high", "xhigh", "max", "default", "config"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MapPick {
    pub member: String,
    pub backup: bool,
    pub agent: String,
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RowMatch {
    pub row: AgentMapRow,
    pub number: usize,
    pub score: usize,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("map pattern")
}

static NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Z][A-Za-z]+(?: [A-Z][A-Za-z]+)*"));
static WITH: LazyLock<Regex> = LazyLock::new(|| re(r"^ with ([a-z][a-z-]*) ([^\s,;()]+)"));
static BACKUP_FLAG: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z ]+ \(backup\)"));
static STEM: LazyLock<Regex> = LazyLock::new(|| re(r"[a-z][a-z0-9]{3,}"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^##\s+(.+)$"));
static ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+)\.\s+(.*)$"));
static MORE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s+\S"));
static STOP_SECTION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(big feature flow|rules|start commands)$"));
static BACKUP_PART: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:second )?backup:?\s*(.*)$"));
static PLACE_PART: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:tab|split pane)\b"));

pub fn pick_of_row(row: &AgentMapRow) -> String {
    if row.member.is_empty() {
        return row.pick.clone();
    }
    let model = if !row.agent.is_empty() && !row.model.is_empty() {
        let effort = if row.effort.is_empty() { String::new() } else { format!(" {}", row.effort) };
        format!(" with {} {}{effort}", row.agent, row.model)
    } else {
        String::new()
    };
    format!("{}{}{model}", row.member, if row.backup { " (backup)" } else { "" })
}

pub fn row_line(row: &AgentMapRow, number: usize) -> String {
    let pick = pick_of_row(row);
    let mut parts = vec![format!("Row {number}: {}", row.job), format!("pick {}", if pick.is_empty() { "none" } else { &pick })];
    if !row.fallback.is_empty() {
        parts.push(format!("backup {}", row.fallback));
    }
    if !row.place.is_empty() {
        parts.push(row.place.clone());
    }
    if !row.notes.is_empty() {
        parts.push(row.notes.clone());
    }
    parts.join(" · ")
}

fn ends_ok(rest: &str) -> bool {
    rest.is_empty() || rest.starts_with([' ', ',', ';', '('])
}

fn tail(rest: &str) -> Option<(String, String, String)> {
    if let Some(found) = WITH.captures(rest) {
        let after = &rest[found[0].len()..];
        if let Some(word) = after.strip_prefix(' ') {
            for effort in EFFORTS {
                if let Some(more) = word.strip_prefix(effort)
                    && ends_ok(more)
                {
                    return Some((found[1].to_string(), found[2].to_string(), effort.to_string()));
                }
            }
        }
        if ends_ok(after) {
            return Some((found[1].to_string(), found[2].to_string(), String::new()));
        }
    }
    ends_ok(rest).then(|| (String::new(), String::new(), String::new()))
}

pub fn parse_pick(pick: &str) -> Option<MapPick> {
    let text = pick.trim();
    let name = NAME.find(text)?.as_str();
    let words: Vec<&str> = name.split(' ').collect();
    for count in (1..=words.len()).rev() {
        let member = words[..count].join(" ");
        let rest = &text[member.len()..];
        let mut tries = Vec::new();
        if let Some(after) = rest.strip_prefix(" (backup)") {
            tries.push(after);
        }
        tries.push(rest);
        for rest in tries {
            if let Some((agent, model, effort)) = tail(rest) {
                return Some(MapPick {
                    member,
                    backup: BACKUP_FLAG.is_match(text),
                    agent,
                    model: model.trim_end_matches('.').to_string(),
                    effort,
                });
            }
        }
    }
    None
}

fn sentences(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '.' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j > i + 1 && j < chars.len() && (chars[j].is_ascii_uppercase() || chars[j] == '\u{60}' || chars[j] == '(') {
                parts.push(chars[start..i].iter().collect());
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    parts.push(chars[start..].iter().collect());
    parts.into_iter().map(|p| p.trim().trim_end_matches('.').to_string()).filter(|p| !p.is_empty()).collect()
}

pub fn import_map_md(source: &str) -> AgentMap {
    let mut map = AgentMap {
        version: 1,
        rows: Vec::new(),
        criteria: DEFAULT_CRITERIA.iter().map(|s| s.to_string()).collect(),
        fallover: FALLOVER_ORDER.iter().map(|s| s.to_string()).collect(),
        at: String::new(),
    };
    let mut section = String::new();
    let mut stop = false;
    let mut rules_on = false;
    let mut items: Vec<(String, String, String)> = Vec::new();
    let mut rules: Vec<String> = Vec::new();
    for line in source.lines() {
        if let Some(heading) = HEADING.captures(line) {
            section = heading[1].trim().to_string();
            rules_on = section.eq_ignore_ascii_case("rules");
            if STOP_SECTION.is_match(&section) {
                stop = true;
            }
            if section.eq_ignore_ascii_case("start commands") {
                rules_on = false;
            }
            continue;
        }
        if rules_on {
            if let Some(rule) = line.strip_prefix("- ") {
                rules.push(rule.trim().to_string());
            } else if MORE.is_match(line)
                && let Some(last) = rules.last_mut()
            {
                last.push(' ');
                last.push_str(line.trim());
            }
            continue;
        }
        if stop || section.is_empty() {
            continue;
        }
        if let Some(item) = ITEM.captures(line) {
            items.push((item[1].to_string(), section.clone(), item[2].trim().to_string()));
        } else if MORE.is_match(line)
            && let Some(last) = items.last_mut()
        {
            last.2.push(' ');
            last.2.push_str(line.trim());
        }
    }
    for (number, section, text) in items {
        let (job, rest) = match text.find(": ") {
            Some(at) if at > 0 => (text[..at].trim().to_string(), text[at + 2..].to_string()),
            _ => (text.clone(), String::new()),
        };
        let mut parts = sentences(&rest).into_iter();
        let pick = parts.next().unwrap_or_default();
        let mut fallback: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut place = String::new();
        for part in parts {
            if let Some(found) = BACKUP_PART.captures(&part) {
                fallback.push(found[1].trim().to_string());
            } else if place.is_empty() && PLACE_PART.is_match(&part) {
                place = part;
            } else {
                notes.push(part);
            }
        }
        let parsed = parse_pick(&pick).unwrap_or_default();
        map.rows.push(AgentMapRow {
            id: format!("m{number}"),
            section,
            job,
            member: parsed.member,
            backup: parsed.backup,
            agent: parsed.agent,
            model: parsed.model,
            effort: parsed.effort,
            pick: clean_line(&pick, 400),
            fallback: clean_line(&fallback.join("; "), 400),
            place: clean_line(&place, 80),
            notes: clean_line(&notes.join(". "), 800),
            off: false,
        });
    }
    map.criteria.extend(rules.iter().map(|r| clean_line(r, 600)));
    map
}

pub fn seed_from_md(conn: &Connection, source: &str, at: &str) -> Result<Option<AgentMap>> {
    if !agent_map::map_rows(conn)?.is_empty() {
        return Ok(None);
    }
    let map = import_map_md(source);
    if map.rows.is_empty() {
        return Ok(None);
    }
    let text = serde_json::to_string(&AgentMap { at: at.to_string(), ..map })?;
    Ok(Some(agent_map::import_agent_map(conn, &text, at)?))
}

fn stems(value: &str) -> HashSet<String> {
    let lower = value.to_lowercase();
    STEM.find_iter(&lower).map(|m| m.as_str()).filter(|w| !STOP.contains(w)).map(|w| w.chars().take(5).collect()).collect()
}

pub fn rows_for_task(map: &AgentMap, task: &str, limit: usize) -> Vec<RowMatch> {
    let wanted = stems(task);
    let mut found: Vec<RowMatch> = map
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| !row.off)
        .map(|(index, row)| RowMatch {
            row: row.clone(),
            number: index + 1,
            score: stems(&format!("{} {}", row.section, row.job)).iter().filter(|s| wanted.contains(*s)).count(),
        })
        .collect();
    found.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.number.cmp(&b.number)));
    found.truncate(limit);
    found
}
