use crate::agent;
use crate::app::{App, lock};
use crate::brains::{BrainInput, check_input, choice_of, id_of_choice};
use crate::deep::{AskInput, clean_reply};
use crate::util::now_ms;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::Extension;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

type Shared = State<Arc<App>>;
pub(crate) type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

fn ok(value: Value) -> Response {
    let mut body = json!({"ok": true});
    if let (Some(out), Value::Object(more)) = (body.as_object_mut(), value) {
        out.extend(more);
    }
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn bad(code: StatusCode, message: impl Into<String>) -> Response {
    (code, axum::Json(json!({"ok": false, "error": message.into()}))).into_response()
}

fn word(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/chat", get(chat_get).post(chat_post))
        .route("/chat/interrupt", post(chat_interrupt))
        .route("/chat/clean", post(chat_clean))
        .route("/context", get(context_get))
        .route("/compact", post(compact_post))
        .route("/brains", get(brains_get).post(brains_post))
        .route("/brains/{id}", delete(brains_delete))
        .route("/brain/choice", post(choice_post))
        .route("/brain/models", get(models_get))
        .route("/brain/model", post(model_post))
        .route("/decision", get(decision_get).post(decision_post))
        .route("/decision/check", post(decision_check))
        .route("/memory", get(memory_get).post(memory_post))
        .route("/memory/{id}", delete(memory_delete))
        .route("/deep", get(deep_get))
        .route("/deep/join", post(deep_join))
        .route("/deep/leave", post(deep_leave))
        .route("/deep/start", post(deep_start))
        .route("/deep/ask", post(deep_ask))
        .route("/deep/digest", post(deep_digest))
        .route("/deep/{id}/reply", post(deep_reply))
        .route("/trees", get(trees_get))
        .route("/trees/{name}", get(tree_get).put(tree_put).delete(tree_delete))
        .route("/trees/{name}/undo", post(tree_undo))
        .route("/watch/dry", post(watch_dry))
        .route("/settings", get(settings_get).post(settings_post))
        .route("/ws", get(ws))
        .merge(crate::cli_brains::door::routes())
        .merge(crate::answer_rules::routes())
        .merge(crate::prompts::routes())
        .merge(crate::voice::routes())
        .merge(crate::voice_map::routes())
        .merge(crate::announce::routes())
        .merge(crate::universe::routes())
        .merge(crate::soap::routes())
        .merge(crate::settings_api::routes())
        .merge(crate::custom_agents::routes())
        .merge(crate::schedules::routes())
        .merge(crate::talk::routes())
        .merge(crate::modes::routes())
        .merge(crate::queue::routes())
        .merge(crate::review_flow::routes())
        .merge(crate::models::routes())
        .merge(crate::workflows::routes())
        .merge(crate::teams::routes())
        .merge(crate::privacy::routes())
        .merge(crate::decide::routes::routes())
        .merge(crate::ollama::routes())
        .merge(crate::desktop::routes())
        .merge(crate::phone_door::routes())
        .merge(crate::extras::routes())
        .merge(crate::browser::routes())
        .merge(crate::assistant::routes())
        .merge(crate::web::routes())
        .merge(crate::knowledge::routes())
        .merge(crate::connectors::routes())
        .merge(crate::drones::routes::routes())
        .merge(crate::leave::routes::routes())
        .merge(crate::tree_api::routes())
        .merge(crate::actions::routes::routes())
        .layer(axum::middleware::from_fn_with_state(app.clone(), crate::gate::check))
        .with_state(app)
}

async fn health(State(app): Shared) -> Response {
    let engine = crate::engine::ping().await.is_ok();
    let brain = agent::brain_info(&app);
    let mut body = json!({"name": app.name(), "version": env!("CARGO_PKG_VERSION"), "busy": agent::busy(&app), "engine": engine, "brain": brain});
    if let Ok(nonce) = std::env::var(nixa_protocol::paths::ENV_RELEASE_NONCE)
        && nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        body["releaseNonce"] = json!(nonce);
        body["releasePid"] = json!(std::process::id());
    }
    ok(body)
}

async fn chat_get(State(app): Shared) -> Response {
    let items = lock(&app.chat).list().to_vec();
    ok(json!({"items": items, "busy": agent::busy(&app)}))
}

async fn chat_post(State(app): Shared, headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    let Some(text) = word(&body, "text") else { return bad(StatusCode::BAD_REQUEST, "text is needed") };
    let typed = body.get("typed").and_then(Value::as_bool).unwrap_or(true);
    let now = now_ms();
    let said = if typed { crate::talk::Said::typed(&text, now) } else { crate::talk::Said::heard(&text, now, now) };
    let said = crate::talk::Said { mac: crate::window_key::allowed(&app, &headers), ..said };
    crate::talk::hear_later(&app, said);
    ok(json!({}))
}

async fn chat_interrupt(State(app): Shared) -> Response {
    ok(json!({"stopped": interrupt_turn(&app)}))
}

pub(crate) fn interrupt_turn(app: &Arc<App>) -> bool {
    let stopped = agent::interrupt(app);
    crate::talk::hold::answered(app);
    stopped
}

async fn chat_clean(State(app): Shared) -> Response {
    agent::reset(&app);
    lock(&app.chat).clear();
    lock(&app.folds).clear();
    if let Err(error) = crate::knowledge::forget_chat(&app) {
        crate::util::log("knowledge", &error);
    }
    lock(&app.brain).carry.clear();
    if let Ok(conn) = nixa_store::open(&app.db_file) {
        nixa_store::db::wipe_free(&conn);
    }
    app.send_out(json!({"type": "chat_cleaned"}));
    ok(json!({}))
}

