use crate::db::{flag, list_text, text_list};
use crate::text::{clean_date, clean_priority, clean_text, hide_private_files, FIELD_MAX};
use crate::time::{now_ms, DAY_MS};
use anyhow::{bail, Result};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const OLD_MS: i64 = 7 * DAY_MS;
pub const TITLE_MAX: usize = 300;
pub const TEXT_MAX: usize = 20_000;
pub const BASE_MAX: usize = 200_000;
pub const UNCOMMITTED_MAX: usize = 500;
pub const MAX_DOING: i64 = 8;
pub const LIST_LIMIT: usize = 2000;
pub const LIST_LIMIT_MAX: usize = 5000;
pub const STARTING_PREFIX: &str = "starting-";
pub const RUN_PREFIX: &str = "run-";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Inbox,
    Todo,
    Doing,
    Blocked,
    Review,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 7] = [
        TaskStatus::Inbox,
        TaskStatus::Todo,
        TaskStatus::Doing,
        TaskStatus::Blocked,
        TaskStatus::Review,
        TaskStatus::Done,
        TaskStatus::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Inbox => "inbox",
            TaskStatus::Todo => "todo",
            TaskStatus::Doing => "doing",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Review => "review",
            TaskStatus::Done => "done",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<TaskStatus> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }

    pub fn is_open(self) -> bool {
        !self.is_closed()
    }

    pub fn is_closed(self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Cancelled)
    }

    pub fn is_held(self) -> bool {
        matches!(self, TaskStatus::Doing | TaskStatus::Blocked | TaskStatus::Review)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    Manual,
    Nixa,
    Agent,
}

const SOURCES: usize = 3;

impl TaskSource {
    pub const ALL: [TaskSource; SOURCES] = [
        TaskSource::Manual,
        TaskSource::Nixa,
        TaskSource::Agent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskSource::Manual => "manual",
            TaskSource::Nixa => "nixa",
            TaskSource::Agent => "agent",
        }
    }

    pub fn parse(text: &str) -> Option<TaskSource> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }

    pub fn group(self) -> TaskGroup {
        TaskGroup::Project
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskGroup {
    Project,
    NixaGap,
}

const GROUPS: usize = 2;

impl TaskGroup {
    pub const ALL: [TaskGroup; GROUPS] = [
        TaskGroup::Project,
        TaskGroup::NixaGap,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskGroup::Project => "project",
            TaskGroup::NixaGap => "nixa-gap",
        }
    }

    pub fn parse(text: &str) -> Option<TaskGroup> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentKind {
    Note,
    Progress,
    Question,
    Answer,
    Result,
    Handoff,
    Error,
    Claim,
}

const COMMENT_KINDS: usize = 8;

impl CommentKind {
    pub const ALL: [CommentKind; COMMENT_KINDS] = [
        CommentKind::Note,
        CommentKind::Progress,
        CommentKind::Question,
        CommentKind::Answer,
        CommentKind::Result,
        CommentKind::Handoff,
        CommentKind::Error,
        CommentKind::Claim,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CommentKind::Note => "note",
            CommentKind::Progress => "progress",
            CommentKind::Question => "question",
            CommentKind::Answer => "answer",
            CommentKind::Result => "result",
            CommentKind::Handoff => "handoff",
            CommentKind::Error => "error",
            CommentKind::Claim => "claim",
        }
    }

    pub fn parse(text: &str) -> Option<CommentKind> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    User,
    Nixa,
    Agent,
    System,
}

const AUTHOR_KINDS: usize = 4;

impl AuthorKind {
    pub const ALL: [AuthorKind; AUTHOR_KINDS] = [
        AuthorKind::User,
        AuthorKind::Nixa,
        AuthorKind::Agent,
        AuthorKind::System,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AuthorKind::User => "user",
            AuthorKind::Nixa => "nixa",
            AuthorKind::Agent => "agent",
            AuthorKind::System => "system",
        }
    }

    pub fn parse(text: &str) -> Option<AuthorKind> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedOn {
    User,
    Decision,
    Task,
    Other,
}

const BLOCKED_ONS: usize = 4;

impl BlockedOn {
    pub const ALL: [BlockedOn; BLOCKED_ONS] = [
        BlockedOn::User,
        BlockedOn::Decision,
        BlockedOn::Task,
        BlockedOn::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BlockedOn::User => "user",
            BlockedOn::Decision => "decision",
            BlockedOn::Task => "task",
            BlockedOn::Other => "other",
        }
    }

    pub fn parse(text: &str) -> Option<BlockedOn> {
        Self::ALL.into_iter().find(|s| s.as_str() == text.trim())
    }

    pub fn is_question(self) -> bool {
        matches!(self, Self::User | Self::Decision)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: AuthorKind,
    pub name: String,
    #[serde(default)]
    pub terminal: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub space_id: String,
}

impl Actor {
    pub fn new(kind: AuthorKind, name: &str) -> Actor {
        Actor { kind, name: name.to_string(), terminal: String::new(), model: String::new(), space_id: String::new() }
    }

