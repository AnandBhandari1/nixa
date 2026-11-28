use crate::soul::{Soul, hash};
use crate::util::{day_of, flat, now_ms};
use nixa_store::memory::{MemoryLine, forget_memory_line, memory_lines, remember};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;

pub const MAX_DOC: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    Memory,
    Rules,
}

impl Doc {
    pub fn parse(word: &str) -> Option<Doc> {
        match word {
            "memory" => Some(Doc::Memory),
            "rules" => Some(Doc::Rules),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Doc::Memory => "memory",
            Doc::Rules => "rules",
        }
    }

    pub fn stale(self) -> &'static str {
        match self {
            Doc::Memory => {
                "Memory changed elsewhere. Your draft is kept. Reload the latest text before saving."
            }
            Doc::Rules => {
                "Standing rules changed elsewhere. Your draft is kept. Reload the latest text before saving."
            }
        }
    }
}

pub fn memory_text(lines: &[MemoryLine]) -> String {
    lines
        .iter()
        .map(|l| format!("- {}", l.text))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn note_lines(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || matches!(line, "-" | "*" | "+") {
            continue;
        }
        let body = ["- ", "* ", "+ "]
            .iter()
            .find_map(|mark| line.strip_prefix(mark))
            .unwrap_or(line);
        let note = flat(body);
        if note.is_empty() || !seen.insert(note.to_lowercase()) {
            continue;
        }
        out.push(note);
    }
    out
}

fn open(db: &Path) -> Result<nixa_store::Connection, String> {
    nixa_store::open(db).map_err(|e| format!("The memory store could not be opened: {e:#}"))
}

fn lines_of(db: &Path) -> Result<Vec<MemoryLine>, String> {
    memory_lines(&open(db)?).map_err(|e| format!("{e:#}"))
}

pub fn read(dir: &Path, db: &Path, doc: Doc) -> Result<Value, String> {
    match doc {
        Doc::Memory => {
            let lines = lines_of(db)?;
            let text = memory_text(&lines);
            let held: Vec<&str> = lines
                .iter()
                .filter(|l| l.is_held())
                .map(|l| l.text.as_str())
                .collect();
            Ok(
                json!({"document": "memory", "hash": hash(&text), "text": text, "notes": lines.len(), "held": held}),
            )
        }
        Doc::Rules => {
            let soul = Soul::open(dir);
            let current = soul.read();
            Ok(
                json!({"document": "rules", "text": current.text, "hash": current.hash, "problem": soul.problem(), "file": soul.file.display().to_string()}),
            )
        }
    }
}

pub fn current_hash(dir: &Path, db: &Path, doc: Doc) -> Result<String, String> {
    read(dir, db, doc)?
        .get("hash")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "No hash.".to_string())
}

#[derive(Debug)]
pub struct Saved {
    pub data: Value,
    pub notes: Vec<String>,
}

pub fn save(dir: &Path, db: &Path, doc: Doc, text: &str, expected: &str) -> Result<Saved, String> {
    if expected.trim().is_empty() {
        return Err("Read the document before saving its complete text.".into());
    }
    if text.len() > MAX_DOC {
        return Err("This document is too long. Nothing was saved.".into());
    }
    if current_hash(dir, db, doc)? != expected {
        return Err(doc.stale().into());
    }
    let notes = match doc {
        Doc::Rules => {
            Soul::open(dir).update(text, expected)?;
            Vec::new()
        }
        Doc::Memory => save_memory(db, text)?,
    };
    Ok(Saved {
        data: read(dir, db, doc)?,
        notes,
    })
}

fn save_memory(db: &Path, text: &str) -> Result<Vec<String>, String> {
    let wanted = note_lines(text);
    let conn = open(db)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let have = memory_lines(&tx).map_err(|e| format!("{e:#}"))?;
    let keep: HashSet<String> = wanted.iter().map(|w| w.to_lowercase()).collect();
    let known: HashSet<String> = have.iter().map(|l| l.text.to_lowercase()).collect();
    let mut notes = Vec::new();
    let mut removed = 0;
    for line in have
        .iter()
        .filter(|l| !keep.contains(&l.text.to_lowercase()))
    {
        if forget_memory_line(&tx, line.id).map_err(|e| format!("{e:#}"))? {
            removed += 1;
        }
    }
    let day = day_of(now_ms());
    let mut added = 0;
    for note in wanted.iter().filter(|w| !known.contains(&w.to_lowercase())) {
        match remember(&tx, note, &day) {
            Ok(done) if done.saved => {
                added += 1;
                if done.line.is_held() {
                    notes.push(done.message);
                }
            }
            Ok(done) => notes.push(done.message),
            Err(e) => notes.push(format!("Not saved: {note}: {e:#}")),
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    notes.insert(0, format!("{added} added, {removed} removed."));
    Ok(notes)
}