async fn context_get(State(app): Shared) -> Response {
    let view = lock(&app.folds).view(agent::context(&app));
    ok(json!({"text": crate::compact::view_text(&app), "view": view}))
}

async fn compact_post(State(app): Shared) -> Response {
    let outcome = crate::compact::compact(&app, "asked");
    ok(json!({"outcome": format!("{outcome:?}").to_lowercase()}))
}

async fn brains_get(State(app): Shared) -> Response {
    let mut list = Vec::new();
    for brain in app.brains.list() {
        let env = crate::keys::env_key(&brain);
        let source = if env.is_some() { "env" } else if crate::keys::keychain_key(&brain.id).await.is_some() { "keychain" } else { "none" };
        list.push(json!({
            "id": brain.id, "choice": choice_of(&brain.id), "name": brain.name, "kind": brain.kind, "kindTitle": brain.kind.title(),
            "baseUrl": brain.base_url, "model": brain.model, "effort": brain.effort, "keyEnv": brain.key_env,
            "keySource": source, "envNames": crate::keys::env_names(&brain),
        }));
    }
    ok(json!({"brains": list, "cli": agent::cli_list(&app), "choice": app.settings().choice}))
}

async fn brains_post(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let id = word(&body, "id");
    let input: BrainInput = match serde_json::from_value(body.clone()) {
        Ok(input) => input,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let saved = app.brains.list();
    let before = id.as_deref().and_then(|i| saved.iter().find(|b| b.id == i)).cloned();
    let has_key = match before.as_ref() {
        Some(before) => crate::keys::env_key(before).is_some() || crate::keys::keychain_key(&before.id).await.is_some(),
        None => !input.key_env.trim().is_empty() && std::env::var(input.key_env.trim()).is_ok_and(|v| !v.trim().is_empty()),
    };
    let checked = match check_input(id.as_deref(), &input, &saved, has_key) {
        Ok(checked) => checked,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let key = input.api_key.trim();
    if !key.is_empty() && !crate::keys::key_is_clean(key) {
        return bad(StatusCode::BAD_REQUEST, "The key has a space or a quote in it. Paste it again.");
    }
    let moved = before.as_ref().is_some_and(|b| crate::brains::server_moved(b, &checked));
    if let Err(error) = check_server(&app, id.as_deref(), &checked, key, !moved).await {
        return bad(StatusCode::BAD_REQUEST, error);
    }
    let is_new = id.is_none();
    let brain = match id {
        Some(id) => match app.brains.update(&id, |b| {
            b.name = checked.name.clone();
            b.kind = checked.kind;
            b.base_url = checked.base_url.clone();
            b.key_env = checked.key_env.clone();
            b.model = checked.model.clone();
        }) {
            Ok(Some(brain)) => brain,
            Ok(None) => return bad(StatusCode::NOT_FOUND, "This brain is not saved any more."),
            Err(error) => return bad(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        },
        None => match app.brains.add(checked) {
            Ok(brain) => brain,
            Err(error) => return bad(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        },
    };
    if !key.is_empty()
        && let Err(error) = crate::keys::save_keychain_key(&brain.id, key).await
    {
        if is_new {
            let _ = app.brains.remove(&brain.id);
        } else if moved {
            crate::keys::delete_keychain_key(&brain.id).await;
        }
        return bad(StatusCode::BAD_REQUEST, format!("The key was not saved in the keychain: {error}"));
    }
    if moved && key.is_empty() {
        crate::keys::delete_keychain_key(&brain.id).await;
    }
    ok(json!({"id": brain.id, "choice": choice_of(&brain.id)}))
}

async fn check_server(app: &App, id: Option<&str>, checked: &crate::brains::Checked, typed: &str, saved_key: bool) -> Result<(), String> {
    let probe = crate::brains::SavedBrain {
        id: id.unwrap_or_default().to_string(),
        name: checked.name.clone(),
        kind: checked.kind,
        base_url: checked.base_url.clone(),
        model: String::new(),
        effort: None,
        key_env: checked.key_env.clone(),
        added_at: 0,
    };
    let api_key = if !typed.is_empty() {
        typed.to_string()
    } else if id.is_some() && saved_key {
        crate::keys::api_key(&probe).await.unwrap_or_default()
    } else {
        crate::keys::env_key(&probe).unwrap_or_default()
    };
    let endpoint = crate::provider::Endpoint { kind: checked.kind, base_url: checked.base_url.clone(), api_key };
    crate::provider::models(&app.http, &endpoint)
        .await
        .map(|_| ())
        .map_err(|error| format!("{error} Nothing was saved."))
}

async fn brains_delete(State(app): Shared, Path(id): Path<String>) -> Response {
    match app.brains.remove(&id) {
        Ok(true) => {
            crate::keys::delete_keychain_key(&id).await;
            if app.settings().choice.as_deref().and_then(id_of_choice) == Some(id.as_str()) {
                let _ = app.save_settings(|s| s.choice = None);
                agent::reset(&app);
                send_brain(&app);
            }
            ok(json!({}))
        }
        Ok(false) => bad(StatusCode::NOT_FOUND, "No such brain."),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn choice_post(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let Some(choice) = word(&body, "choice") else { return bad(StatusCode::BAD_REQUEST, "choice is needed") };
    match agent::pick(&app, &choice) {
        Ok(_) => {
            send_brain(&app);
            ok(json!({"choice": choice}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn models_get(State(app): Shared) -> Response {
    match agent::load_models(&app).await {
        Ok(models) => ok(json!({"models": models})),
        Err(error) => bad(StatusCode::BAD_GATEWAY, error),
    }
}

async fn model_post(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let effort = body.get("effort").map(|_| word(&body, "effort"));
    match agent::set_model(&app, word(&body, "model"), effort) {
        Ok(value) => {
            send_brain(&app);
            ok(value)
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn decision_get(State(app): Shared) -> Response {
    let full = app.decision.full();
    ok(json!({"full": full.word(), "name": app.decision.name(), "text": crate::decision::full_meaning(full, &app.name())}))
}

pub const CHECK_FLAGS: [&str; 5] = ["--sample", "--kind", "--capability", "--held", "--bench"];

async fn decision_check(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let args: Vec<String> = body.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if let Some(bad_flag) = args.iter().find(|a| a.starts_with("--") && !CHECK_FLAGS.contains(&a.as_str())) {
        return bad(StatusCode::BAD_REQUEST, format!("{bad_flag} is not a check flag here."));
    }
    let dir = app.dir.clone();
    match tokio::task::spawn_blocking(move || crate::fast_trees::report(&args, &dir)).await {
        Ok(Ok(report)) => ok(json!({"report": report})),
        Ok(Err(error)) => bad(StatusCode::BAD_REQUEST, error),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn decision_post(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let Some(word) = word(&body, "full").or_else(|| word(&body, "who")) else {
        return bad(StatusCode::BAD_REQUEST, "full is deep or nixa");
    };
    let full = match app.decision.set_choice(&word) {
        Ok(full) => full,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let line = format!("the user set who decides: {}. {}", crate::decision::full_name(full), crate::decision::full_meaning(full, &app.name()));
    app.info(&line);
    let note = format!("Note: {line}");
    {
        let mut brain = lock(&app.brain);
        brain.carry = if brain.carry.is_empty() { note } else { format!("{}\n\n{note}", brain.carry) };
    }
    app.send_out(json!({"type": "decision", "full": full.word()}));
    ok(json!({"full": full.word(), "name": app.decision.name()}))
}

pub fn send_brain(app: &App) {
    app.send_out(json!({"type": "brain", "brain": agent::brain_info(app)}));
}

async fn memory_get(State(app): Shared) -> Response {
    match app.memory.all() {
        Ok(lines) => ok(json!({"lines": lines, "prompt": app.memory.prompt()})),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn memory_post(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let Some(text) = word(&body, "text") else { return bad(StatusCode::BAD_REQUEST, "text is needed") };
    let message = app.memory.add(&text);
    if message.starts_with("Could not") {
        return bad(StatusCode::INTERNAL_SERVER_ERROR, message);
    }
    ok(json!({"message": message}))
}

async fn memory_delete(State(app): Shared, Path(id): Path<i64>) -> Response {
    match app.memory.remove_line(id) {
        Ok(true) => {
            if let Err(error) = crate::knowledge::forget_memory(&app) {
                crate::util::log("knowledge", &error);
            }
            ok(json!({}))
        }
        Ok(false) => bad(StatusCode::NOT_FOUND, "No such memory note."),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(crate) async fn caller(body: &Value, peer: Peer) -> Result<String, (StatusCode, String)> {
    let Some(terminal) = word(body, "terminal") else { return Err((StatusCode::BAD_REQUEST, "terminal is needed: run it from inside a Nixa terminal (NIXA_TERM_ID)".into())) };
    let id = crate::engine::term_id(&terminal).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    match crate::engine::get(&id).await {
        Ok(term) if term.running => {
            let name = term.id.to_string();
            crate::extras::peer::from_terminal(peer.map(|Extension(ConnectInfo(a))| a), &name, term.pid).await.map_err(|e| (StatusCode::FORBIDDEN, e))?;
            Ok(name)
        }
        Ok(_) => Err((StatusCode::FORBIDDEN, format!("Terminal {terminal} does not run."))),
        Err(_) => Err((StatusCode::FORBIDDEN, format!("Terminal {terminal} is not a Nixa terminal."))),
    }
}

async fn deep_get(State(app): Shared) -> Response {
    let view = lock(&app.deep).view(None, now_ms());
    let text = lock(&app.deep).view_text(now_ms());
    ok(json!({"view": view, "text": text}))
}

// A terminal identity must always be proved, even when a window token is sent.
// Local user commands outside terminals use the private window token instead.
async fn deep_access(app: &App, headers: &HeaderMap, body: &Value, peer: Peer) -> Result<Option<String>, (StatusCode, String)> {
    if word(body, "terminal").is_some() {
        return caller(body, peer).await.map(Some);
    }
    if crate::window_key::allowed(app, headers) {
        Ok(None)
    } else {
        Err((StatusCode::UNAUTHORIZED, "A private Nixa window token or verified terminal is required.".into()))
    }
}

async fn deep_space(app: &App, body: &Value, terminal: Option<&str>) -> Result<crate::spaces::Space, String> {
    let named = word(body, "space");
    if named.is_some() || terminal.is_none() {
        return crate::deep_service::resolve(app, named.as_deref(), terminal);
    }
    let terminal = terminal.unwrap();
    // A newly created terminal may not have reached the watch cache yet.
    let info = crate::engine::get(&crate::engine::term_id(terminal).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
    let spaces = app.spaces();
    let found = match info.labels.get(crate::deep::SPACE_LABEL) {
        Some(id) => crate::spaces::find(&spaces, id),
        None => crate::spaces::for_folder(&spaces, &info.cwd),
    }.ok_or_else(|| format!("No space is known for terminal {terminal}. Give its space explicitly."))?;
    crate::deep_service::ensure_open(app, found)
}

async fn deep_digest(State(app): Shared, headers: HeaderMap, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let terminal = match deep_access(&app, &headers, &body, peer).await {
        Ok(terminal) => terminal,
        Err((code, error)) => return bad(code, error),
    };
    let space = match deep_space(&app, &body, terminal.as_deref()).await {
        Ok(space) => space,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    match crate::deep_digest::digest(&app, &space) {
        Ok(digest) => ok(digest),
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn deep_join(State(app): Shared, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let terminal = match caller(&body, peer).await {
        Ok(t) => t,
        Err((code, message)) => return bad(code, message),
    };
    let space = match deep_space(&app, &body, Some(&terminal)).await {
        Ok(space) => space,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let origin = match deep_space(&app, &json!({}), Some(&terminal)).await {
        Ok(space) => space,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    if origin.id != space.id {
        return bad(StatusCode::FORBIDDEN, "A deep brain must join its own terminal's space, not another space.");
    }
    let name = word(&body, "name").unwrap_or_default();
    let result = lock(&app.deep).join(&terminal, &name, &space, word(&body, "as").as_deref(), now_ms());
    match result {
        Ok((seat, actions)) => {
            crate::watch::run_actions(&app, actions).await;
            ok(json!({"seat": seat}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn deep_leave(State(app): Shared, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let terminal = match caller(&body, peer).await {
        Ok(t) => t,
        Err((code, message)) => return bad(code, message),
    };
    let result = lock(&app.deep).leave(&terminal);
    match result {
        Ok(seat) => ok(json!({"seat": seat})),
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn deep_start(State(app): Shared, headers: HeaderMap, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let terminal = match deep_access(&app, &headers, &body, peer).await {
        Ok(terminal) => terminal,
        Err((code, error)) => return bad(code, error),
    };
    let space = match deep_space(&app, &body, terminal.as_deref()).await {
        Ok(space) => space,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let kind = word(&body, "brain").unwrap_or_else(|| "claude".into());
    if body.get("ensure").and_then(Value::as_bool).unwrap_or(false) {
        return match crate::deep_service::ensure(&app, &kind, &space) {
            Ok(crate::deep_service::Ensured::Already(text)) => {
                crate::watch::tell_event(&app, &text);
                ok(json!({"space": space.id, "brain": kind, "started": false}))
            }
            Ok(crate::deep_service::Ensured::Started(actions)) => {
                crate::watch::run_actions(&app, actions).await;
                ok(json!({"space": space.id, "brain": kind, "started": true}))
            }
            Err(error) => bad(StatusCode::BAD_REQUEST, error),
        };
    }
    let result = crate::deep_service::start(&app, &kind, &space);
    match result {
        Ok(actions) => {
            crate::watch::run_actions(&app, actions).await;
            ok(json!({"space": space.id, "brain": kind}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn deep_ask(State(app): Shared, headers: HeaderMap, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let terminal = match deep_access(&app, &headers, &body, peer).await {
        Ok(terminal) => terminal,
        Err((code, error)) => return bad(code, error),
    };
    let space = match deep_space(&app, &body, terminal.as_deref()).await {
        Ok(space) => space,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let input = AskInput {
        question: word(&body, "question").unwrap_or_default(),
        options: body.get("options").and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        notes: word(&body, "notes").unwrap_or_default(),
        kind: word(&body, "kind"),
        wait: body.get("wait").and_then(Value::as_u64),
        brain: word(&body, "brain"),
    };
    let result = crate::deep_service::ask(&app, input, &space);
    match result {
        Ok((ask, free)) => {
            crate::watch::deep_tick(&app).await;
            ok(json!({"ask": ask, "free": free}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn deep_reply(State(app): Shared, Path(id): Path<String>, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let terminal = match caller(&body, peer).await {
        Ok(t) => t,
        Err((code, message)) => return bad(code, message),
    };
    let space_id = lock(&app.deep).by_terminal(&terminal).map(|seat| seat.space.clone());
    if let Err(error) = space_id.ok_or_else(|| "Only a joined deep brain can answer.".to_string())
        .and_then(|space| crate::deep_service::resolve(&app, Some(&space), None)) {
        return bad(StatusCode::FORBIDDEN, error);
    }
    let reply = match clean_reply(&body) {
        Ok(reply) => reply,
        Err(error) => return bad(StatusCode::BAD_REQUEST, error),
    };
    let result = lock(&app.deep).reply(&terminal, &id, reply, now_ms());
    match result {
        Ok((ask, actions)) => {
            crate::watch::run_actions(&app, actions).await;
            ok(json!({"ask": ask}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn trees_get(State(app): Shared) -> Response {
    let list = lock(&app.trees).list();
    ok(json!({"trees": list}))
}

async fn tree_get(State(app): Shared, headers: axum::http::HeaderMap, Path(name): Path<String>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let result = lock(&app.trees).read(&name);
    match result {
        Ok(tree) => ok(json!({"tree": tree})),
        Err(error) => bad(StatusCode::NOT_FOUND, error),
    }
}

pub fn tree_note(carry: &str, name: &str) -> String {
    let note = format!("Note: the user changed rule tree {name}. Read it again before you use it.");
    if carry.split("\n\n").any(|line| line == note) {
        carry.to_string()
    } else if carry.is_empty() {
        note
    } else {
        format!("{carry}\n\n{note}")
    }
}

async fn tree_put(State(app): Shared, headers: axum::http::HeaderMap, Path(name): Path<String>, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let raw = body.get("tree").cloned().unwrap_or(body);
    let result = lock(&app.trees).put(&name, &raw, "user");
    match result {
        Ok(tree) => {
            crate::tree_api::changed(&app, &name, "by the user");
            ok(json!({"tree": tree}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn tree_delete(State(app): Shared, headers: axum::http::HeaderMap, Path(name): Path<String>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let result = lock(&app.trees).remove_tree(&name, "user");
    match result {
        Ok(()) => {
            crate::tree_api::changed(&app, &name, "by the user");
            ok(json!({}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn tree_undo(State(app): Shared, headers: axum::http::HeaderMap, Path(name): Path<String>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let result = lock(&app.trees).undo(&name, "user");
    match result {
        Ok(tree) => {
            crate::tree_api::changed(&app, &name, "by the user");
            ok(json!({"tree": tree}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn watch_dry(State(app): Shared, body: Option<axum::Json<Value>>) -> Response {
    let raw = body.and_then(|b| b.0.get("tree").cloned());
    let tree = match raw {
        Some(value) => crate::trees::check_tree(&value, Some(crate::trees::WATCH_TREE)),
        None => lock(&app.trees).read(crate::trees::WATCH_TREE),
    };
    match tree {
        Ok(tree) => {
            let subjects = crate::watch::subjects(&app);
            let mut body = crate::walk::walks_json(&tree, &subjects);
            body["text"] = json!(crate::walk::dry_run(&tree, &subjects));
            ok(body)
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn settings_get(State(app): Shared) -> Response {
    ok(json!({"settings": app.settings(), "watchMinutes": app.watch_minutes(), "name": app.name(), "dataDir": app.dir.display().to_string()}))
}

async fn settings_post(State(app): Shared, axum::Json(body): axum::Json<Value>) -> Response {
    let minutes = body.get("watchMinutes").and_then(Value::as_u64);
    if minutes.is_some_and(|m| m > 240) {
        return bad(StatusCode::BAD_REQUEST, "watchMinutes is 0 to 240");
    }
    let result = app.save_settings(|s| {
        if let Some(m) = minutes {
            s.watch_minutes = Some(m);
        }
        if let Some(name) = word(&body, "assistantName") {
            s.assistant_name = Some(name.chars().take(40).collect());
        }
        if let Some(on) = body.get("deepAutoStart").and_then(Value::as_bool) {
            s.deep_auto_start = Some(on);
        }
    });
    match result {
        Ok(settings) => ok(json!({"settings": settings})),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn ws(State(app): Shared, headers: axum::http::HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    upgrade.on_upgrade(move |socket| ws_loop(app, socket))
}

async fn ws_loop(app: Arc<App>, mut socket: WebSocket) {
    let mut rx = app.out.subscribe();
    let hello = json!({"type": "hello", "name": app.name(), "busy": agent::busy(&app), "items": lock(&app.chat).list().to_vec()});
    if socket.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    let phone = crate::phone_door::state_event(&app);
    if socket.send(Message::Text(phone.to_string().into())).await.is_err() {
        return;
    }
    let (reply_tx, mut replies) = tokio::sync::mpsc::unbounded_channel::<Value>();
    loop {
        tokio::select! {
            Some(reply) = replies.recv() => {
                if socket.send(Message::Text(reply.to_string().into())).await.is_err() {
                    break;
                }
            }
            out = rx.recv() => match out {
                Ok(value) => {
                    if socket.send(Message::Text(value.to_string().into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(value) = serde_json::from_str::<Value>(text.as_str())
                        && ws_async_kind(&value)
                    {
                        let (app, tx) = (app.clone(), reply_tx.clone());
                        tokio::spawn(async move {
                            if let Some(reply) = ws_async(&app, &value).await {
                                tx.send(reply).ok();
                            }
                        });
                        continue;
                    }
                    if let Some(reply) = on_ws(&app, text.as_str())
                        && socket.send(Message::Text(reply.to_string().into())).await.is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}

fn ws_async_kind(value: &Value) -> bool {
    crate::custom_agents::is_ws(value) || crate::queue::is_ws(value) || crate::models::is_ws(value) || crate::workflows::is_ws(value)
}

async fn ws_async(app: &Arc<App>, value: &Value) -> Option<Value> {
    if crate::custom_agents::is_ws(value) {
        return crate::custom_agents::ws_request(app, value).await;
    }
    if crate::queue::is_ws(value) {
        return crate::queue::ws_request(app, value).await;
    }
    if crate::models::is_ws(value) {
        return crate::models::ws_request(app, value).await;
    }
    crate::workflows::ws_request(app, value).await
}

fn on_ws(app: &Arc<App>, text: &str) -> Option<Value> {
    let Ok(value) = serde_json::from_str::<Value>(text) else { return None };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    if let Some(done) = crate::settings_api::ws(app, &kind, &value) {
        let request = value.get("request").and_then(Value::as_str)?;
        return Some(crate::settings_api::ws_answer(&kind, request, done));
    }
    if let Some(done) = crate::web::ws(app, &kind, &value) {
        let request = value.get("request").and_then(Value::as_str)?;
        return Some(crate::settings_api::ws_answer(&kind, request, done));
    }
    if let Some(done) = crate::connectors::ws(app, &kind, &value) {
        let request = value.get("request").and_then(Value::as_str)?;
        return Some(crate::settings_api::ws_answer(&kind, request, done));
    }
    let handled = ws_dispatch(app, &kind, &value);
    let request = value.get("request").and_then(Value::as_str)?;
    Some(ws_reply(&kind, request, handled))
}

pub fn ws_reply(kind: &str, request: &str, handled: bool) -> Value {
    let kind = if kind.is_empty() { "unknown" } else { kind };
    match handled {
        true => json!({"type": format!("{kind}_reply"), "request": request, "ok": true, "data": null}),
        false => json!({"type": format!("{kind}_reply"), "request": request, "ok": false, "error": format!("The brain does not know the message {kind}.")}),
    }
}

fn ws_dispatch(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    let voice = crate::voice::ws(app, kind, value);
    if crate::announce::ws(app, kind, value) {
        return true;
    }
    if crate::universe::ws(app, kind, value) {
        return true;
    }
    if crate::soap::ws(app, kind, value) {
        return true;
    }
    if crate::prompts::ws(app, kind, value) {
        return true;
    }
    if crate::schedules::ws(app, kind, value) {
        return true;
    }
    if crate::privacy::ws(app, kind, value) {
        return true;
    }
    if crate::modes::ws(app, kind, value) {
        return true;
    }
    if crate::browser::ws(app, kind, value) {
        return true;
    }
    if crate::phone_door::ws(app, kind, value) {
        return true;
    }
    if crate::teams::ws(app, kind, value) {
        return true;
    }
    if kind.starts_with("set_") || matches!(kind, "confirm" | "interrupt") {
        crate::talk::flow::touch_later(app);
    }
    if crate::talk::ws(app, kind, value) {
        return true;
    }
    voice || ws_chat(app, kind, value)
}

fn ws_chat(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    match kind {
        "say" | "typed" | "heard" => {
            if let Some(line) = word(value, "text") {
                let now = crate::util::now_ms();
                let said = if kind == "heard" { crate::talk::Said::heard(&line, now, now) } else { crate::talk::Said::typed(&line, now) };
                crate::talk::hear_later(app, said);
            }
            true
        }
        "interrupt" => {
            interrupt_turn(app);
            true
        }
        "task_read_stop" => {
            crate::talk::hold::stop_reads(app);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deep_routes_require_authority_and_reject_invalid_or_locked_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        for (id, locked) in [("nixa", false), ("locked", true)] {
            nixa_store::spaces::set_space(&conn, id, &nixa_store::spaces::SpacePatch { locked: Some(locked), ..Default::default() }).unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
        let http = reqwest::Client::new();
        let key = crate::window_key::HEADER;
        for path in ["ask", "start", "digest"] {
            let url = format!("{base}/deep/{path}");
            let body = json!({"space": "nixa", "question": "Which agent?"});
            assert_eq!(http.post(&url).json(&body).send().await.unwrap().status(), 401);
            assert_eq!(http.post(&url).header(key, "wrong").json(&body).send().await.unwrap().status(), 401);
            for name in ["missing", "locked"] {
                let body = json!({"space": name, "question": "Which agent?"});
                let reply = http.post(&url).header(key, &app.window_token).json(&body).send().await.unwrap();
                assert_eq!(reply.status(), 400, "{path} {name}");
                let reply: Value = reply.json().await.unwrap();
                assert!(reply["error"].as_str().unwrap().contains(name), "{reply}");
            }
            // A token never lets a client impersonate an engine terminal.
            let reply = http.post(&url).header(key, &app.window_token).json(&json!({"terminal": "not-a-terminal", "space": "nixa"})).send().await.unwrap();
            assert_eq!(reply.status(), 400);
        }
        let read: Value = http.post(format!("{base}/deep/digest")).header(key, &app.window_token).json(&json!({"space": "nixa"})).send().await.unwrap().json().await.unwrap();
        assert_eq!(read["ok"], true);
        assert_eq!(read["space"]["id"], "nixa");
        assert!(read["rules"]["hash"].is_string());
        assert!(lock(&app.deep).asks.is_empty());
        assert!(lock(&app.deep).seats.is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn saving_a_brain_checks_its_server_first() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let asked = String::from_utf8_lossy(&buf[..n]).to_string();
                let (code, body) = if asked.contains("Bearer good") { ("200 OK", r#"{"data":[{"id":"m1"}]}"#) } else { ("401 No", r#"{"error":{"message":"bad key"}}"#) };
                let reply = format!("HTTP/1.1 {code}
Content-Type: application/json
Content-Length: {}
Connection: close

{body}", body.len());
                stream.write_all(reply.as_bytes()).await.ok();
            }
        });
        let url = format!("http://127.0.0.1:{port}/v1");
        let input = BrainInput { name: "Local".into(), kind: "openai".into(), base_url: url, ..BrainInput::default() };
        let checked = check_input(None, &input, &[], false).unwrap();
        assert!(check_server(&app, None, &checked, "good", true).await.is_ok());
        let refused = check_server(&app, None, &checked, "wrong", true).await.unwrap_err();
        assert!(refused.contains("refused") && refused.ends_with("Nothing was saved."), "{refused}");
        let gone = BrainInput { name: "Gone".into(), kind: "openai".into(), base_url: "http://127.0.0.1:1".into(), ..BrainInput::default() };
        let gone = check_input(None, &gone, &[], false).unwrap();
        assert!(check_server(&app, None, &gone, "", true).await.unwrap_err().ends_with("Nothing was saved."));
    }

    #[tokio::test]
    async fn a_saved_brain_with_a_new_server_needs_the_key_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let saved = BrainInput { name: "Saved".into(), kind: "openai".into(), base_url: "https://api.example.test/v1".into(), api_key: "k".into(), ..BrainInput::default() };
        let saved = app.brains.add(check_input(None, &saved, &[], true).unwrap()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        tokio::spawn(async move {
            while listener.accept().await.is_ok() {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        });
        let addr = format!("127.0.0.1:{port}");
        let base = format!("http://{addr}/v1");
        let token = app.window_token.clone();
        let post = |body: Value| {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/brains")
                .header("host", "127.0.0.1:7421")
                .header("content-type", "application/json")
                .header(crate::window_key::HEADER, token.as_str())
                .body(axum::body::Body::from(body.to_string()))
                .unwrap();
            use tower::ServiceExt;
            router(app.clone()).oneshot(request)
        };
        let moved = post(json!({"id": saved.id, "name": "Saved", "kind": "openai", "baseUrl": base, "apiKey": ""})).await.unwrap();
        assert_eq!(moved.status(), 400);
        let new_env = post(json!({"id": saved.id, "name": "Saved", "kind": "openai", "baseUrl": "https://api.example.test/v1", "keyEnv": "OTHER_API_KEY"})).await.unwrap();
        assert_eq!(new_env.status(), 400);
        let any_env = post(json!({"name": "Other", "kind": "openai", "baseUrl": base, "keyEnv": "GITHUB_TOKEN"})).await.unwrap();
        assert_eq!(any_env.status(), 400);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert_eq!(app.brains.get(&saved.id).unwrap().base_url, "https://api.example.test/v1");
    }

    #[tokio::test]
    async fn chat_from_an_agent_terminal_is_never_the_users_yes() {
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let post = |header: &'static str, text: &str| {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/chat")
                .header("host", "127.0.0.1:7421")
                .header("content-type", "application/json")
                .header(header, if header == crate::gate::HEADER { app.gate_token.as_str() } else { app.window_token.as_str() })
                .body(axum::body::Body::from(json!({"text": text}).to_string()))
                .unwrap();
            router(app.clone()).oneshot(request)
        };
        let wait_for = |count: usize| {
            let app = app.clone();
            async move {
                for _ in 0..200 {
                    if lock(&app.chat).list().iter().filter(|i| i.who == crate::chat::Who::User).count() >= count {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                panic!("the chat line did not arrive");
            }
        };
        let held: crate::yes_once::HeldAsk = std::sync::Mutex::new(None);
        assert!(!crate::yes_once::approved_once(&app, &held, "push dev", &["push"], "push dev"));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert_eq!(post(crate::gate::HEADER, "yes push").await.unwrap().status(), 200);
        wait_for(1).await;
        assert!(lock(&app.brain).last_user.is_none(), "an agent line must not count as the user");
        assert!(!crate::yes_once::approved_once(&app, &held, "push dev", &["push"], "push dev"));
        assert_eq!(post(crate::window_key::HEADER, "yes push").await.unwrap().status(), 200);
        wait_for(2).await;
        assert!(crate::yes_once::approved_once(&app, &held, "push dev", &["push"], "push dev"));
    }

    #[test]
    fn a_tree_change_is_noted_once_without_its_text() {
        let one = tree_note("", "watch-agents");
        assert!(one.contains("watch-agents") && !one.contains('{'));
        assert_eq!(tree_note(&one, "watch-agents"), one);
        let two = tree_note(&one, "other");
        assert_eq!(two.matches("Note:").count(), 2);
        assert_eq!(tree_note(&two, "other"), two);
    }

    #[tokio::test]
    async fn chat_clean_carries_no_deleted_lines() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        lock(&app.brain).carry = "old note".into();
        chat_clean(State(app.clone())).await;
        assert!(lock(&app.brain).carry.is_empty());
    }

    #[tokio::test]
    async fn memory_delete_removes_its_knowledge_rows() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        app.memory.add("The blue door code is 4417");
        crate::knowledge::forget_memory(&app).unwrap();
        let conn = crate::knowledge::index::open(&app.knowledge.file).unwrap();
        let count = |c: &nixa_store::Connection| c.query_row("SELECT COUNT(*) FROM chunks WHERE text LIKE '%4417%'", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count(&conn), 1);
        let id = app.memory.all().unwrap()[0].id;
        memory_delete(State(app.clone()), Path(id)).await;
        assert_eq!(count(&conn), 0);
        let on: i64 = conn.query_row("PRAGMA secure_delete", [], |r| r.get(0)).unwrap();
        assert_eq!(on, 1);
        let store = nixa_store::open(&app.db_file).unwrap();
        let on: i64 = store.query_row("PRAGMA secure_delete", [], |r| r.get(0)).unwrap();
        assert_eq!(on, 1);
    }

    #[tokio::test]
    async fn tree_routes_need_the_window_token() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = router(app.clone());
        tokio::spawn(async move { axum::serve(listener, served).await.ok() });
        let http = reqwest::Client::new();
        let name = crate::trees::WATCH_TREE;
        let key = crate::window_key::HEADER;
        let token = app.window_token.clone();
        assert_eq!(http.get(format!("{base}/trees/{name}")).send().await.unwrap().status(), 401);
        assert_eq!(http.get(format!("{base}/trees/{name}")).header(key, "wrong").send().await.unwrap().status(), 401);
        let read: Value = http.get(format!("{base}/trees/{name}")).header(key, &token).send().await.unwrap().json().await.unwrap();
        let tree = read["tree"].clone();
        assert!(tree.is_object(), "{read}");
        let open = http.put(format!("{base}/trees/{name}")).json(&json!({"tree": tree})).send().await.unwrap();
        assert_eq!(open.status(), 401);
        assert!(lock(&app.brain).carry.is_empty());
        assert_eq!(http.delete(format!("{base}/trees/{name}")).send().await.unwrap().status(), 401);
        assert_eq!(http.post(format!("{base}/trees/{name}/undo")).send().await.unwrap().status(), 401);
        for _ in 0..2 {
            let saved = http.put(format!("{base}/trees/{name}")).header(key, &token).json(&json!({"tree": tree})).send().await.unwrap();
            assert_eq!(saved.status(), 200);
        }
        let carry = lock(&app.brain).carry.clone();
        assert_eq!(carry.matches("Note:").count(), 1, "{carry}");
        assert!(carry.contains("Its JSON now: {"), "{carry}");
    }

    #[tokio::test]
    async fn the_schedule_list_needs_the_window_token() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = router(app.clone());
        tokio::spawn(async move { axum::serve(listener, served).await.ok() });
        let http = reqwest::Client::new();
        assert_eq!(http.get(format!("{base}/schedules")).send().await.unwrap().status(), 401);
        let seen = http.get(format!("{base}/schedules")).header(crate::window_key::HEADER, &app.window_token).send().await.unwrap();
        assert_eq!(seen.status(), 200);
    }

    async fn ws_open_close(addr: std::net::SocketAddr, token: &str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let ask = format!("GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{}: {token}\r\n\r\n", crate::window_key::HEADER);
        stream.write_all(ask.as_bytes()).await.unwrap();
        let mut buf = [0u8; 256];
        let n = stream.read(&mut buf).await.unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 101"));
    }

    async fn read_frame(stream: &mut tokio::net::TcpStream) -> Value {
        use tokio::io::AsyncReadExt;
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).await.unwrap();
        assert_eq!(head[0], 0x81);
        let len = match head[1] & 0x7f {
            126 => stream.read_u16().await.unwrap() as usize,
            127 => stream.read_u64().await.unwrap() as usize,
            n => n as usize,
        };
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn a_new_window_gets_the_camera_state_right_after_hello() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, served).await.ok() });
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let ask = format!("GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{}: {}\r\n\r\n", crate::window_key::HEADER, app.window_token);
        stream.write_all(ask.as_bytes()).await.unwrap();
        let mut seen = Vec::new();
        while !seen.ends_with(b"\r\n\r\n") {
            seen.push(stream.read_u8().await.unwrap());
        }
        assert!(String::from_utf8_lossy(&seen).starts_with("HTTP/1.1 101"));
        let hello = tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut stream)).await.unwrap();
        assert_eq!(hello["type"], json!("hello"));
        let phone = tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut stream)).await.unwrap();
        assert_eq!(phone["type"], json!("phone"));
        assert_eq!(phone["camera"]["supported"], json!(false));
        assert!(phone["camera"]["log"].is_array());
        assert!(phone["phone"].is_object());
    }

    #[test]
    fn many_connects_and_disconnects_keep_the_thread_count_flat() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let live = Arc::new(AtomicUsize::new(0));
        let (up, down) = (live.clone(), live.clone());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_keep_alive(std::time::Duration::from_millis(100))
            .enable_all()
            .on_thread_start(move || {
                up.fetch_add(1, Ordering::SeqCst);
            })
            .on_thread_stop(move || {
                down.fetch_sub(1, Ordering::SeqCst);
            })
            .build()
            .unwrap();
        runtime.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let app = App::open(dir.path()).unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let served = router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
            tokio::spawn(async move { axum::serve(listener, served).await.ok() });
            let http = reqwest::Client::builder().pool_max_idle_per_host(0).build().unwrap();
            let round = |count: usize| {
                let (http, token) = (http.clone(), app.window_token.clone());
                async move {
                    for _ in 0..count {
                        ws_open_close(addr, &token).await;
                        http.get(format!("http://{addr}/chat")).send().await.unwrap();
                    }
                }
            };
            round(20).await;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            let before = live.load(Ordering::SeqCst);
            round(300).await;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            let after = live.load(Ordering::SeqCst);
            assert!(after <= before + 2, "threads grew from {before} to {after}");
        });
    }

    #[tokio::test]
    async fn task_read_stop_gets_a_reply() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let reply = on_ws(&app, r#"{"type":"task_read_stop","request":"r1"}"#).unwrap();
        assert_eq!(reply, json!({"type": "task_read_stop_reply", "request": "r1", "ok": true, "data": null}));
        assert!(on_ws(&app, r#"{"type":"task_read_stop"}"#).is_none());
    }

    #[test]
    fn ws_requests_get_a_reply_by_kind() {
        assert_eq!(ws_reply("prompts", "app-1", true), json!({"type": "prompts_reply", "request": "app-1", "ok": true, "data": null}));
        let missed = ws_reply("nope", "app-2", false);
        assert_eq!(missed["type"], "nope_reply");
        assert_eq!(missed["ok"], false);
        assert_eq!(ws_reply("", "app-3", false)["type"], "unknown_reply");
    }
}
