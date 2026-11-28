use super::lock::{locked_text, space_locked, task_locked};
use super::start::{Seen, checkpoint, continue_task, resume_todos, start_now};
use super::{changed, db, places, status_event, view, waiting_lines};
use crate::app::App;
use crate::tools::{Args, flag, number, text};
use axum::Router;
use axum::Extension;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nixa_store::spaces::{MAX_DOING_TOP, SpacePatch};
use nixa_store::tasks;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

pub const WS_KIND: &str = "tasks";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    User,
    Brain,
    Terminal(String),
}

impl Role {
    pub fn word(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Brain => "brain",
            Role::Terminal(_) => "agent",
        }
    }
}

type Shared = State<Arc<App>>;
type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

fn addr_of(peer: Peer) -> Option<SocketAddr> {
    peer.map(|Extension(ConnectInfo(addr))| addr)
}

fn ok(value: Value) -> Response {
    let mut body = json!({"ok": true});
    if let (Some(out), Value::Object(more)) = (body.as_object_mut(), value) {
        out.extend(more);
    }
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn bad(code: StatusCode, message: impl Into<String>) -> Response {
    (
        code,
        axum::Json(json!({"ok": false, "error": message.into()})),
    )
        .into_response()
}

fn fail(e: anyhow::Error) -> String {
    e.to_string()
}

fn args_of(body: &Value) -> Args {
    body.as_object().cloned().unwrap_or_default()
}

fn id_of(args: &Args) -> Result<i64, String> {
    args.get("id")
        .and_then(|v| {
            v.as_i64().or_else(|| {
                v.as_str()
                    .and_then(|s| s.trim().trim_start_matches('#').parse().ok())
            })
        })
        .filter(|n| *n > 0)
        .ok_or_else(|| "id is needed: a task number.".to_string())
}

pub async fn caller(terminal: &str, peer: Option<SocketAddr>) -> Result<String, String> {
    let id = crate::engine::term_id(terminal).map_err(|e| e.to_string())?;
    match crate::engine::get(&id).await {
        Ok(term) if term.running => {
            let name = term.id.to_string();
            crate::extras::peer::from_terminal(peer, &name, term.pid).await?;
            Ok(name)
        }
        Ok(_) => Err(format!("Terminal {terminal} does not run.")),
        Err(_) => Err(format!("Terminal {terminal} is not a Nixa terminal.")),
    }
}

async fn role_of(
    app: &App,
    headers: &HeaderMap,
    args: &Args,
    terminal_ok: bool,
    peer: Option<SocketAddr>,
) -> Result<Role, Option<String>> {
    if crate::window_key::allowed(app, headers) {
        return Ok(Role::User);
    }
    if terminal_ok && let Some(terminal) = text(args, "terminal") {
        return caller(&terminal, peer).await.map(Role::Terminal).map_err(Some);
    }
    Err(None)
}

fn answer(result: Result<Value, String>) -> Response {
    match result {
        Ok(value) => ok(value),
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/tasks/queue", get(queue_get))
        .route("/tasks/next", get(next_get))
        .route("/tasks/spaces", get(spaces_get))
        .route("/tasks/spaces/{space}", post(space_post))
        .route("/tasks/resume", post(resume_post))
        .route("/tasks/changed", post(changed_post))
        .route("/tasks/{id}/start", post(start_post))
        .route("/tasks/{id}/continue", post(continue_post))
        .route("/tasks/{id}/checkpoint", post(checkpoint_post))
        .route("/tasks/{id}/plan", get(plan_get).post(plan_post))
}

async fn run(
    app: Arc<App>,
    headers: HeaderMap,
    name: &str,
    args: Args,
    terminal_ok: bool,
    peer: Option<SocketAddr>,
) -> Response {
    let role = match role_of(&app, &headers, &args, terminal_ok, peer).await {
        Ok(role) => role,
        Err(Some(error)) => return bad(StatusCode::FORBIDDEN, error),
        Err(None) => return crate::window_key::refused(),
    };
    answer(action(&app, name, &args, &role).await)
}

async fn queue_get(State(app): Shared, headers: HeaderMap) -> Response {
    run(app, headers, "queue", Args::new(), false, None).await
}

async fn next_get(
    State(app): Shared,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let args: Args = query
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    run(app, headers, "next", args, true, addr_of(peer)).await
}

async fn spaces_get(State(app): Shared, headers: HeaderMap) -> Response {
    run(app, headers, "spaces", Args::new(), false, None).await
}

async fn space_post(
    State(app): Shared,
    headers: HeaderMap,
    Path(space): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut args = args_of(&body);
    args.insert("space".into(), Value::String(space));
    run(app, headers, "space", args, false, None).await
}

async fn resume_post(
    State(app): Shared,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    run(app, headers, "resume", args_of(&body), false, None).await
}

async fn changed_post(
    State(app): Shared,
    headers: HeaderMap,
    peer: Peer,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut args = args_of(&body);
    if !crate::window_key::allowed(&app, &headers) && !args.contains_key("terminal") && crate::gate::allowed(&app, &headers) {
        args.insert("role".into(), json!("nixa"));
        return answer(action(&app, "changed", &args, &Role::User).await);
    }
    run(app, headers, "changed", args, true, addr_of(peer)).await
}

fn with_id(body: &Value, id: i64) -> Args {
    let mut args = args_of(body);
    args.insert("id".into(), json!(id));
    args
}

async fn start_post(
    State(app): Shared,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let body = body.map(|b| b.0).unwrap_or_else(|| json!({}));
    run(app, headers, "start", with_id(&body, id), false, None).await
}

async fn continue_post(
    State(app): Shared,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let body = body.map(|b| b.0).unwrap_or_else(|| json!({}));
    run(app, headers, "continue", with_id(&body, id), false, None).await
}

async fn checkpoint_post(
    State(app): Shared,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<i64>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let body = body.map(|b| b.0).unwrap_or_else(|| json!({}));
    run(app, headers, "checkpoint", with_id(&body, id), true, addr_of(peer)).await
}

async fn plan_get(
    State(app): Shared,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut args = Args::new();
    args.insert("id".into(), json!(id));
    if let Some(terminal) = query.get("terminal") {
        args.insert("terminal".into(), Value::String(terminal.clone()));
    }
    run(app, headers, "plan", args, true, addr_of(peer)).await
}

async fn plan_post(
    State(app): Shared,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<i64>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let body = body.map(|b| b.0).unwrap_or_else(|| json!({}));
    run(app, headers, "plan", with_id(&body, id), true, addr_of(peer)).await
}

pub fn is_ws(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(WS_KIND)
}

pub async fn ws_request(app: &Arc<App>, message: &Value) -> Option<Value> {
    if !is_ws(message) {
        return None;
    }
    let request = message.get("request").and_then(Value::as_str)?.to_string();
    let mut args: Args = match message.get("args") {
        Some(Value::Object(map)) => map.clone(),
        _ => message.as_object().cloned().unwrap_or_default(),
    };
    args.remove("type");
    args.remove("request");
    let name = message
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| text(&args, "action"))
        .unwrap_or_else(|| "queue".into());
    args.remove("action");
    let kind = format!("{WS_KIND}_reply");
    Some(match action(app, &name, &args, &Role::User).await {
        Ok(data) => json!({"type": kind, "request": request, "ok": true, "data": data}),
        Err(error) => json!({"type": kind, "request": request, "ok": false, "error": error}),
    })
}

fn agent_auto(role: &Role) -> bool {
    matches!(role, Role::Terminal(_))
}

fn agent_step(app: &App, role: &Role, what: &str) {
    if let Role::Terminal(me) = role {
        crate::modes::note(app, "task", &format!("{what}: done for the agent's request"), Some(me.as_str()), Some("Nixa lets the agent do it"));
    }
}

fn user_only(role: &Role, what: &str) -> Result<(), String> {
    match role {
        Role::User | Role::Brain => Ok(()),
        _ => Err(format!("Only the user or Nixa can {what}. Nothing was changed. Ask Nixa to do it: Nixa does it on request. Then go on with your own work.")),
    }
}

pub async fn action(app: &Arc<App>, name: &str, args: &Args, role: &Role) -> Result<Value, String> {
    match name {
        "queue" => Ok(json!({"queue": view(app), "waiting": waiting_lines(app)})),
        "next" => next(app, args, role),
        "spaces" => spaces(app),
        "space" => space(app, args, role),
        window if super::window::ACTIONS.contains(&window) => {
            if agent_auto(role) {
                if !["options", "list", "show"].contains(&window) {
                    agent_step(app, role, &format!("tasks window {window}"));
                }
            } else {
                user_only(role, "change tasks from the Tasks window")?;
            }
            super::window::action(app, window, args).await
        }
        "start" => {
            let id = id_of(args)?;
            if let Role::Terminal(me) = role {
                crate::modes::note(app, "task", &format!("task #{id}: started for the agent's request"), Some(me), Some("Nixa starts it"));
            }
            let task = start_now(app, id, *role == Role::User).await?;
            Ok(json!({"task": task, "queued": task.terminal_id.is_empty()}))
        }
        "continue" => {
            let id = id_of(args)?;
            if !agent_auto(role) {
                user_only(role, "continue or hand over a task")?;
            }
            let assign = match text(args, "assign").or_else(|| text(args, "who")) {
                Some(who) => {
                    let conn = db(app)?;
                    Some(super::window::assign_of(&conn, &who)?).filter(|(_, w)| !w.is_empty())
                }
                None => None,
            };
            let done = continue_task(app, id, assign).await?;
            Ok(json!({"task": done.task, "how": done.how, "terminal": done.terminal}))
        }
        "checkpoint" => {
            let id = id_of(args)?;
            let task = tasks::get_task(&db(app)?, id)
                .map_err(fail)?
                .ok_or_else(|| format!("No task #{id}."))?;
            if let Role::Terminal(me) = role
                && task.terminal_id != *me
            {
                return Err(format!("Task #{id} is not held by terminal {me}."));
            }
            if task.terminal_id.is_empty() {
                return Err(format!(
                    "Task #{id} has no claim, so there is no work to save."
                ));
            }
            let why = text(args, "why").unwrap_or_else(|| "asked for a checkpoint".into());
            let saved = checkpoint(app, &task, &super::prompt::cut(&why, 120), Seen::Read).await;
            changed(app, "checkpoint", Some(id));
            Ok(json!({"task": saved}))
        }
        "plan" => super::plan::act(app, args, role).await,
        "resume" => {
            if agent_auto(role) {
                agent_step(app, role, "resume the todo tasks");
            } else {
                user_only(role, "resume the todo tasks")?;
            }
            let (started, skipped) = resume_todos(app, text(args, "space").as_deref()).await;
            Ok(json!({"started": started, "skipped": skipped}))
        }
        "changed" => {
            let id = id_of(args)?;
            let why = text(args, "why").ok_or("why is needed, for example status:review.")?;
            let store = text(args, "db").filter(|store| !store.trim().is_empty());
            if let Some(store) = &store
                && !same_file(store, &app.db_file)
            {
                return Err("This change is for the task store of another Nixa data folder.".into());
            }
            let task = tasks::get_task(&db(app)?, id).map_err(fail)?.ok_or_else(|| format!("No task #{id}."))?;
            let who = match role {
                Role::User => text(args, "role").unwrap_or_else(|| "user".into()),
                Role::Terminal(terminal) if task.terminal_id != *terminal || store.is_none() => {
                    return Err(format!("Terminal {terminal} does not hold task #{id} in this data folder."));
                }
                other => other.word().to_string(),
            };
            if matches!(role, Role::User) && !["user", "nixa"].contains(&who.as_str()) {
                return Err("role is user or nixa. An agent reports from its own terminal.".into());
            }
            if why.strip_prefix("status:").is_some_and(|state| state != task.status.as_str()) {
                return Ok(json!({"id": id, "raised": false, "status": task.status.as_str()}));
            }
            let raised = status_event(app, id, &why, &who).await;
            Ok(json!({"id": id, "raised": raised}))
        }
        _ => Err(format!(
            "The tasks message does not know the action {name}."
        )),
    }
}

fn same_file(given: &str, own: &std::path::Path) -> bool {
    let real = |path: &std::path::Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    real(std::path::Path::new(given)) == real(own)
}

fn space_key(app: &App, key: &str) -> Result<crate::spaces::Space, String> {
    let conn = db(app)?;
    let pl = places(&conn);
    crate::spaces::find(&pl.spaces, key)
        .cloned()
        .ok_or_else(|| format!("No space named {key}."))
}

fn next(app: &App, args: &Args, role: &Role) -> Result<Value, String> {
    let conn = db(app)?;
    let pl = places(&conn);
    let key = match (text(args, "space"), role) {
        (Some(key), _) => key,
        (None, Role::Terminal(terminal)) => app
            .space_of_terminal(terminal)
            .map(|s| s.id)
            .ok_or("The space of this terminal is not known. Pass space.")?,
        _ => return Err("space is needed.".into()),
    };
    let space =
        crate::spaces::find(&pl.spaces, &key).ok_or_else(|| format!("No space named {key}."))?;
    if space_locked(space, &pl.rows) {
        return Err(locked_text(&space.name));
    }
    let task = tasks::next_task(&conn, &space.id)
        .map_err(fail)?
        .filter(|t| task_locked(t, &pl.rows).is_none());
    Ok(json!({"task": task, "space": space.id}))
}

fn spaces(app: &App) -> Result<Value, String> {
    let conn = db(app)?;
    let pl = places(&conn);
    let list: Vec<Value> = pl
        .spaces
        .iter()
        .map(|space| {
            let row = pl.row(&space.id);
            let doing = tasks::doing_count(&conn, &space.id).unwrap_or(0);
            json!({
                "id": space.id,
                "name": space.name,
                "folder": space.folder,
                "autoStart": row.is_some_and(|r| r.auto_start),
                "autoSpace": super::lock::auto_space(space),
                "maxDoing": pl.max_doing(&space.id),
                "locked": row.is_some_and(|r| r.locked),
                "doing": doing,
            })
        })
        .collect();
    Ok(json!({"spaces": list}))
}

fn space(app: &Arc<App>, args: &Args, role: &Role) -> Result<Value, String> {
    let key = text(args, "space").ok_or("space is needed.")?;
    let space = space_key(app, &key)?;
    let mut patch = SpacePatch {
        auto_start: flag(args, "autoStart").or_else(|| flag(args, "auto_start")),
        ..SpacePatch::default()
    };
    if let Some(max) = number(args, "maxDoing").or_else(|| number(args, "max_doing")) {
        if max == 0 || max as i64 > MAX_DOING_TOP {
            return Err(format!("maxDoing is 1 to {MAX_DOING_TOP}."));
        }
        patch.max_doing = Some(max as i64);
    }
    if let Some(locked) = flag(args, "locked") {
        user_only(role, "lock or unlock a space")?;
        patch.locked = Some(locked);
    }
    if patch == SpacePatch::default() {
        return Err("Nothing to change. Pass autoStart, maxDoing or locked.".into());
    }
    let conn = db(app)?;
    let row = nixa_store::spaces::set_space(&conn, &space.id, &patch).map_err(fail)?;
    if let Some(on) = patch.auto_start {
        super::start::mark_auto_start(&conn, &space.id, on);
    }
    drop(conn);
    let mut said = Vec::new();
    if let Some(on) = patch.auto_start {
        said.push(
            if on {
                "auto start on"
            } else {
                "auto start off"
            }
            .to_string(),
        );
    }
    if let Some(max) = patch.max_doing {
        said.push(format!("at most {max} tasks at once"));
    }
    if let Some(locked) = patch.locked {
        said.push(if locked { "locked" } else { "unlocked" }.to_string());
    }
    let line = format!("Space {}: {}.", space.name, said.join(", "));
    app.info(&line);
    changed(app, "space", None);
    if patch.auto_start == Some(true) || patch.locked == Some(false) || patch.max_doing.is_some() {
        super::tick_soon(app);
    }
    Ok(json!({"space": row, "text": line}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_store::tasks::{Actor, AuthorKind, BlockedOn, CommentInput, CommentKind, TaskInput, TaskStatus};
    use std::sync::Mutex;

    const HOLDER: &str = "no-such-terminal!";
    static REVIEWS: Mutex<Vec<(std::path::PathBuf, i64)>> = Mutex::new(Vec::new());

    fn record(app: Arc<App>, id: i64, _why: String) -> futures_util::future::BoxFuture<'static, ()> {
        Box::pin(async move { REVIEWS.lock().unwrap().push((app.dir.clone(), id)) })
    }

    fn app() -> (tempfile::TempDir, Arc<App>) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        (dir, app)
    }

    fn held_task(app: &App, workflow: &str) -> i64 {
        let conn = db(app).unwrap();
        let input = TaskInput { title: "Fix the parser".into(), space_id: "nixa".into(), workflow: workflow.into(), ..TaskInput::default() };
        let id = tasks::add_task(&conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap().id;
        hold(&conn, id);
        id
    }

    fn hold(conn: &nixa_store::Connection, id: i64) {
        let claim = tasks::claim_task(conn, id, &Actor::in_terminal(AuthorKind::Agent, "agent", HOLDER), "").unwrap();
        assert!(claim.ok, "{}", claim.reason);
    }

    fn told(app: &App, id: i64, why: &str, terminal: bool) -> Args {
        let mut body = json!({"id": id, "why": why, "db": app.db_file});
        if terminal {
            body["terminal"] = json!(HOLDER);
        }
        args_of(&body)
    }

    fn blocked_on_user(app: &App, id: i64, question: &str) {
        let conn = db(app).unwrap();
        let by = Actor::in_terminal(AuthorKind::Agent, "agent", HOLDER);
        tasks::add_comment(&conn, id, &CommentInput::new(CommentKind::Question, question), &by).unwrap();
        tasks::set_status(&conn, id, TaskStatus::Blocked, Some(BlockedOn::User), true).unwrap();
    }

    #[tokio::test]
    async fn an_agent_review_from_the_cli_settles_once() {
        super::super::set_review(record);
        let (_dir, app) = app();
        let id = held_task(&app, "");
        tasks::set_status(&db(&app).unwrap(), id, TaskStatus::Review, None, false).unwrap();
        let agent = Role::Terminal(HOLDER.into());
        let first = action(&app, "changed", &told(&app, id, "status:review", true), &agent).await.unwrap();
        assert_eq!(first["raised"], true);
        let again = action(&app, "changed", &told(&app, id, "status:review", true), &agent).await.unwrap();
        assert_eq!(again["raised"], false);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let seen = REVIEWS.lock().unwrap().iter().filter(|(dir, task)| *dir == app.dir && *task == id).count();
        assert_eq!(seen, 1);

        let other = action(&app, "changed", &told(&app, id, "status:review", false), &Role::Terminal("m9".into())).await.unwrap_err();
        assert!(other.contains("does not hold"), "{other}");
        let faked = action(&app, "changed", &args_of(&json!({"id": id, "why": "status:review", "role": "agent"})), &Role::User).await.unwrap_err();
        assert!(faked.contains("reports from its own terminal"), "{faked}");
        let mut elsewhere = told(&app, id, "status:review", true);
        elsewhere.insert("db".into(), json!("/tmp/another-nixa/nixa.db"));
        let elsewhere = action(&app, "changed", &elsewhere, &agent).await.unwrap_err();
        assert!(elsewhere.contains("another Nixa data folder"), "{elsewhere}");
        let stale = action(&app, "changed", &told(&app, id, "status:blocked", true), &agent).await.unwrap();
        assert_eq!(stale["raised"], false);
        assert_eq!(stale["status"], "review");
    }

    #[tokio::test]
    async fn a_cli_block_on_the_user_does_not_wait_for_the_user() {
        let (_dir, app) = app();
        let id = held_task(&app, "");
        blocked_on_user(&app, id, "Which file is the real one?");
        let reply = action(&app, "changed", &told(&app, id, "status:blocked", false), &Role::User).await.unwrap();
        assert_eq!(reply["raised"], true);
        let chat = crate::app::lock(&app.chat).list().iter().map(|item| item.text.clone()).collect::<Vec<_>>();
        let line = format!("Task #{id}");
        assert!(!chat.iter().any(|text| text.starts_with(&line) && text.contains("waits for you")), "{chat:?}");
    }

    #[tokio::test]
    async fn an_agent_resumes_todo_tasks() {
        let (_dir, app) = app();
        let agent = Role::Terminal(HOLDER.into());
        let body = args_of(&json!({"space": "nixa"}));
        let yes = action(&app, "resume", &body, &agent).await.unwrap();
        assert_eq!(yes["started"], json!([]));
    }
}
