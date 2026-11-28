use serde_json::{Map, Value, json};

pub const STATUSES: [&str; 7] = [
    "inbox",
    "todo",
    "doing",
    "blocked",
    "review",
    "done",
    "cancelled",
];
pub const OPEN_STATUSES: [&str; 5] = ["inbox", "todo", "doing", "blocked", "review"];
pub const CLOSED_STATUSES: [&str; 2] = ["done", "cancelled"];
pub const SOURCES: [&str; 3] = ["manual", "nixa", "agent"];
pub const GROUPS: [&str; 2] = ["project", "nixa-gap"];
pub const COMMENT_KINDS: [&str; 7] = [
    "note", "progress", "question", "answer", "result", "handoff", "error",
];
pub const BLOCKED_ON: [&str; 3] = ["user", "task", "other"];
const CUSTOM_VALUE: &str = "custom:";
const MEMBER_VALUE: &str = "member:";
pub const SAVED_WORK: &str = "Work so far from ";
pub const ACTIVE_RUN: [&str; 3] = ["running", "paused", "waiting"];
pub const MAX_ROWS: usize = 400;
pub const SHOWN_OPTIONS: usize = 60;
pub const NOT_CONNECTED: &str = crate::links::brain::NOT_CONNECTED;
pub const STOPPED: &str = crate::links::brain::STOPPED;

pub fn offline(error: &str) -> bool {
    error == NOT_CONNECTED || error == STOPPED
}

pub fn status_label(value: &str) -> String {
    match value {
        "inbox" => "Inbox",
        "todo" => "To do",
        "doing" => "Doing",
        "blocked" => "Blocked",
        "review" => "Review",
        "done" => "Done",
        "cancelled" => "Cancelled",
        other => other,
    }
    .to_string()
}

pub fn source_label(value: &str) -> String {
    match value {
        "manual" => "Manual",
        "nixa" => "Nixa",
        "agent" => "Agent",
        other => other,
    }
    .to_string()
}

pub fn group_label(value: &str) -> String {
    match value {
        "project" => "Project",
        "nixa-gap" => "Nixa gap",
        other => other,
    }
    .to_string()
}

pub fn group_detail(value: &str) -> &'static str {
    match value {
        "project" => "Normal work in a space",
        "nixa-gap" => "A gap or improvement in Nixa's own brain. Lands in the nixa space.",
        _ => "",
    }
}

pub fn kind_label(value: &str) -> String {
    match value {
        "note" => "Note",
        "progress" => "Progress",
        "question" => "Question",
        "answer" => "Answer",
        "result" => "Result",
        "handoff" => "Handoff",
        "error" => "Error",
        "claim" => "Claim",
        other => other,
    }
    .to_string()
}

pub fn author_label(value: &str) -> String {
    match value {
        "user" => "You",
        "decision" => "Nixa decision",
        "nixa" => "Nixa",
        "agent" => "Agent",
        "system" => "System",
        other => other,
    }
    .to_string()
}

pub fn blocked_label(value: &str) -> String {
    match value {
        "user" => "You",
        "task" => "Another task",
        "other" => "Something else",
        other => other,
    }
    .to_string()
}

pub fn get<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k).filter(|x| !x.is_null()))
}

