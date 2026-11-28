use super::chunk::{Part, chunk_markdown, fts_query};
use crate::chat::{ChatItem, Who};
use nixa_store::Connection;
use nixa_store::memory::MemoryLine;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const FILE: &str = "knowledge.db";
pub const MAX_FILE_BYTES: u64 = 400_000;
pub const MAX_FILES: usize = 5000;
pub const MAX_DEPTH: usize = 12;
pub const MEMORY_PATH: &str = "memory";
pub const CHAT_LAST: &str = "chat_last";
const SKIP_DIRS: [&str; 10] = ["node_modules", ".git", ".obsidian", ".trash", "_archive", "graft", ".claude", ".config", "target", "dist"];
const TOKENIZER: &str = "porter unicode61";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Memory,
    Chat,
    Docs,
    Notes,
}

pub const SOURCES: [&str; 4] = ["memory", "chat", "docs", "notes"];

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Memory => "memory",
            Source::Chat => "chat",
            Source::Docs => "docs",
            Source::Notes => "notes",
        }
    }

    pub fn parse(text: &str) -> Option<Source> {
        match text.trim() {
            "memory" => Some(Source::Memory),
            "chat" => Some(Source::Chat),
            "docs" => Some(Source::Docs),
            "notes" => Some(Source::Notes),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub source: Source,
    pub dir: PathBuf,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Hit {
    pub source: String,
    pub title: String,
    pub path: String,
    pub text: String,
    pub score: f64,
    pub indexed_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub source: Option<Source>,
    pub project: Option<String>,
    pub path_contains: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub files: usize,
    pub changed: usize,
    pub failed: usize,
    pub removed: usize,
    pub complete: bool,
    pub error: Option<String>,
}

pub fn open(file: &Path) -> Result<Connection, String> {
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = file.parent() {
            nixa_protocol::paths::make_private_dirs(dir).map_err(|e| format!("The knowledge index cannot open: {e}"))?;
        }
        std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(file).map_err(|e| format!("The knowledge index cannot open: {e}"))?;
    }
    let conn = Connection::open(file).map_err(|e| format!("The knowledge index cannot open: {e}"))?;
    set_private(file);
    conn.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
        PRAGMA secure_delete = ON;
        CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, source TEXT NOT NULL, stamp TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS chunks (id INTEGER PRIMARY KEY, path TEXT NOT NULL, source TEXT NOT NULL, title TEXT NOT NULL, text TEXT NOT NULL, flagged INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS chunks_path ON chunks(path);
        CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(title, text, tokenize = '{TOKENIZER}');
        CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
    ))
    .map_err(|e| format!("The knowledge index cannot start: {e}"))?;
    Ok(conn)
}

fn set_private(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for path in [file.to_path_buf(), PathBuf::from(format!("{}-wal", file.display())), PathBuf::from(format!("{}-shm", file.display()))] {
        if path.exists() {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn count(conn: &Connection) -> usize {
    conn.query_row("SELECT count(*) FROM chunks", [], |r| r.get::<_, i64>(0)).map(|n| n as usize).unwrap_or(0)
}

pub fn meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).ok()
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", (key, value)).map(|_| ()).map_err(err)
}

fn stamp_of(conn: &Connection, path: &str) -> Option<String> {
    conn.query_row("SELECT stamp FROM files WHERE path = ?1", [path], |r| r.get(0)).ok()
}

fn remove_in(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE path = ?1)", [path]).map_err(err)?;
    conn.execute("DELETE FROM chunks WHERE path = ?1", [path]).map_err(err)?;
    conn.execute("DELETE FROM files WHERE path = ?1", [path]).map_err(err)?;
    Ok(())
}

pub fn remove(conn: &Connection, path: &str) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(err)?;
    remove_in(&tx, path)?;
    tx.commit().map_err(err)
}

