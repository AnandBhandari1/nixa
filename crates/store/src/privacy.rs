use crate::settings::{setting_bool, set_setting_bool};
use crate::text::clean_line;
use crate::time::now_ms;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

pub const PRIVACY_ON_KEY: &str = "privacy.on";
const VALUE_MAX: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyKind {
    Owner,
    Alias,
    Word,
}

impl PrivacyKind {
    pub const ALL: [PrivacyKind; 3] = [PrivacyKind::Owner, PrivacyKind::Alias, PrivacyKind::Word];

    pub fn as_str(self) -> &'static str {
        match self {
            PrivacyKind::Owner => "owner",
            PrivacyKind::Alias => "alias",
            PrivacyKind::Word => "word",
        }
    }

    pub fn parse(text: &str) -> Option<PrivacyKind> {
        Self::ALL.into_iter().find(|k| k.as_str() == text.trim())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyEntry {
    pub id: i64,
    pub kind: PrivacyKind,
    pub value: String,
    pub at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyNames {
    pub on: bool,
    pub owner: String,
    pub aliases: Vec<String>,
    pub words: Vec<String>,
}

fn to_entry(row: &Row) -> rusqlite::Result<PrivacyEntry> {
    Ok(PrivacyEntry {
        id: row.get("id")?,
        kind: PrivacyKind::parse(&row.get::<_, String>("kind")?).unwrap_or(PrivacyKind::Word),
        value: row.get("value")?,
        at: row.get("at")?,
    })
}

pub fn privacy_list(conn: &Connection) -> Result<Vec<PrivacyEntry>> {
    let mut stmt = conn.prepare("SELECT * FROM privacy_list ORDER BY kind, id")?;
    let rows = stmt.query_map([], to_entry)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn add_private(conn: &Connection, kind: PrivacyKind, value: &str) -> Result<PrivacyEntry> {
    let value = clean_line(value, VALUE_MAX);
    if value.is_empty() {
        bail!("Give the name or word to keep private.");
    }
    let tx = conn.unchecked_transaction()?;
    if kind == PrivacyKind::Owner {
        tx.execute("DELETE FROM privacy_list WHERE kind = 'owner'", [])?;
    }
    tx.execute(
        "INSERT INTO privacy_list (kind, value, at) VALUES (?1, ?2, ?3) ON CONFLICT(kind, value) DO NOTHING",
        params![kind.as_str(), value, now_ms()],
    )?;
    let entry = tx.query_row(
        "SELECT * FROM privacy_list WHERE kind = ?1 AND value = ?2",
        params![kind.as_str(), value],
        to_entry,
    )?;
    tx.commit()?;
    Ok(entry)
}

pub fn remove_private(conn: &Connection, kind: PrivacyKind, value: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM privacy_list WHERE kind = ?1 AND lower(value) = lower(?2)",
        params![kind.as_str(), value.trim()],
    )? > 0)
}

pub fn privacy_on(conn: &Connection) -> Result<bool> {
    setting_bool(conn, PRIVACY_ON_KEY, true)
}

pub fn set_privacy_on(conn: &Connection, on: bool) -> Result<()> {
    set_setting_bool(conn, PRIVACY_ON_KEY, on)
}

pub fn privacy_names(conn: &Connection) -> Result<PrivacyNames> {
    let mut names = PrivacyNames { on: privacy_on(conn)?, ..PrivacyNames::default() };
    for entry in privacy_list(conn)? {
        match entry.kind {
            PrivacyKind::Owner => names.owner = entry.value,
            PrivacyKind::Alias => names.aliases.push(entry.value),
            PrivacyKind::Word => names.words.push(entry.value),
        }
    }
    Ok(names)
}

pub fn private_words_in(names: &PrivacyNames, text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    names
        .words
        .iter()
        .filter(|w| {
            let wanted = w.to_lowercase();
            lower.match_indices(&wanted).any(|(at, found)| {
                let before = lower[..at].chars().next_back().is_none_or(|c| !is_word(c));
                let after = lower[at + found.len()..].chars().next().is_none_or(|c| !is_word(c));
                before && after
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    #[test]
    fn list_switch_and_match() {
        let conn = temp_db("privacy");
        assert!(privacy_on(&conn).unwrap());
        add_private(&conn, PrivacyKind::Owner, "Owner One").unwrap();
        add_private(&conn, PrivacyKind::Owner, "Owner Two").unwrap();
        add_private(&conn, PrivacyKind::Alias, "otwo").unwrap();
        add_private(&conn, PrivacyKind::Word, "salary").unwrap();
        let again = add_private(&conn, PrivacyKind::Word, "salary").unwrap();
        assert!(add_private(&conn, PrivacyKind::Word, " ").is_err());
        assert_eq!(privacy_list(&conn).unwrap().len(), 3);
        let names = privacy_names(&conn).unwrap();
        assert_eq!(names.owner, "Owner Two");
        assert_eq!(names.aliases, vec!["otwo".to_string()]);
        assert_eq!(private_words_in(&names, "What is his Salary?"), vec!["salary".to_string()]);
        assert!(private_words_in(&names, "salaryman").is_empty());
        set_privacy_on(&conn, false).unwrap();
        assert!(!privacy_names(&conn).unwrap().on);
        assert!(remove_private(&conn, PrivacyKind::Word, "SALARY").unwrap());
        assert!(!remove_private(&conn, PrivacyKind::Word, "salary").unwrap());
        assert_eq!(again.kind, PrivacyKind::Word);
    }
}