pub fn text(v: &Value, keys: &[&str]) -> String {
    match get(v, keys) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

pub fn num(v: &Value, keys: &[&str]) -> i64 {
    match get(v, keys) {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

pub fn flag(v: &Value, keys: &[&str]) -> bool {
    matches!(get(v, keys), Some(Value::Bool(true)))
}

pub fn texts(v: &Value, keys: &[&str]) -> Vec<String> {
    match get(v, keys) {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn items<'a>(v: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    match get(v, keys) {
        Some(Value::Array(list)) => list.iter().collect(),
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub status: String,
    pub space_id: String,
    pub space_name: String,
    pub assigned_kind: String,
    pub assigned: String,
    pub claimed_by: String,
    pub terminal_id: String,
    pub claimed_at: i64,
    pub active_at: i64,
    pub needs_user: bool,
    pub blocked_on: String,
    pub source: String,
    pub group: String,
    pub created_by: String,
    pub created_terminal: String,
    pub repo: String,
    pub branch: String,
    pub commit_sha: String,
    pub pr_url: String,
    pub uncommitted: Vec<String>,
    pub then_member: String,
    pub workflow: String,
    pub archived_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: i64,
    pub comments: i64,
    pub run_box: String,
}

pub fn parse_task(v: &Value) -> Option<Task> {
    let id = num(v, &["id"]);
    if id <= 0 {
        return None;
    }
    Some(Task {
        id,
        title: text(v, &["title"]),
        description: text(v, &["description"]),
        status: text(v, &["status"]),
        space_id: text(v, &["space_id", "spaceId"]),
        space_name: text(v, &["space_name", "spaceName"]),
        assigned_kind: text(v, &["assigned_kind", "assignedKind"]),
        assigned: text(v, &["assigned"]),
        claimed_by: text(v, &["claimed_by", "claimedBy"]),
        terminal_id: text(v, &["terminal_id", "terminalId"]),
        claimed_at: num(v, &["claimed_at", "claimedAt"]),
        active_at: num(v, &["active_at", "activeAt"]),
        needs_user: flag(v, &["needs_user", "needsUser"]),
        blocked_on: text(v, &["blocked_on", "blockedOn"]),
        source: text(v, &["source"]),
        group: text(v, &["group"]),
        created_by: text(v, &["created_by", "createdBy"]),
        created_terminal: text(v, &["created_terminal", "createdTerminal"]),
        repo: text(v, &["repo"]),
        branch: text(v, &["branch"]),
        commit_sha: text(v, &["commit_sha", "commitSha"]),
        pr_url: text(v, &["pr_url", "prUrl"]),
        uncommitted: texts(v, &["uncommitted"]),
        then_member: text(v, &["then_member", "thenMember"]),
        workflow: text(v, &["workflow"]),
        archived_at: num(v, &["archived_at", "archivedAt"]),
        created_at: num(v, &["created_at", "createdAt"]),
        updated_at: num(v, &["updated_at", "updatedAt"]),
        closed_at: num(v, &["closed_at", "closedAt"]),
        comments: num(v, &["comments"]),
        run_box: text(v, &["run_box", "runBox"]),
    })
}

pub fn merge_task(old: &Task, v: &Value) -> Task {
    let Some(mut fresh) = parse_task(v) else {
        return old.clone();
    };
    if get(v, &["comments"]).is_none() {
        fresh.comments = old.comments;
    }
    if get(v, &["run_box", "runBox"]).is_none() {
        fresh.run_box = old.run_box.clone();
    }
    fresh
}

pub fn parse_tasks(v: &Value) -> Vec<Task> {
    items(v, &["tasks"])
        .into_iter()
        .filter_map(parse_task)
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links {
    pub files: Vec<String>,
    pub branch: String,
    pub commit: String,
    pub pr: String,
    pub urls: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comment {
    pub id: String,
    pub at: i64,
    pub author_kind: String,
    pub author: String,
    pub model: String,
    pub terminal_id: String,
    pub kind: String,
    pub text: String,
    pub links: Links,
}

pub fn parse_comment(v: &Value) -> Comment {
    let links = get(v, &["links"]).cloned().unwrap_or(Value::Null);
    Comment {
        id: text(v, &["id"]),
        at: num(v, &["at"]),
        author_kind: text(v, &["author_kind", "authorKind"]),
        author: text(v, &["author"]),
        model: text(v, &["model"]),
        terminal_id: text(v, &["terminal_id", "terminalId"]),
        kind: text(v, &["kind"]),
        text: text(v, &["text"]),
        links: Links {
            files: texts(&links, &["files"]),
            branch: text(&links, &["branch"]),
            commit: text(&links, &["commit"]),
            pr: text(&links, &["pr"]),
            urls: texts(&links, &["urls"]),
        },
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub state: String,
    pub line: String,
    pub title: String,
    pub ask_why: String,
}

pub fn parse_run(v: &Value) -> Option<Run> {
    let run = get(v, &["run"])?;
    if !run.is_object() {
        return None;
    }
    let ask = get(run, &["ask"]).cloned().unwrap_or(Value::Null);
    Some(Run {
        state: text(run, &["state"]),
        line: text(run, &["line"]),
        title: text(run, &["title"]),
        ask_why: text(&ask, &["why"]),
    })
}

pub fn run_active(run: Option<&Run>) -> bool {
    run.is_some_and(|r| ACTIVE_RUN.contains(&r.state.as_str()))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub task: Task,
    pub comments: Vec<Comment>,
    pub run: Option<Run>,
}

pub fn parse_detail(v: &Value) -> Option<Detail> {
    let task = parse_task(get(v, &["task"])?)?;
    Some(Detail {
        task,
        comments: items(v, &["comments"])
            .into_iter()
            .map(parse_comment)
            .collect(),
        run: parse_run(v),
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceOpt {
    pub id: String,
    pub name: String,
    pub folder: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Person {
    pub value: String,
    pub label: String,
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub name: String,
    pub title: String,
    pub broken: String,
    pub when: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub spaces: Vec<SpaceOpt>,
    pub assignees: Vec<Person>,
    pub statuses: Vec<String>,
    pub sources: Vec<String>,
    pub groups: Vec<String>,
    pub workflows: Vec<Flow>,
}

fn or_default(list: Vec<String>, fallback: &[&str]) -> Vec<String> {
    if list.is_empty() {
        fallback.iter().map(|s| s.to_string()).collect()
    } else {
        list
    }
}

pub fn parse_flows(v: &Value) -> Vec<Flow> {
    items(v, &["workflows"])
        .into_iter()
        .map(|f| Flow {
            name: text(f, &["name"]),
            title: text(f, &["title"]),
            broken: text(f, &["broken"]),
            when: text(f, &["when"]),
        })
        .filter(|f| !f.name.is_empty())
        .collect()
}

pub fn parse_options(v: &Value) -> Options {
    Options {
        spaces: items(v, &["spaces"])
            .into_iter()
            .map(|s| SpaceOpt {
                id: text(s, &["id"]),
                name: text(s, &["name"]),
                folder: text(s, &["folder"]),
            })
            .filter(|s| !s.id.is_empty())
            .collect(),
        assignees: items(v, &["assignees"])
            .into_iter()
            .map(|p| Person {
                value: text(p, &["value"]),
                label: text(p, &["label"]),
                kind: text(p, &["kind"]),
                detail: text(p, &["detail"]),
            })
            .filter(|p| !p.value.is_empty())
            .collect(),
        statuses: or_default(texts(v, &["statuses"]), &STATUSES),
        sources: or_default(texts(v, &["sources"]), &SOURCES),
        groups: or_default(texts(v, &["groups"]), &GROUPS),
        workflows: parse_flows(v),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub space: String,
    pub status: String,
    pub group: String,
    pub source: String,
    pub assigned: String,
    pub done: bool,
    pub archived: bool,
}

impl Filters {
    pub fn any(&self) -> bool {
        !self.space.is_empty()
            || !self.status.is_empty()
            || !self.group.is_empty()
            || !self.source.is_empty()
            || !self.assigned.is_empty()
    }
}

pub fn list_args(f: &Filters, typed: &str) -> Value {
    let open = OPEN_STATUSES.join(",");
    let status = if f.archived {
        if f.status == "open" {
            open
        } else {
            f.status.clone()
        }
    } else if f.status == "open" || (f.status.is_empty() && !f.done) {
        open
    } else {
        f.status.clone()
    };
    let mut out = Map::new();
    if !f.space.is_empty() && f.space != "none" {
        out.insert("space".into(), json!(f.space));
    }
    if !status.is_empty() {
        out.insert("status".into(), json!(status));
    }
    if !f.group.is_empty() {
        out.insert("group".into(), json!(f.group));
    }
    if !f.source.is_empty() {
        out.insert("source".into(), json!(f.source));
    }
    if !f.assigned.is_empty() && f.assigned != "none" {
        out.insert("assigned".into(), json!(f.assigned));
    }
    let typed = typed.trim();
    if !typed.is_empty() {
        out.insert("q".into(), json!(typed));
    }
    if f.done || f.archived || CLOSED_STATUSES.contains(&f.status.as_str()) {
        out.insert("old".into(), json!(true));
    }
    if f.archived {
        out.insert("archived".into(), json!(true));
    }
    Value::Object(out)
}

pub fn shown(tasks: &[Task], f: &Filters) -> Vec<Task> {
    tasks
        .iter()
        .filter(|t| {
            (f.space != "none" || t.space_id.is_empty())
                && (f.assigned != "none" || t.assigned.is_empty())
        })
        .cloned()
        .collect()
}

pub fn group_of(task: &Task) -> String {
    if GROUPS.contains(&task.group.as_str()) {
        task.group.clone()
    } else {
        "project".into()
    }
}

fn member_value(options: &Options, id: &str) -> String {
    let id = id.strip_prefix(CUSTOM_VALUE).or_else(|| id.strip_prefix(MEMBER_VALUE)).unwrap_or(id);
    let custom = format!("{CUSTOM_VALUE}{id}");
    let member = format!("{MEMBER_VALUE}{id}");
    options.assignees.iter()
        .find(|p| p.value == custom || p.value == member)
        .map(|p| p.value.clone())
        .unwrap_or(custom)
}

pub fn assigned_value(options: &Options, task: &Task) -> String {
    if !task.assigned.is_empty() && matches!(task.assigned_kind.as_str(), "member" | "custom") {
        member_value(options, &task.assigned)
    } else {
        task.assigned.clone()
    }
}

pub fn then_value(options: &Options, task: &Task) -> String {
    if task.then_member.is_empty() {
        String::new()
    } else {
        member_value(options, &task.then_member)
    }
}

pub fn is_closed(status: &str) -> bool {
    CLOSED_STATUSES.contains(&status)
}

pub fn special_holder(task: &Task) -> bool {
    task.terminal_id.starts_with("starting-") || task.terminal_id.starts_with("run-")
}

pub fn agent_held(task: &Task) -> bool {
    !task.terminal_id.is_empty()
        && !special_holder(task)
        && ["doing", "blocked", "review"].contains(&task.status.as_str())
}

pub fn worked_on(task: &Task, comments: &[Comment]) -> bool {
    !task.uncommitted.is_empty()
        || comments.iter().any(|c| {
            c.author_kind == "agent"
                || (c.author_kind == "system"
                    && c.kind == "handoff"
                    && c.text.starts_with(SAVED_WORK))
        })
}

pub fn nixa_space_id(spaces: &[SpaceOpt]) -> String {
    let base = |s: &SpaceOpt| {
        s.folder
            .split('/')
            .rfind(|p| !p.is_empty())
            .unwrap_or("")
            .to_lowercase()
    };
    spaces
        .iter()
        .find(|s| s.name.trim().eq_ignore_ascii_case("nixa"))
        .or_else(|| spaces.iter().find(|s| base(s) == "nixa"))
        .map(|s| s.id.clone())
        .unwrap_or_default()
}

pub fn ago(at: i64, now: i64) -> String {
    if at <= 0 {
        return String::new();
    }
    let minutes = ((now - at).max(0) as f64 / 60_000.0).round() as i64;
    if minutes < 1 {
        return "just now".into();
    }
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format!("{hours} h ago");
    }
    let days = (hours as f64 / 24.0).round() as i64;
    if days == 1 {
        "1 day ago".into()
    } else {
        format!("{days} days ago")
    }
}

pub fn clock_text(at: i64, offset_min: i64) -> String {
    let local = at + offset_min * 60_000;
    let days = local.div_euclid(nixa_store::time::DAY_MS);
    let (y, m, d) = nixa_store::time::civil_from_days(days);
    let rest = local.rem_euclid(nixa_store::time::DAY_MS) / 60_000;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rest / 60, rest % 60)
}

pub fn when(at: i64, now: i64, offset_min: i64) -> String {
    if at <= 0 {
        return String::new();
    }
    format!("{} ({})", clock_text(at, offset_min), ago(at, now))
}

pub fn now_ms() -> i64 {
    nixa_store::time::now_ms()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Opt {
    pub value: String,
    pub label: String,
    pub detail: String,
}

pub fn opt(value: &str, label: &str, detail: &str) -> Opt {
    Opt {
        value: value.into(),
        label: label.into(),
        detail: detail.into(),
    }
}

pub fn matches(options: &[Opt], typed: &str) -> Vec<Opt> {
    let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    options
        .iter()
        .filter(|o| {
            let hay = format!("{} {} {}", o.label, o.value, o.detail).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .cloned()
        .collect()
}

pub fn with_gone(mut options: Vec<Opt>, value: &str, label: &str, detail: &str) -> Vec<Opt> {
    if !value.is_empty() && !options.iter().any(|o| o.value == value) {
        options.push(opt(
            value,
            if label.is_empty() { value } else { label },
            detail,
        ));
    }
    options
}

pub fn space_options(o: &Options, none: &str) -> Vec<Opt> {
    let mut out = vec![opt(none, "No space", "")];
    out.extend(o.spaces.iter().map(|s| opt(&s.id, &s.name, &s.folder)));
    out
}

pub fn person_detail(p: &Person) -> String {
    let more = if p.detail.is_empty() {
        String::new()
    } else {
        format!(" · {}", p.detail)
    };
    if p.kind == "member" {
        format!("Team member{more}")
    } else if p.value.starts_with("custom:") {
        format!("Custom agent{more}")
    } else {
        "Agent type".into()
    }
}

pub fn assignee_options(o: &Options, none: &str) -> Vec<Opt> {
    let mut out = vec![opt(none, "Nobody", "Nixa picks or you pick later")];
    out.extend(
        o.assignees
            .iter()
            .map(|p| opt(&p.value, &p.label, &person_detail(p))),
    );
    out
}

pub fn then_options(o: &Options) -> Vec<Opt> {
    let mut out = vec![opt(
        "",
        "Usual next step",
        "A reviewer's findings go to Developer, then back to the reviewer once",
    )];
    out.extend(o.assignees.iter().filter(|p| matches!(p.kind.as_str(), "member" | "custom")).map(|p| {
        let more = if p.detail.is_empty() {
            String::new()
        } else {
            format!(" · {}", p.detail)
        };
        opt(&p.value, &p.label, &format!("Takes the next step{more}"))
    }));
    out
}

pub fn workflow_options(o: &Options) -> Vec<Opt> {
    let mut out = vec![opt("none", "No workflow", "The task uses the normal flow")];
    out.extend(o.workflows.iter().map(|f| {
        let label = if f.title.is_empty() {
            &f.name
        } else {
            &f.title
        };
        let detail = if f.broken.is_empty() {
            f.when.clone()
        } else {
            format!("Broken: {}", f.broken)
        };
        opt(&f.name, label, &detail)
    }));
    out
}

pub fn group_options(o: &Options) -> Vec<Opt> {
    o.groups
        .iter()
        .map(|g| opt(g, &group_label(g), group_detail(g)))
        .collect()
}

pub fn status_options(o: &Options) -> Vec<Opt> {
    o.statuses
        .iter()
        .map(|s| opt(s, &status_label(s), ""))
        .collect()
}

pub fn filter_status_options(o: &Options, done: bool) -> Vec<Opt> {
    let mut out = vec![if done {
        opt("", "All statuses", "Open and finished tasks")
    } else {
        opt(
            "",
            "All open",
            "Not done and not cancelled. Show done adds finished tasks.",
        )
    }];
    if done {
        out.push(opt("open", "Open", "Not done and not cancelled"));
    }
    out.extend(status_options(o));
    out
}

pub fn comment_kind_options() -> Vec<Opt> {
    COMMENT_KINDS
        .iter()
        .map(|k| opt(k, &kind_label(k), ""))
        .collect()
}

pub fn blocked_options() -> Vec<Opt> {
    BLOCKED_ON
        .iter()
        .map(|k| opt(k, &blocked_label(k), ""))
        .collect()
}

pub fn space_name(o: &Options, task: &Task) -> String {
    if task.space_id.is_empty() {
        return String::new();
    }
    o.spaces
        .iter()
        .find(|s| s.id == task.space_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| {
            if task.space_name.is_empty() {
                task.space_id.clone()
            } else {
                task.space_name.clone()
            }
        })
}

pub fn assigned_name(o: &Options, task: &Task) -> String {
    let value = assigned_value(o, task);
    if value.is_empty() {
        return String::new();
    }
    o.assignees
        .iter()
        .find(|p| p.value == value)
        .map(|p| p.label.clone())
        .unwrap_or_else(|| task.assigned.clone())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Id,
    Title,
    Status,
    Assigned,
}

impl Col {
    pub fn label(self) -> &'static str {
        match self {
            Col::Id => "#",
            Col::Title => "Title",
            Col::Status => "Status",
            Col::Assigned => "Assigned",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Col::Id => "number",
            Col::Title => "title",
            Col::Status => "status",
            Col::Assigned => "assigned",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Col::Id => "id",
            Col::Title => "title",
            Col::Status => "status",
            Col::Assigned => "assigned",
        }
    }
}

pub const COLUMNS: [Col; 4] = [Col::Id, Col::Title, Col::Status, Col::Assigned];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub col: Col,
    pub up: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            col: Col::Id,
            up: true,
        }
    }
}

pub fn flip(sort: Sort, col: Col) -> Sort {
    if sort.col == col {
        Sort { col, up: !sort.up }
    } else {
        Sort { col, up: true }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum Key {
    Num(i64),
    Text(String),
}

fn sort_key(task: &Task, col: Col, o: &Options) -> (bool, Key) {
    match col {
        Col::Id => (false, Key::Num(task.id)),
        Col::Title => (task.title.is_empty(), Key::Text(task.title.to_lowercase())),
        Col::Status => (
            false,
            Key::Num(
                STATUSES
                    .iter()
                    .position(|s| *s == task.status)
                    .map(|p| p as i64)
                    .unwrap_or(-1),
            ),
        ),
        Col::Assigned => (
            task.assigned.is_empty(),
            Key::Text(assigned_name(o, task).to_lowercase()),
        ),
    }
}

pub fn sort_tasks(list: &[Task], sort: Sort, o: &Options) -> Vec<Task> {
    let mut out = list.to_vec();
    out.sort_by(|a, b| {
        let (ea, ka) = sort_key(a, sort.col, o);
        let (eb, kb) = sort_key(b, sort.col, o);
        if ea != eb {
            return if ea {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
        }
        let order = ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal);
        let order = if sort.up { order } else { order.reverse() };
        order
            .then(a.created_at.cmp(&b.created_at))
            .then(a.id.cmp(&b.id))
    });
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub key: String,
    pub name: String,
    pub tasks: Vec<Task>,
}

pub fn group_tasks(list: &[Task], o: &Options) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for task in list {
        match groups.iter_mut().find(|g| g.key == task.space_id) {
            Some(g) => g.tasks.push(task.clone()),
            None => groups.push(Group {
                key: task.space_id.clone(),
                name: if task.space_id.is_empty() {
                    "No space".into()
                } else {
                    space_name(o, task)
                },
                tasks: vec![task.clone()],
            }),
        }
    }
    let count = o.spaces.len();
    let rank = |key: &str| {
        if key.is_empty() {
            count + 1
        } else {
            o.spaces.iter().position(|s| s.id == key).unwrap_or(count)
        }
    };
    groups.sort_by(|a, b| {
        rank(&a.key)
            .cmp(&rank(&b.key))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    groups
}

pub fn count_text(n: usize) -> String {
    if n == 1 {
        "1 task".into()
    } else {
        format!("{n} tasks")
    }
}

pub fn comments_text(n: usize) -> String {
    if n == 1 {
        "1 comment".into()
    } else {
        format!("{n} comments")
    }
}

pub fn row_label(task: &Task) -> String {
    let title = if task.title.is_empty() {
        "(no title)"
    } else {
        &task.title
    };
    format!(
        "#{} {}{}",
        task.id,
        title,
        if task.needs_user {
            ", waits for you"
        } else {
            ""
        }
    )
}

pub fn run_tip(task: &Task, space: &str, assigned: &str) -> String {
    if task.space_id.is_empty() && task.assigned.is_empty() {
        return "Run now. Pick a Space and an Assigned agent first.".into();
    }
    if task.space_id.is_empty() {
        return "Run now. Pick a Space first.".into();
    }
    let place = if space.is_empty() { "its space" } else { space };
    if task.assigned.is_empty() && !task.workflow.is_empty() {
        return format!(
            "Run now: start the workflow {} in {place}; its first box picks the agent",
            task.workflow
        );
    }
    if task.assigned.is_empty() {
        return "Run now. Pick an Assigned agent first.".into();
    }
    let who = if assigned.is_empty() {
        &task.assigned
    } else {
        assigned
    };
    let flow = if task.workflow.is_empty() {
        String::new()
    } else {
        format!(" with the workflow {}", task.workflow)
    };
    format!("Run now: start {who} in {place}{flow}")
}

pub fn runnable(task: &Task) -> bool {
    (task.status == "todo" || task.status == "inbox") && task.terminal_id.is_empty()
}

pub fn ready_to_run(task: &Task) -> bool {
    !task.space_id.is_empty() && (!task.assigned.is_empty() || !task.workflow.is_empty())
}

pub fn needs_pick_text(id: i64) -> String {
    format!(
        "Task #{id} needs a Space and an Assigned agent before it can run. Pick them in the panel on the right, then press Run now."
    )
}

pub fn continue_tip(task: &Task, held: bool) -> String {
    if task.space_id.is_empty() || task.assigned.is_empty() {
        return "Pick a Space and an Assigned agent first".into();
    }
    if task.archived_at > 0 {
        return "Unarchive the task first".into();
    }
    if held {
        let who = if task.claimed_by.is_empty() {
            "the agent"
        } else {
            &task.claimed_by
        };
        format!(
            "Tell {who} in terminal {} to go on where it stopped. If that agent is gone, Nixa starts a new one with the work so far.",
            task.terminal_id
        )
    } else {
        "Start again from where the work stopped. Nixa gives the agent the changed files, the last screen and the notes. It uses the old terminal when that agent is still open.".into()
    }
}

pub fn continue_note(how: &str, terminal: &str) -> String {
    match how {
        "told" => format!("Told the agent in terminal {terminal} to go on."),
        "back" => format!("Back to terminal {terminal}, which did the earlier work."),
        "started" => format!("Started in terminal {terminal} with the work so far."),
        "handed" => format!("Handed over. The new agent is in terminal {terminal}."),
        _ => String::new(),
    }
}

pub fn handover_text(task: &Task, label: &str) -> String {
    let who = if label.is_empty() {
        "the new agent"
    } else {
        label
    };
    if agent_held(task) {
        format!(
            "Nixa saves the work so far (changed files, the last screen and the notes), closes terminal {}, and starts {who} in a new terminal with all of it.",
            task.terminal_id
        )
    } else {
        format!(
            "Nixa starts {who} in a new terminal with the work so far (changed files, the saved screen and the notes)."
        )
    }
}

pub fn delete_text(_task: &Task) -> &'static str {
    "The task and all its comments are removed. This cannot be undone. Archive the task instead to keep the record."
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Undo {
    pub id: i64,
    pub status: String,
    pub back: String,
    pub title: String,
}

pub fn undo_of(task: &Task, status: &str) -> Undo {
    Undo {
        id: task.id,
        status: status.into(),
        back: if task.status == "doing" {
            "todo".into()
        } else {
            task.status.clone()
        },
        title: task.title.clone(),
    }
}

pub fn undo_text(u: &Undo) -> String {
    let title = if u.title.is_empty() {
        String::new()
    } else if u.title.chars().count() > 60 {
        format!(" {}…", u.title.chars().take(60).collect::<String>())
    } else {
        format!(" {}", u.title)
    };
    format!(
        "#{}{title} is {}. It left the list.",
        u.id,
        if u.status == "done" {
            "done"
        } else {
            "cancelled"
        }
    )
}

pub fn undo_tip(u: &Undo) -> String {
    format!("Put #{} back to {}", u.id, status_label(&u.back))
}

pub fn source_line(task: &Task) -> String {
    source_label(&task.source)
}

pub fn git_line(task: &Task) -> String {
    [
        (!task.repo.is_empty()).then(|| format!("repo {}", task.repo)),
        (!task.branch.is_empty()).then(|| format!("branch {}", task.branch)),
        (!task.commit_sha.is_empty()).then(|| format!("commit {}", task.commit_sha)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

pub fn claim_line(task: &Task, now: i64) -> String {
    let who = if task.claimed_by.is_empty() {
        "an agent"
    } else {
        &task.claimed_by
    };
    let mut out = format!("{who} in terminal {}", task.terminal_id);
    if task.claimed_at > 0 {
        out.push_str(&format!(", since {}", ago(task.claimed_at, now)));
    }
    if task.active_at > 0 {
        out.push_str(&format!(", last active {}", ago(task.active_at, now)));
    }
    out
}

pub fn made_by_line(task: &Task) -> String {
    let who = if task.created_by.is_empty() {
        "unknown"
    } else {
        &task.created_by
    };
    if task.created_terminal.is_empty() {
        who.to_string()
    } else {
        format!("{who} in terminal {}", task.created_terminal)
    }
}

pub fn comment_meta(c: &Comment, now: i64) -> String {
    let kind_of = author_label(&c.author_kind);
    let who = if c.author.is_empty() {
        kind_of.clone()
    } else {
        c.author.clone()
    };
    let who = if who == kind_of {
        who
    } else {
        format!("{who} ({kind_of})")
    };
    [
        who,
        c.model.clone(),
        kind_label(&c.kind),
        if c.terminal_id.is_empty() {
            String::new()
        } else {
            format!("terminal {}", c.terminal_id)
        },
        ago(c.at, now),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

pub fn link_notes(l: &Links) -> Vec<String> {
    [
        (!l.files.is_empty()).then(|| format!("Files: {}", l.files.join(", "))),
        (!l.branch.is_empty()).then(|| format!("Branch: {}", l.branch)),
        (!l.commit.is_empty()).then(|| format!("Commit: {}", l.commit)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub fn link_urls(l: &Links) -> Vec<(String, String)> {
    let mut urls: Vec<String> = Vec::new();
    if !l.pr.is_empty() {
        urls.push(l.pr.clone());
    }
    urls.extend(l.urls.iter().cloned());
    urls.into_iter()
        .enumerate()
        .map(|(i, u)| {
            (
                if u == l.pr {
                    "Open PR".to_string()
                } else {
                    format!("Link {}", i + 1)
                },
                u,
            )
        })
        .collect()
}

pub fn safe_link(url: &str) -> bool {
    let lower = url.to_lowercase();
    let web = ["http://", "https://"]
        .iter()
        .find_map(|p| lower.strip_prefix(p))
        .is_some_and(|rest| {
            rest.chars()
                .next()
                .is_some_and(|c| !c.is_whitespace() && c != '/')
        });
    let mail = lower.strip_prefix("mailto:").is_some_and(|rest| {
        let (user, host) = rest.split_once('@').unwrap_or(("", ""));
        !user.is_empty() && !host.is_empty() && !rest.contains(char::is_whitespace)
    });
    web || mail
}

const HTML_TAGS: [&str; 30] = [
    "p",
    "br",
    "ul",
    "ol",
    "li",
    "strong",
    "b",
    "em",
    "i",
    "u",
    "s",
    "a",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "div",
    "span",
    "code",
    "pre",
    "blockquote",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
    "hr",
];

pub(super) fn tag_name(inner: &str) -> String {
    inner
        .trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

pub fn is_html(text: &str) -> bool {
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            return false;
        };
        let name = tag_name(&after[..end]);
        if HTML_TAGS.contains(&name.as_str()) {
            return true;
        }
        rest = &after[end..];
    }
    false
}

pub(super) fn entity(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub space: String,
    pub group: String,
    pub assign: String,
    pub then: String,
}

pub fn empty_draft(space: &str) -> Draft {
    Draft {
        space: space.into(),
        group: "project".into(),
        assign: String::new(),
        then: String::new(),
    }
}

pub fn add_args(title: &str, description: &str, d: &Draft) -> Value {
    let mut out = Map::new();
    out.insert("title".into(), json!(title));
    if !description.is_empty() {
        out.insert("description".into(), json!(description));
    }
    out.insert(
        "group".into(),
        json!(if d.group.is_empty() {
            "project"
        } else {
            &d.group
        }),
    );
    for (key, value) in [
        ("space", &d.space),
        ("assign", &d.assign),
        ("then", &d.then),
    ] {
        if !value.is_empty() {
            out.insert(key.into(), json!(value));
        }
    }
    Value::Object(out)
}

pub fn text_patch(task: &Task, title: &str, description: &str) -> Option<Value> {
    let mut out = Map::new();
    out.insert("id".into(), json!(task.id));
    if title != task.title {
        out.insert("title".into(), json!(title));
    }
    if description != task.description {
        out.insert("description".into(), json!(description));
    }
    (out.len() > 1).then_some(Value::Object(out))
}

pub fn group_patch(task: &Task, group: &str, spaces: &[SpaceOpt]) -> Option<Value> {
    if group == task.group {
        return None;
    }
    let mut out = Map::new();
    out.insert("id".into(), json!(task.id));
    out.insert("group".into(), json!(group));
    if group == "nixa-gap" && task.terminal_id.is_empty() {
        let nixa = nixa_space_id(spaces);
        if !nixa.is_empty() && nixa != task.space_id {
            out.insert("space".into(), json!(nixa));
        }
    }
    Some(Value::Object(out))
}

pub fn waiting_count(tasks: &[Task]) -> usize {
    tasks.iter().filter(|t| t.needs_user).count()
}

pub fn window_title(waiting: usize) -> String {
    if waiting > 0 {
        format!("Tasks ({waiting})")
    } else {
        "Tasks".into()
    }
}

pub fn waiting_text(waiting: usize) -> String {
    format!("{waiting} waiting for you")
}

pub fn empty_text(has_tasks: bool, filtered: bool, done: bool) -> &'static str {
    if has_tasks || filtered {
        "No task matches these filters."
    } else if done {
        "No tasks yet. Click New task to add one."
    } else {
        "No open tasks. Click New task to add one, or Show done to see finished tasks."
    }
}

pub fn more_rows_text(n: usize) -> String {
    format!("{n} more tasks are not shown. Use the filters to narrow the list.")
}

pub fn panel_title(task: &Task) -> String {
    format!(
        "#{} · {}",
        task.id,
        if task.title.is_empty() {
            status_label(&task.status)
        } else {
            task.title.clone()
        }
    )
}