pub fn store(conn: &Connection, path: &str, source: Source, parts: &[Part], stamp: &str) -> Result<usize, String> {
    let tx = conn.unchecked_transaction().map_err(err)?;
    remove_in(&tx, path)?;
    let mut flagged = 0;
    for part in parts {
        let held = nixa_store::text::looks_injected(&format!("{}\n{}", part.title, part.text));
        if held {
            flagged += 1;
        }
        tx.execute("INSERT INTO chunks (path, source, title, text, flagged) VALUES (?1, ?2, ?3, ?4, ?5)", (path, source.as_str(), &part.title, &part.text, held as i64)).map_err(err)?;
        let id = tx.last_insert_rowid();
        tx.execute("INSERT INTO chunks_fts (rowid, title, text) VALUES (?1, ?2, ?3)", (id, &part.title, &part.text)).map_err(err)?;
    }
    tx.execute("INSERT INTO files (path, source, stamp) VALUES (?1, ?2, ?3)", (path, source.as_str(), stamp)).map_err(err)?;
    tx.commit().map_err(err)?;
    if flagged > 0 {
        crate::util::log("knowledge", &format!("held back {flagged} part(s) that read like orders from {}", if source == Source::Chat { "chat" } else { path }));
    }
    Ok(flagged)
}

pub fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let day = crate::util::day_of(ms);
    let rest = secs % 86_400;
    format!("{day}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn file_stamp(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let ms = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some(format!("{ms}:{}", meta.len()))
}

pub fn private_roots() -> Vec<PathBuf> {
    let home = crate::project_tools::policy::home();
    let mut roots = crate::project_tools::policy::private_roots(home.as_deref(), &nixa_protocol::paths::data_dir());
    let real: Vec<PathBuf> = roots.iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
    for path in real {
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
    roots
}

pub fn blocked_in(path: &Path, roots: &[PathBuf]) -> bool {
    use crate::project_tools::policy::{protected_in, sensitive_file, sensitive_name};
    if sensitive_file(path) || protected_in(path, roots) {
        return true;
    }
    match std::fs::canonicalize(path) {
        Ok(real) => protected_in(&real, roots) || sensitive_name(&real),
        Err(_) => true,
    }
}

fn linked_note(path: &Path, roots: &[PathBuf]) -> bool {
    if blocked_in(path, roots) {
        crate::util::log("knowledge", "skipped a link to a secret or private file");
        return false;
    }
    std::fs::canonicalize(path).map(|real| real.extension().is_some_and(|e| e == "md") && real.is_file()).unwrap_or(false)
}

pub struct Walk {
    pub files: Vec<PathBuf>,
    pub full: bool,
}

pub fn walk(dir: &Path, limit: usize) -> Result<Walk, String> {
    walk_with(dir, limit, &private_roots())
}

pub fn walk_with(dir: &Path, limit: usize, roots: &[PathBuf]) -> Result<Walk, String> {
    let mut out = Walk { files: Vec::new(), full: false };
    if blocked_in(dir, roots) {
        return Err(format!("{}: this folder holds private settings or secrets", dir.display()));
    }
    walk_in(dir, 0, limit, roots, &mut out).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(out)
}

fn walk_in(dir: &Path, depth: usize, limit: usize, roots: &[PathBuf], out: &mut Walk) -> std::io::Result<()> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.files.len() >= limit {
            out.full = true;
            return Ok(());
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        let note = name.ends_with(".md");
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && depth < MAX_DEPTH {
                walk_in(&path, depth + 1, limit, roots, out)?;
            }
        } else if kind.is_file() && note {
            if !blocked_in(&path, roots) {
                out.files.push(path);
            }
        } else if kind.is_symlink() && note && linked_note(&path, roots) {
            out.files.push(path);
        }
    }
    Ok(())
}

pub fn index_file(conn: &Connection, path: &Path, source: Source, label: &str) -> Result<bool, String> {
    let key = path.display().to_string();
    let Some(stamp) = file_stamp(path) else {
        if stamp_of(conn, &key).is_some() {
            remove(conn, &key)?;
            return Ok(true);
        }
        return Ok(false);
    };
    if stamp_of(conn, &key).as_deref() == Some(stamp.as_str()) {
        return Ok(false);
    }
    let bytes = std::fs::read(path).map_err(err)?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let parts = chunk_markdown(&text, &format!("{label} / {name}"));
    store(conn, &key, source, &parts, &stamp)?;
    Ok(true)
}