    pub fn in_terminal(kind: AuthorKind, name: &str, terminal: &str) -> Actor {
        Actor { terminal: terminal.to_string(), ..Actor::new(kind, name) }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLink {
    pub source: String,
    pub id: String,
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub space_id: String,
    pub space_name: String,
    pub assigned_kind: String,
    pub assigned: String,
    pub claimed_by: String,
    pub terminal_id: String,
    pub claimed_at: i64,
    pub active_at: i64,
    pub needs_user: bool,
    pub blocked_on: Option<BlockedOn>,
    pub source: TaskSource,
    pub group: TaskGroup,
    pub created_by: String,
    pub created_terminal: String,
    pub link_source: String,
    pub link_id: String,
    pub link_number: String,
    pub link_url: String,
    pub link_status: String,
    pub link_seen_at: i64,
    pub priority: String,
    pub due_date: String,
    pub repo: String,
    pub branch: String,
    pub commit_sha: String,
    pub pr_url: String,
    pub base: String,
    pub uncommitted: Vec<String>,
    pub then_member: String,
    pub workflow: String,
    pub archived_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentLinks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub commit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pr: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskComment {
    pub id: i64,
    pub task_id: i64,
    pub at: i64,
    pub author_kind: AuthorKind,
    pub author: String,
    pub model: String,
    pub terminal_id: String,
    pub space_id: String,
    pub kind: CommentKind,
    pub text: String,
    pub links: CommentLinks,
    pub extra: serde_json::Map<String, serde_json::Value>,
    pub link_comment_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskInput {
    pub title: String,
    pub description: String,
    pub status: Option<TaskStatus>,
    pub space_id: String,
    pub space_name: String,
    pub assigned_kind: String,
    pub assigned: String,
    pub workflow: String,
    pub source: Option<TaskSource>,
    pub group: Option<TaskGroup>,
    pub priority: String,
    pub due_date: String,
    pub repo: String,
    pub branch: String,
    pub commit_sha: String,
    pub pr_url: String,
    pub link: Option<TaskLink>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub group: Option<TaskGroup>,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    pub assigned_kind: Option<String>,
    pub assigned: Option<String>,
    pub priority: Option<String>,
    pub due_date: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub pr_url: Option<String>,
    pub link_number: Option<String>,
    pub link_url: Option<String>,
    pub link_status: Option<String>,
    pub link_seen_at: Option<i64>,
    pub base: Option<String>,
    pub needs_user: Option<bool>,
    pub then_member: Option<String>,
    pub workflow: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommentInput {
    pub kind: Option<CommentKind>,
    pub text: String,
    pub links: CommentLinks,
    pub extra: serde_json::Map<String, serde_json::Value>,
    pub link_comment_id: String,
    pub at: Option<i64>,
}

impl CommentInput {
    pub fn new(kind: CommentKind, text: &str) -> CommentInput {
        CommentInput { kind: Some(kind), text: text.to_string(), ..CommentInput::default() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Archived {
    #[default]
    Any,
    Hide,
    Only,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFilter {
    pub archived: Archived,
    pub space: Option<String>,
    pub status: Vec<TaskStatus>,
    pub source: Vec<TaskSource>,
    pub group: Vec<TaskGroup>,
    pub assigned: String,
    pub terminal: String,
    pub q: String,
    pub old: bool,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimResult {
    pub ok: bool,
    pub reason: String,
    pub task: Option<Task>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskDetail {
    pub task: Task,
    pub comments: Vec<TaskComment>,
}

pub fn starting_holder(id: i64) -> String {
    format!("{STARTING_PREFIX}{id}")
}

pub fn is_starting(terminal: &str) -> bool {
    terminal.starts_with(STARTING_PREFIX)
}

pub fn run_holder(run: &str) -> String {
    format!("{RUN_PREFIX}{run}")
}

pub fn is_run_holder(terminal: &str) -> bool {
    terminal.starts_with(RUN_PREFIX)
}

fn task_text(text: &str, max: usize) -> String {
    clean_text(&hide_private_files(text), max)
}

fn field(text: &str) -> String {
    clean_text(text, FIELD_MAX)
}

fn assigned_kind(text: &str) -> String {
    match text.trim() {
        "member" => "member".to_string(),
        "agent" => "agent".to_string(),
        _ => String::new(),
    }
}

fn clean_workflow(text: &str) -> Result<String> {
    let name = field(text);
    let ok = name.is_empty()
        || (name.len() <= 60
            && name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    if !ok {
        bail!("Workflow name: small letters, numbers and dashes, up to 60.");
    }
    Ok(name)
}

fn text_map(text: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn clean_links(links: &CommentLinks) -> CommentLinks {
    let list = |items: &[String], max: usize| -> Vec<String> {
        items.iter().map(|i| field(i)).filter(|i| !i.is_empty()).take(max).collect()
    };
    CommentLinks {
        files: list(&links.files, 50),
        urls: list(&links.urls, 20),
        branch: field(&links.branch),
        commit: field(&links.commit),
        pr: field(&links.pr),
    }
}

fn to_task(row: &Row) -> rusqlite::Result<Task> {
    let source = TaskSource::parse(&row.get::<_, String>("source")?).unwrap_or(TaskSource::Manual);
    let group = TaskGroup::parse(&row.get::<_, String>("task_group")?).unwrap_or(source.group());
    Ok(Task {
        id: row.get("id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        status: TaskStatus::parse(&row.get::<_, String>("status")?).unwrap_or(TaskStatus::Inbox),
        space_id: row.get("space_id")?,
        space_name: row.get("space_name")?,
        assigned_kind: row.get("assigned_kind")?,
        assigned: row.get("assigned")?,
        claimed_by: row.get("claimed_by")?,
        terminal_id: row.get("terminal_id")?,
        claimed_at: row.get("claimed_at")?,
        active_at: row.get("active_at")?,
        needs_user: row.get::<_, i64>("needs_user")? == 1,
        blocked_on: BlockedOn::parse(&row.get::<_, String>("blocked_on")?),
        source,
        group,
        created_by: row.get("created_by")?,
        created_terminal: row.get("created_terminal")?,
        link_source: row.get("link_source")?,
        link_id: row.get("link_id")?,
        link_number: row.get("link_number")?,
        link_url: row.get("link_url")?,
        link_status: row.get("link_status")?,
        link_seen_at: row.get("link_seen_at")?,
        priority: clean_priority(&row.get::<_, String>("priority")?),
        due_date: row.get("due_date")?,
        repo: row.get("repo")?,
        branch: row.get("branch")?,
        commit_sha: row.get("commit_sha")?,
        pr_url: row.get("pr_url")?,
        base: row.get("base")?,
        uncommitted: text_list(&row.get::<_, String>("uncommitted")?),
        then_member: row.get("then_member")?,
        workflow: row.get("workflow")?,
        archived_at: row.get("archived_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        closed_at: row.get("closed_at")?,
    })
}

fn to_comment(row: &Row) -> rusqlite::Result<TaskComment> {
    let links: String = row.get("links")?;
    let extra: String = row.get("extra")?;
    Ok(TaskComment {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        at: row.get("at")?,
        author_kind: AuthorKind::parse(&row.get::<_, String>("author_kind")?).unwrap_or(AuthorKind::System),
        author: row.get("author")?,
        model: row.get("model")?,
        terminal_id: row.get("terminal_id")?,
        space_id: row.get("space_id")?,
        kind: CommentKind::parse(&row.get::<_, String>("kind")?).unwrap_or(CommentKind::Note),
        text: row.get("text")?,
        links: serde_json::from_str(&links).unwrap_or_default(),
        extra: text_map(&extra),
        link_comment_id: row.get("link_comment_id")?,
    })
}

fn tasks_where(conn: &Connection, sql: &str, values: Vec<Value>) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params_from_iter(values), to_task)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_task(conn: &Connection, id: i64) -> Result<Option<Task>> {
    let task = conn.query_row("SELECT * FROM tasks WHERE id = ?1", params![id], to_task).optional()?;
    Ok(task)
}

pub fn must_task(conn: &Connection, id: i64) -> Result<Task> {
    match get_task(conn, id)? {
        Some(task) => Ok(task),
        None => bail!("No task #{id}."),
    }
}

pub fn add_task(conn: &Connection, input: &TaskInput, by: &Actor) -> Result<Task> {
    let title = task_text(&input.title, TITLE_MAX);
    if title.is_empty() {
        bail!("A task needs a title.");
    }
    let source = input.source.unwrap_or(match by.kind {
        AuthorKind::User => TaskSource::Manual,
        AuthorKind::Agent => TaskSource::Agent,
        _ => TaskSource::Nixa,
    });
    let group = input.group.unwrap_or(source.group());
    let status = input.status.unwrap_or(if by.kind == AuthorKind::User { TaskStatus::Todo } else { TaskStatus::Inbox });
    if status == TaskStatus::Doing {
        bail!("A new task cannot be in doing, because no agent holds it yet. Add it as todo, then start it or claim it.");
    }
    let workflow = clean_workflow(&input.workflow)?;
    let at = now_ms();
    let text = |v: &str| Value::Text(field(v));
    #[allow(unused_mut)]
    let mut columns: Vec<(&str, Value)> = vec![
        ("title", Value::Text(title)),
        ("description", Value::Text(task_text(&input.description, TEXT_MAX))),
        ("status", Value::Text(status.as_str().to_string())),
        ("space_id", text(&input.space_id)),
        ("space_name", text(&input.space_name)),
        ("assigned_kind", Value::Text(assigned_kind(&input.assigned_kind))),
        ("assigned", text(&input.assigned)),
        ("source", Value::Text(source.as_str().to_string())),
        ("task_group", Value::Text(group.as_str().to_string())),
        ("created_by", text(&by.name)),
        ("created_terminal", text(&by.terminal)),
        ("priority", Value::Text(clean_priority(&input.priority))),
        ("due_date", Value::Text(clean_date(&input.due_date))),
        ("repo", text(&input.repo)),
        ("branch", text(&input.branch)),
        ("commit_sha", text(&input.commit_sha)),
        ("pr_url", text(&input.pr_url)),
        ("workflow", Value::Text(workflow)),
        ("created_at", Value::Integer(at)),
        ("updated_at", Value::Integer(at)),
        ("closed_at", Value::Integer(if status.is_closed() { at } else { 0 })),
    ];
    let link = input.link.clone();
    if let Some(link) = &link {
        columns.extend([
            ("link_source", text(&link.source)),
            ("link_id", text(&link.id)),
            ("link_number", text(&link.number)),
            ("link_url", text(&link.url)),
            ("link_status", text(&link.status)),
            ("link_seen_at", Value::Integer(at)),
        ]);
    }
    let names: Vec<&str> = columns.iter().map(|(name, _)| *name).collect();
    let marks: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
    conn.execute(
        &format!("INSERT INTO tasks ({}) VALUES ({})", names.join(", "), marks.join(", ")),
        params_from_iter(columns.into_iter().map(|(_, value)| value)),
    )?;
    let id = conn.last_insert_rowid();
    must_task(conn, id)
}

pub fn update_task(conn: &Connection, id: i64, patch: &TaskPatch) -> Result<Task> {
    must_task(conn, id)?;
    let mut sets: Vec<&str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let mut put = |column: &'static str, value: Value| {
        sets.push(column);
        values.push(value);
    };
    let text = |v: &str| Value::Text(field(v));
    if let Some(v) = &patch.title {
        let title = task_text(v, TITLE_MAX);
        if title.is_empty() {
            bail!("A task needs a title.");
        }
        put("title", Value::Text(title));
    }
    if let Some(v) = &patch.description {
        put("description", Value::Text(task_text(v, TEXT_MAX)));
    }
    if let Some(v) = patch.group {
        put("task_group", Value::Text(v.as_str().to_string()));
    }
    let (link_number, link_url, link_status, link_seen_at) = (&patch.link_number, &patch.link_url, &patch.link_status, &patch.link_seen_at);
    let plain = [
        ("space_id", &patch.space_id),
        ("space_name", &patch.space_name),
        ("assigned", &patch.assigned),
        ("repo", &patch.repo),
        ("branch", &patch.branch),
        ("commit_sha", &patch.commit_sha),
        ("pr_url", &patch.pr_url),
        ("link_number", link_number),
        ("link_url", link_url),
        ("link_status", link_status),
        ("then_member", &patch.then_member),
    ];
    for (column, value) in plain {
        if let Some(v) = value {
            put(column, text(v));
        }
    }
    if let Some(v) = &patch.assigned_kind {
        put("assigned_kind", Value::Text(assigned_kind(v)));
    }
    if let Some(v) = &patch.priority {
        put("priority", Value::Text(clean_priority(v)));
    }
    if let Some(v) = &patch.due_date {
        put("due_date", Value::Text(clean_date(v)));
    }
    if let Some(v) = link_seen_at {
        put("link_seen_at", Value::Integer(*v));
    }
    if let Some(v) = &patch.base {
        put("base", Value::Text(crate::text::clip(v, TEXT_MAX)));
    }
    if let Some(v) = patch.needs_user {
        put("needs_user", Value::Integer(flag(v)));
    }
    if let Some(v) = &patch.workflow {
        put("workflow", Value::Text(clean_workflow(v)?));
    }
    let extra = false;
    if sets.is_empty() && !extra {
        return must_task(conn, id);
    }
    let assigns: Vec<String> = sets.iter().map(|c| format!("{c} = ?")).chain(std::iter::once("updated_at = ?".to_string())).collect();
    values.push(Value::Integer(now_ms()));
    values.push(Value::Integer(id));
    conn.execute(
        &format!("UPDATE tasks SET {} WHERE id = ?", assigns.join(", ")),
        params_from_iter(values),
    )?;
    must_task(conn, id)
}

pub fn set_status(conn: &Connection, id: i64, status: TaskStatus, blocked_on: Option<BlockedOn>, needs_user: bool) -> Result<Task> {
    let before = must_task(conn, id)?;
    if status == TaskStatus::Doing && before.terminal_id.is_empty() {
        bail!("Task #{id} has no claim, so it cannot be in doing. Start it, or claim it for the terminal that works on it.");
    }
    let at = now_ms();
    let closed = status.is_closed();
    let free = matches!(status, TaskStatus::Inbox | TaskStatus::Todo);
    let blocked = if status == TaskStatus::Blocked { Some(blocked_on.unwrap_or(BlockedOn::Other)) } else { None };
    let needs = !closed && !free && (needs_user || blocked == Some(BlockedOn::User));
    let release = if free { ", claimed_by = '', terminal_id = '', claimed_at = 0, base = ''" } else { "" };
    conn.execute(
        &format!("UPDATE tasks SET status = ?1, blocked_on = ?2, needs_user = ?3, updated_at = ?4, closed_at = ?5{release} WHERE id = ?6"),
        params![
            status.as_str(),
            blocked.map(|b| b.as_str()).unwrap_or(""),
            flag(needs),
            at,
            if closed { at } else { 0 },
            id
        ],
    )?;
    must_task(conn, id)
}

pub fn claim_task(conn: &Connection, id: i64, by: &Actor, base: &str) -> Result<ClaimResult> {
    let terminal = field(&by.terminal);
    if terminal.is_empty() {
        return Ok(ClaimResult { ok: false, reason: "A claim needs a terminal id.".to_string(), task: None });
    }
    let at = now_ms();
    let name = if field(&by.name).is_empty() { terminal.clone() } else { field(&by.name) };
    let base = if base.len() > BASE_MAX { "" } else { base };
    let changed = conn.execute(
        "UPDATE tasks SET status = 'doing', claimed_by = ?1, terminal_id = ?2, claimed_at = ?3, active_at = ?3, needs_user = 0, blocked_on = '', base = ?4, updated_at = ?3
         WHERE id = ?5 AND status = 'todo' AND terminal_id = ''",
        params![name, terminal, at, base, id],
    )?;
    let Some(task) = get_task(conn, id)? else {
        return Ok(ClaimResult { ok: false, reason: format!("No task #{id}."), task: None });
    };
    if changed == 1 {
        return Ok(ClaimResult { ok: true, reason: String::new(), task: Some(task) });
    }
    let reason = if is_starting(&task.terminal_id) {
        format!("Nixa is starting an agent for task #{id} right now.")
    } else if !task.terminal_id.is_empty() {
        let who = if task.claimed_by.is_empty() { &task.terminal_id } else { &task.claimed_by };
        format!("Task #{id} is claimed by {who} in terminal {}.", task.terminal_id)
    } else if task.status == TaskStatus::Inbox {
        format!("Task #{id} is still in the inbox. Nixa or the user moves it to todo first.")
    } else {
        format!("Task #{id} is {}. Only a todo task can be claimed.", task.status.as_str())
    };
    Ok(ClaimResult { ok: false, reason, task: Some(task) })
}

pub fn release_task(conn: &Connection, id: i64) -> Result<Task> {
    set_status(conn, id, TaskStatus::Todo, None, false)
}

pub fn assign_task(conn: &Connection, id: i64, kind: &str, who: &str) -> Result<Task> {
    let patch = TaskPatch { assigned_kind: Some(kind.to_string()), assigned: Some(who.to_string()), ..TaskPatch::default() };
    update_task(conn, id, &patch)
}

pub fn done_task(conn: &Connection, id: i64, note: &str, by: &Actor) -> Result<Task> {
    let task = set_status(conn, id, TaskStatus::Done, None, false)?;
    if note.trim().is_empty() {
        return Ok(task);
    }
    add_comment(conn, id, &CommentInput::new(CommentKind::Result, note), by)?;
    must_task(conn, id)
}

pub fn move_claim(conn: &Connection, id: i64, from: &str, to_terminal: &str, to_name: &str) -> Result<Option<Task>> {
    let terminal = field(to_terminal);
    if terminal.is_empty() || from.is_empty() {
        return Ok(None);
    }
    let name = if field(to_name).is_empty() { terminal.clone() } else { field(to_name) };
    let at = now_ms();
    let changed = conn.execute(
        "UPDATE tasks SET terminal_id = ?1, claimed_by = ?2, active_at = ?3, updated_at = ?3 WHERE id = ?4 AND terminal_id = ?5 AND status = 'doing'",
        params![terminal, name, at, id, from],
    )?;
    if changed == 1 { get_task(conn, id) } else { Ok(None) }
}

pub fn drop_claim(conn: &Connection, id: i64, terminal: &str) -> Result<Option<Task>> {
    if terminal.is_empty() {
        return Ok(None);
    }
    let changed = conn.execute(
        "UPDATE tasks SET status = 'todo', blocked_on = '', needs_user = 0, closed_at = 0, claimed_by = '', terminal_id = '', claimed_at = 0, base = '', updated_at = ?1
         WHERE id = ?2 AND terminal_id = ?3 AND status = 'doing'",
        params![now_ms(), id, terminal],
    )?;
    if changed == 1 { get_task(conn, id) } else { Ok(None) }
}

pub fn unclaim_task(conn: &Connection, id: i64) -> Result<Task> {
    must_task(conn, id)?;
    conn.execute(
        "UPDATE tasks SET claimed_by = '', terminal_id = '', claimed_at = 0, base = '', updated_at = ?1 WHERE id = ?2",
        params![now_ms(), id],
    )?;
    must_task(conn, id)
}

pub fn touch_terminal(conn: &Connection, terminal: &str) -> Result<usize> {
    if terminal.is_empty() {
        return Ok(0);
    }
    Ok(conn.execute(
        "UPDATE tasks SET active_at = ?1 WHERE terminal_id = ?2 AND status IN ('doing', 'blocked', 'review')",
        params![now_ms(), terminal],
    )?)
}

const COMMENT_SQL: &str = "INSERT OR IGNORE INTO task_comments (task_id, at, author_kind, author, model, terminal_id, space_id, kind, text, links, extra, link_comment_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";

pub fn add_comment(conn: &Connection, task_id: i64, input: &CommentInput, by: &Actor) -> Result<Option<TaskComment>> {
    let task = must_task(conn, task_id)?;
    let text = crate::text::hide_secrets(&task_text(&input.text, TEXT_MAX), &[]);
    if text.is_empty() {
        bail!("A comment needs text.");
    }
    let kind = input.kind.unwrap_or(CommentKind::Note);
    let extra = crate::text::clip(&serde_json::to_string(&input.extra)?, TEXT_MAX);
    let at = input.at.filter(|at| *at > 0).unwrap_or_else(now_ms);
    let space = if field(&by.space_id).is_empty() { task.space_id.clone() } else { field(&by.space_id) };
    let link_comment = &input.link_comment_id;
    let values: Vec<Value> = vec![
        Value::Integer(task_id),
        Value::Integer(at),
        Value::Text(by.kind.as_str().to_string()),
        Value::Text(field(&by.name)),
        Value::Text(field(&by.model)),
        Value::Text(field(&by.terminal)),
        Value::Text(space),
        Value::Text(kind.as_str().to_string()),
        Value::Text(text),
        Value::Text(serde_json::to_string(&clean_links(&input.links))?),
        Value::Text(extra),
        Value::Text(field(link_comment)),
    ];
    let changed = conn.execute(COMMENT_SQL, params_from_iter(values))?;
    if changed == 0 {
        return Ok(None);
    }
    let comment_id = conn.last_insert_rowid();
    conn.execute("UPDATE tasks SET updated_at = ?1 WHERE id = ?2", params![now_ms(), task_id])?;
    Ok(Some(conn.query_row("SELECT * FROM task_comments WHERE id = ?1", params![comment_id], to_comment)?))
}

pub fn task_comments(conn: &Connection, task_id: i64) -> Result<Vec<TaskComment>> {
    let mut stmt = conn.prepare("SELECT * FROM task_comments WHERE task_id = ?1 ORDER BY at, id")?;
    let rows = stmt.query_map(params![task_id], to_comment)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn comment_counts(conn: &Connection, ids: &[i64]) -> Result<BTreeMap<i64, i64>> {
    let mut counts = BTreeMap::new();
    if ids.is_empty() {
        return Ok(counts);
    }
    let marks = vec!["?"; ids.len()].join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT task_id, COUNT(*) FROM task_comments WHERE task_id IN ({marks}) GROUP BY task_id"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (id, count) = row?;
        counts.insert(id, count);
    }
    Ok(counts)
}

pub fn show_task(conn: &Connection, id: i64) -> Result<Option<TaskDetail>> {
    let Some(task) = get_task(conn, id)? else {
        return Ok(None);
    };
    Ok(Some(TaskDetail { comments: task_comments(conn, id)?, task }))
}

pub fn list_tasks(conn: &Connection, filter: &TaskFilter) -> Result<Vec<Task>> {
    let mut wheres: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let mut many = |column: &str, items: Vec<&'static str>, wheres: &mut Vec<String>| {
        if items.is_empty() {
            return;
        }
        wheres.push(format!("{column} IN ({})", vec!["?"; items.len()].join(", ")));
        values.extend(items.into_iter().map(|i| Value::Text(i.to_string())));
    };
    match filter.archived {
        Archived::Hide => wheres.push("archived_at = 0".to_string()),
        Archived::Only => wheres.push("archived_at > 0".to_string()),
        Archived::Any => {}
    }
    many("status", filter.status.iter().map(|s| s.as_str()).collect(), &mut wheres);
    many("source", filter.source.iter().map(|s| s.as_str()).collect(), &mut wheres);
    many("task_group", filter.group.iter().map(|s| s.as_str()).collect(), &mut wheres);
    if let Some(space) = &filter.space {
        wheres.push("space_id = ?".to_string());
        values.push(Value::Text(space.clone()));
    }
    if !filter.assigned.is_empty() {
        wheres.push("assigned = ?".to_string());
        let custom = filter.assigned.strip_prefix("custom:").or_else(|| filter.assigned.strip_prefix("member:"));
        if let Some(id) = custom {
            wheres.push("assigned_kind = 'member'".to_string());
            values.push(Value::Text(id.to_string()));
        } else {
            values.push(Value::Text(filter.assigned.clone()));
        }
    }
    if !filter.terminal.is_empty() {
        wheres.push("terminal_id = ?".to_string());
        values.push(Value::Text(filter.terminal.clone()));
    }
    let q = filter.q.trim();
    if !q.is_empty() {
        let mut like = String::from("%");
        for c in q.chars() {
            if matches!(c, '\\' | '%' | '_') {
                like.push('\\');
            }
            like.push(c);
        }
        like.push('%');
        wheres.push(SEARCH_SQL.to_string());
        for _ in 0..SEARCH_SQL.matches('?').count() {
            values.push(Value::Text(like.clone()));
        }
    }
    if !filter.old {
        wheres.push("NOT (status IN ('done', 'cancelled') AND closed_at > 0 AND closed_at < ?)".to_string());
        values.push(Value::Integer(now_ms() - OLD_MS));
    }
    let limit = filter.limit.filter(|l| *l > 0).map(|l| l.min(LIST_LIMIT_MAX)).unwrap_or(LIST_LIMIT);
    let where_text = if wheres.is_empty() { String::new() } else { format!(" WHERE {}", wheres.join(" AND ")) };
    tasks_where(conn, &format!("SELECT * FROM tasks{where_text} ORDER BY created_at, id LIMIT {limit}"), values)
}

const SEARCH_SQL: &str = "(title LIKE ? ESCAPE '\\' OR description LIKE ? ESCAPE '\\' OR link_number LIKE ? ESCAPE '\\')";

pub fn next_task(conn: &Connection, space_id: &str) -> Result<Option<Task>> {
    let mut stmt = conn.prepare("SELECT * FROM tasks WHERE status = 'todo' AND terminal_id = '' AND space_id = ?1 ORDER BY created_at, id")?;
    let rows = stmt.query_map(params![space_id], to_task)?;
    for row in rows {
        let task = row?;
        if crate::task_waits::open_waits(conn, task.id).is_empty() {
            return Ok(Some(task));
        }
    }
    Ok(None)
}

pub fn doing_count(conn: &Connection, space_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE space_id = ?1 AND status = 'doing'",
        params![space_id],
        |row| row.get(0),
    )?)
}

pub fn task_by_link(conn: &Connection, source: TaskSource, link_source: &str, link_id: &str) -> Result<Option<Task>> {
    if link_id.trim().is_empty() {
        return Ok(None);
    }
    let task = conn
        .query_row(
            "SELECT * FROM tasks WHERE source = ?1 AND link_source = ?2 AND link_id = ?3",
            params![source.as_str(), link_source, link_id],
            to_task,
        )
        .optional()?;
    Ok(task)
}

pub fn held_tasks(conn: &Connection) -> Result<Vec<Task>> {
    tasks_where(conn, "SELECT * FROM tasks WHERE terminal_id <> '' AND status IN ('doing', 'blocked', 'review') ORDER BY id", Vec::new())
}

pub fn waiting_tasks(conn: &Connection) -> Result<Vec<Task>> {
    tasks_where(conn, "SELECT * FROM tasks WHERE needs_user = 1 AND status IN ('blocked', 'review') ORDER BY created_at, id", Vec::new())
}

pub fn set_uncommitted(conn: &Connection, id: i64, files: &[String]) -> Result<Option<Task>> {
    let mut list: Vec<String> = files.iter().map(|f| field(f)).filter(|f| !f.is_empty()).collect();
    list.sort();
    list.dedup();
    list.truncate(UNCOMMITTED_MAX);
    let text = if list.is_empty() { String::new() } else { list_text(&list) };
    conn.execute("UPDATE tasks SET uncommitted = ?1, updated_at = ?2 WHERE id = ?3", params![text, now_ms(), id])?;
    get_task(conn, id)
}

pub fn uncommitted_tasks(conn: &Connection) -> Result<Vec<Task>> {
    tasks_where(conn, "SELECT * FROM tasks WHERE status = 'done' AND uncommitted <> '' ORDER BY id", Vec::new())
}

pub fn remove_task(conn: &Connection, id: i64) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;
    if let Some(task) = get_task(&tx, id)?
        && !task.link_id.is_empty()
    {
        tx.execute(
            "INSERT INTO task_dropped (source, link_id, title, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source, link_id) DO UPDATE SET title = excluded.title, at = excluded.at",
            params![task.source.as_str(), task.link_id, task.title, now_ms()],
        )?;
    }
    tx.execute("DELETE FROM task_comments WHERE task_id = ?1", params![id])?;
    let gone = tx.execute("DELETE FROM tasks WHERE id = ?1", params![id])? == 1;
    tx.commit()?;
    Ok(gone)
}

pub fn was_dropped(conn: &Connection, source: TaskSource, link_id: &str) -> Result<bool> {
    if link_id.is_empty() {
        return Ok(false);
    }
    Ok(conn
        .query_row("SELECT 1 FROM task_dropped WHERE source = ?1 AND link_id = ?2", params![source.as_str(), link_id], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn archive_task(conn: &Connection, id: i64) -> Result<Task> {
    let before = must_task(conn, id)?;
    if before.status.is_open() {
        set_status(conn, id, TaskStatus::Cancelled, None, false)?;
    }
    let at = now_ms();
    conn.execute("UPDATE tasks SET archived_at = ?1, updated_at = ?1 WHERE id = ?2", params![at, id])?;
    must_task(conn, id)
}

pub fn unarchive_task(conn: &Connection, id: i64) -> Result<Task> {
    must_task(conn, id)?;
    conn.execute("UPDATE tasks SET archived_at = 0, updated_at = ?1 WHERE id = ?2", params![now_ms(), id])?;
    must_task(conn, id)
}

pub fn task_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM task_meta WHERE key = ?1", params![key], |row| row.get(0)).optional()?)
}

pub fn set_task_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO task_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn remove_task_meta(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM task_meta WHERE key = ?1", params![key])? == 1)
}

pub fn task_meta_list(conn: &Connection, prefix: &str) -> Result<Vec<(String, String)>> {
    let clean: String = prefix.chars().filter(|c| *c != '%' && *c != '_').collect();
    let mut stmt = conn.prepare("SELECT key, value FROM task_meta WHERE key LIKE ?1 AND value <> '' ORDER BY key")?;
    let rows = stmt.query_map(params![format!("{clean}%")], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn has_task(conn: &Connection, id: i64) -> Result<bool> {
    Ok(get_task(conn, id)?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;

    fn owner() -> Actor {
        Actor::new(AuthorKind::User, "Sam")
    }

    fn agent(terminal: &str) -> Actor {
        Actor::in_terminal(AuthorKind::Agent, "builder", terminal)
    }

    fn todo(conn: &Connection, title: &str, space: &str) -> Task {
        let input = TaskInput { title: title.to_string(), space_id: space.to_string(), ..TaskInput::default() };
        add_task(conn, &input, &owner()).unwrap()
    }

    #[test]
    fn create_defaults_and_list() {
        let conn = temp_db("tasks-create");
        let a = todo(&conn, "  First  ", "s1");
        assert_eq!(a.title, "First");
        assert_eq!(a.status, TaskStatus::Todo);
        assert_eq!(a.source, TaskSource::Manual);
        assert_eq!(a.group, TaskGroup::Project);
        let by_nixa = add_task(&conn, &TaskInput { title: "From Nixa".into(), ..TaskInput::default() }, &Actor::new(AuthorKind::Nixa, "Nixa")).unwrap();
        assert_eq!(by_nixa.status, TaskStatus::Inbox);
        assert_eq!(by_nixa.source, TaskSource::Nixa);
        assert!(add_task(&conn, &TaskInput { title: " ".into(), ..TaskInput::default() }, &owner()).is_err());
        let doing = TaskInput { title: "x".into(), status: Some(TaskStatus::Doing), ..TaskInput::default() };
        assert!(add_task(&conn, &doing, &owner()).is_err());
        let all = list_tasks(&conn, &TaskFilter::default()).unwrap();
        assert_eq!(all.len(), 2);
        let found = list_tasks(&conn, &TaskFilter { q: "nixa".into(), ..TaskFilter::default() }).unwrap();
        assert_eq!(found.len(), 1);
        let in_space = list_tasks(&conn, &TaskFilter { space: Some("s1".into()), ..TaskFilter::default() }).unwrap();
        assert_eq!(in_space[0].id, a.id);
        let inbox = list_tasks(&conn, &TaskFilter { status: vec![TaskStatus::Inbox], ..TaskFilter::default() }).unwrap();
        assert_eq!(inbox[0].id, by_nixa.id);
    }

    #[test]
    fn comments_hide_secrets() {
        let conn = temp_db("tasks-comment-secret");
        let task = todo(&conn, "Build", "s1");
        let saved = add_comment(&conn, task.id, &CommentInput::new(CommentKind::Note, "set MY_API_KEY=abcdefgh123 now"), &owner()).unwrap().unwrap();
        assert!(saved.text.contains("<hidden>"), "{}", saved.text);
        assert!(!saved.text.contains("abcdefgh123"));
    }

    #[test]
    fn claim_flow() {
        let conn = temp_db("tasks-claim");
        let task = todo(&conn, "Build", "s1");
        let no_terminal = claim_task(&conn, task.id, &owner(), "").unwrap();
        assert!(!no_terminal.ok);
        let first = claim_task(&conn, task.id, &agent("m1"), "abc").unwrap();
        assert!(first.ok);
        let held = first.task.unwrap();
        assert_eq!(held.status, TaskStatus::Doing);
        assert_eq!(held.terminal_id, "m1");
        assert_eq!(held.base, "abc");
        let second = claim_task(&conn, task.id, &agent("m2"), "").unwrap();
        assert!(!second.ok);
        assert!(second.reason.contains("claimed by builder in terminal m1"));
        assert_eq!(doing_count(&conn, "s1").unwrap(), 1);
        assert_eq!(held_tasks(&conn).unwrap().len(), 1);
        assert_eq!(touch_terminal(&conn, "m1").unwrap(), 1);
        let moved = move_claim(&conn, task.id, "m1", "m3", "helper").unwrap().unwrap();
        assert_eq!(moved.terminal_id, "m3");
        assert!(drop_claim(&conn, task.id, "m1").unwrap().is_none());
        let dropped = drop_claim(&conn, task.id, "m3").unwrap().unwrap();
        assert_eq!(dropped.status, TaskStatus::Todo);
        assert!(dropped.terminal_id.is_empty());
        assert_eq!(next_task(&conn, "s1").unwrap().unwrap().id, task.id);
    }

    #[test]
    fn inbox_cannot_be_claimed_and_status_rules() {
        let conn = temp_db("tasks-status");
        let task = add_task(&conn, &TaskInput { title: "Idea".into(), ..TaskInput::default() }, &Actor::new(AuthorKind::Nixa, "Nixa")).unwrap();
        let claim = claim_task(&conn, task.id, &agent("m1"), "").unwrap();
        assert!(claim.reason.contains("inbox"));
        assert!(set_status(&conn, task.id, TaskStatus::Doing, None, false).is_err());
        let blocked = set_status(&conn, task.id, TaskStatus::Blocked, Some(BlockedOn::User), false).unwrap();
        assert!(blocked.needs_user);
        assert_eq!(blocked.blocked_on, Some(BlockedOn::User));
        assert_eq!(waiting_tasks(&conn).unwrap().len(), 1);
        let other = set_status(&conn, task.id, TaskStatus::Blocked, None, false).unwrap();
        assert_eq!(other.blocked_on, Some(BlockedOn::Other));
        let done = done_task(&conn, task.id, "All good", &agent("m1")).unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert!(done.closed_at > 0);
        assert!(!done.needs_user);
        let comments = task_comments(&conn, task.id).unwrap();
        assert_eq!(comments[0].kind, CommentKind::Result);
        conn.execute("UPDATE tasks SET closed_at = 1 WHERE id = ?1", params![task.id]).unwrap();
        assert!(list_tasks(&conn, &TaskFilter::default()).unwrap().is_empty());
        assert_eq!(list_tasks(&conn, &TaskFilter { old: true, ..TaskFilter::default() }).unwrap().len(), 1);
    }

    #[test]
    fn update_assign_comment_show() {
        let conn = temp_db("tasks-update");
        let task = todo(&conn, "Fix", "s1");
        let patch = TaskPatch { priority: Some("★p2".into()), due_date: Some("2026-12-01".into()), workflow: Some("build-review".into()), ..TaskPatch::default() };
        let updated = update_task(&conn, task.id, &patch).unwrap();
        assert_eq!(updated.priority, "P2");
        assert_eq!(updated.due_date, "2026-12-01");
        assert_eq!(updated.workflow, "build-review");
        assert!(update_task(&conn, task.id, &TaskPatch { workflow: Some("Bad Name".into()), ..TaskPatch::default() }).is_err());
        assert!(update_task(&conn, task.id, &TaskPatch { title: Some("".into()), ..TaskPatch::default() }).is_err());
        let assigned = assign_task(&conn, task.id, "agent", "custom:a1").unwrap();
        assert_eq!(assigned.assigned_kind, "agent");
        assert_eq!(assigned.assigned, "custom:a1");
        let mut input = CommentInput::new(CommentKind::Progress, "half way, see /x/.config/nixa/private/k/f.txt");
        input.links.files = vec!["src/a.rs".into(), " ".into()];
        let comment = add_comment(&conn, task.id, &input, &agent("m1")).unwrap().unwrap();
        assert!(comment.text.contains("<private file>"));
        assert_eq!(comment.links.files, vec!["src/a.rs".to_string()]);
        assert_eq!(comment.space_id, "s1");
        assert!(add_comment(&conn, task.id, &CommentInput::new(CommentKind::Note, " "), &owner()).is_err());
        let detail = show_task(&conn, task.id).unwrap().unwrap();
        assert_eq!(detail.comments.len(), 1);
        assert_eq!(comment_counts(&conn, &[task.id, 999]).unwrap().get(&task.id), Some(&1));
        assert!(show_task(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn archive_remove_meta_uncommitted() {
        let conn = temp_db("tasks-archive");
        let task = add_task(&conn, &TaskInput { title: "Plain".into(), ..TaskInput::default() }, &owner()).unwrap();
        assert_eq!(task.group, TaskGroup::Project);
        let archived = archive_task(&conn, task.id).unwrap();
        assert_eq!(archived.status, TaskStatus::Cancelled);
        assert!(archived.archived_at > 0);
        assert_eq!(list_tasks(&conn, &TaskFilter { archived: Archived::Hide, ..TaskFilter::default() }).unwrap().len(), 0);
        assert_eq!(unarchive_task(&conn, task.id).unwrap().archived_at, 0);
        set_task_meta(&conn, "handoff:1", "{}").unwrap();
        set_task_meta(&conn, "other", "x").unwrap();
        assert_eq!(task_meta(&conn, "handoff:1").unwrap().as_deref(), Some("{}"));
        assert_eq!(task_meta_list(&conn, "handoff:").unwrap().len(), 1);
        let files = vec!["b.rs".to_string(), "a.rs".to_string(), "a.rs".to_string()];
        let with = set_uncommitted(&conn, task.id, &files).unwrap().unwrap();
        assert_eq!(with.uncommitted, vec!["a.rs".to_string(), "b.rs".to_string()]);
        assert!(remove_task(&conn, task.id).unwrap());
        assert!(!has_task(&conn, task.id).unwrap());
        assert!(must_task(&conn, task.id).is_err());
    }

    #[test]
    fn outside_links_are_kept_found_and_dropped() {
        let conn = temp_db("tasks-link");
        let link = TaskLink { source: "tracker".into(), id: "i-7".into(), number: "T-7".into(), url: "https://example.com/t/7".into(), status: "open".into() };
        let input = TaskInput { title: "From a tracker".into(), link: Some(link), ..TaskInput::default() };
        let task = add_task(&conn, &input, &owner()).unwrap();
        assert_eq!((task.link_source.as_str(), task.link_id.as_str(), task.link_number.as_str()), ("tracker", "i-7", "T-7"));
        assert_eq!((task.link_url.as_str(), task.link_status.as_str()), ("https://example.com/t/7", "open"));
        assert!(task.link_seen_at > 0);
        assert_eq!(task_by_link(&conn, TaskSource::Manual, "tracker", "i-7").unwrap().unwrap().id, task.id);
        assert!(task_by_link(&conn, TaskSource::Manual, "tracker", "").unwrap().is_none());
        assert!(add_task(&conn, &input, &owner()).is_err());
        let found = list_tasks(&conn, &TaskFilter { q: "T-7".into(), ..TaskFilter::default() }).unwrap();
        assert_eq!(found.len(), 1);
        let before = task.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let patch = TaskPatch { link_status: Some("closed".into()), link_seen_at: Some(5), ..TaskPatch::default() };
        let changed = update_task(&conn, task.id, &patch).unwrap();
        assert_eq!((changed.link_status.as_str(), changed.link_seen_at), ("closed", 5));
        assert!(changed.updated_at > before);
        let mut comment = CommentInput::new(CommentKind::Note, "from the tracker");
        comment.link_comment_id = "c1".into();
        let saved = add_comment(&conn, task.id, &comment, &owner()).unwrap().unwrap();
        assert_eq!(saved.link_comment_id, "c1");
        assert!(add_comment(&conn, task.id, &comment, &owner()).unwrap().is_none());
        let json = serde_json::to_value(&changed).unwrap();
        assert!(json.get("link_id").is_some());
        assert!(!was_dropped(&conn, TaskSource::Manual, "i-7").unwrap());
        assert!(remove_task(&conn, task.id).unwrap());
        assert!(was_dropped(&conn, TaskSource::Manual, "i-7").unwrap());
        assert!(!was_dropped(&conn, TaskSource::Manual, "").unwrap());
        let plain = todo(&conn, "No link", "s1");
        assert!(remove_task(&conn, plain.id).unwrap());
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM task_dropped", [], |row| row.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn public_task_json_has_no_private_keys() {
        let conn = temp_db("tasks-json");
        let task = todo(&conn, "Plain", "s1");
        let text = serde_json::to_string(&show_task(&conn, task.id).unwrap().unwrap()).unwrap();
        assert!(!text.contains("boss"), "{text}");
    }

}