pub fn refresh_memory(conn: &Connection, lines: &[MemoryLine]) -> Result<bool, String> {
    let usable: Vec<&MemoryLine> = lines.iter().filter(|l| !l.is_held()).collect();
    if usable.is_empty() {
        if stamp_of(conn, MEMORY_PATH).is_some() {
            remove(conn, MEMORY_PATH)?;
            return Ok(true);
        }
        return Ok(false);
    }
    let joined: String = usable.iter().map(|l| format!("{}\n", l.note())).collect();
    let newest = usable.iter().map(|l| l.at.max(0) as u64).max().unwrap_or(0);
    let stamp = format!("{newest}:{}", crate::util::short_hash(&joined));
    if stamp_of(conn, MEMORY_PATH).as_deref() == Some(stamp.as_str()) {
        return Ok(false);
    }
    let parts: Vec<Part> = usable.iter().map(|l| Part { title: "Memory note".into(), text: l.note() }).collect();
    store(conn, MEMORY_PATH, Source::Memory, &parts, &stamp)?;
    Ok(true)
}

pub fn chat_path(item: &ChatItem) -> String {
    format!("chat:{}", item.id)
}

fn talk_part(item: &ChatItem, name: &str) -> Option<Part> {
    let who = match item.who {
        Who::User => "User",
        Who::Nixa => name,
        _ => return None,
    };
    if item.quiet || item.routine {
        return None;
    }
    let clean = crate::util::flat(&item.text);
    if clean.chars().count() < 3 {
        return None;
    }
    let stamp = iso(item.at);
    let title = format!("Talk {} {} {who}", &stamp[..10], &stamp[11..16]);
    Some(Part { title, text: format!("{who}: {clean}") })
}

pub fn add_chat(conn: &Connection, items: &[ChatItem], name: &str) -> Result<usize, String> {
    let last: u64 = meta(conn, CHAT_LAST).and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut added = 0;
    let mut top = last;
    for item in items.iter().filter(|i| i.id > last) {
        top = top.max(item.id);
        if let Some(part) = talk_part(item, name) {
            store(conn, &chat_path(item), Source::Chat, &[part], &iso(item.at))?;
            added += 1;
        }
    }
    if top > last {
        set_meta(conn, CHAT_LAST, &top.to_string())?;
    }
    Ok(added)
}

pub fn wipe_free(conn: &Connection) {
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

pub fn forget_chat(conn: &Connection, newest_id: u64) -> Result<usize, String> {
    let paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM files WHERE source = 'chat'").map_err(err)?;
        stmt.query_map([], |r| r.get(0)).map_err(err)?.filter_map(Result::ok).collect()
    };
    for path in &paths {
        remove(conn, path)?;
    }
    let last: u64 = meta(conn, CHAT_LAST).and_then(|v| v.parse().ok()).unwrap_or(0);
    set_meta(conn, CHAT_LAST, &last.max(newest_id).to_string())?;
    Ok(paths.len())
}

pub fn index_roots(conn: &Connection, roots: &[Root], missing: &[String]) -> Report {
    index_roots_with(conn, roots, missing, &private_roots())
}

pub fn index_roots_with(conn: &Connection, roots: &[Root], missing: &[String], private: &[PathBuf]) -> Report {
    let mut report = Report { complete: missing.is_empty(), ..Report::default() };
    if !missing.is_empty() {
        report.error = Some(format!("missing folder: {}", missing.join(", ")));
    }
    let mut jobs: Vec<(PathBuf, Source, String)> = Vec::new();
    for root in roots {
        let room = MAX_FILES.saturating_sub(jobs.len());
        match walk_with(&root.dir, room, private) {
            Ok(found) => {
                if found.full || room == 0 {
                    report.complete = false;
                    report.error = Some(format!("more than {MAX_FILES} notes; the rest were not read"));
                }
                jobs.extend(found.files.into_iter().map(|p| (p, root.source, root.label.clone())));
            }
            Err(error) => {
                report.complete = false;
                report.failed += 1;
                report.error = Some(error);
            }
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    for (path, source, label) in &jobs {
        if !seen.insert(path.display().to_string()) {
            continue;
        }
        match index_file(conn, path, *source, label) {
            Ok(true) => report.changed += 1,
            Ok(false) => {}
            Err(error) => {
                report.failed += 1;
                if report.failed <= 3 {
                    crate::util::log("knowledge", &format!("could not index {}: {error}", path.display()));
                }
            }
        }
    }
    report.files = seen.len();
    if report.complete {
        let known: Vec<String> = conn
            .prepare("SELECT path FROM files WHERE source IN ('docs', 'notes')")
            .and_then(|mut s| s.query_map([], |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default();
        for path in known.into_iter().filter(|p| !seen.contains(p)) {
            if remove(conn, &path).is_ok() {
                report.removed += 1;
            }
        }
    }
    if report.failed > 0 && report.error.is_none() {
        report.error = Some(format!("{} file(s) failed", report.failed));
    }
    report
}

fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

const SEARCH_SQL: &str = "SELECT chunks.source, chunks.title, chunks.path, chunks.text, bm25(chunks_fts) AS rank, files.stamp FROM chunks_fts JOIN chunks ON chunks.id = chunks_fts.rowid LEFT JOIN files ON files.path = chunks.path WHERE chunks_fts MATCH ?1 AND chunks.flagged = 0 AND (?2 = '' OR chunks.source = ?2) AND (?3 = '' OR chunks.path = ?3 OR chunks.path LIKE ?4 ESCAPE '\\') AND (?5 = '' OR ('/' || lower(chunks.path) || '/') LIKE ?5 ESCAPE '\\') AND (?6 = '' OR lower(chunks.path) LIKE ?6 ESCAPE '\\') ORDER BY rank LIMIT ?7";

pub fn search(conn: &Connection, query: &str, wanted: &Query) -> Vec<Hit> {
    let words = fts_query(query);
    if words.is_empty() {
        return Vec::new();
    }
    let source = wanted.source.map(|s| s.as_str().to_string()).unwrap_or_default();
    let project = wanted.project.as_deref().map(str::trim).unwrap_or_default();
    let (folder, folder_like, name_like) = if project.starts_with('/') {
        let folder = project.trim_end_matches('/');
        (folder.to_string(), format!("{}/%", like_escape(folder)), String::new())
    } else if project.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        (String::new(), String::new(), format!("%/{}/%", like_escape(&project.to_lowercase())))
    };
    let part = wanted.path_contains.as_deref().map(str::trim).filter(|p| !p.is_empty()).map(|p| format!("%{}%", like_escape(&p.to_lowercase()))).unwrap_or_default();
    let limit = wanted.limit.clamp(1, 50) as i64;
    let rows = conn.prepare(SEARCH_SQL).and_then(|mut stmt| {
        stmt.query_map((words, source, folder, folder_like, name_like, part, limit), |r| {
            let source: String = r.get(0)?;
            let stamp: Option<String> = r.get(5)?;
            Ok(Hit { indexed_at: indexed_at(&source, stamp.as_deref()), source, title: r.get(1)?, path: r.get(2)?, text: r.get(3)?, score: -r.get::<_, f64>(4)? })
        })
        .map(|rows| rows.filter_map(Result::ok).collect::<Vec<Hit>>())
    });
    match rows {
        Ok(hits) => hits,
        Err(error) => {
            crate::util::log("knowledge", &format!("search failed: {error}"));
            Vec::new()
        }
    }
}

fn indexed_at(source: &str, stamp: Option<&str>) -> Option<String> {
    let stamp = stamp?;
    if source == "chat" {
        return Some(stamp.to_string());
    }
    stamp.split(':').next().and_then(|ms| ms.parse::<u64>().ok()).filter(|ms| *ms > 0).map(iso)
}
