use crate::app::App;
use crate::brains::{BrainInput, Checked, SavedBrain, check_input, choice_of};
use crate::provider::{self, Endpoint, Kind};
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use std::sync::Arc;

pub const NAME: &str = "Ollama";

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/brain/ollama", post(ollama_post))
}

async fn ollama_post(State(app): State<Arc<App>>, body: Option<axum::Json<Value>>) -> Response {
    let url = body.as_ref().and_then(|b| b.get("baseUrl")).and_then(Value::as_str).map(str::to_string);
    match make_or_choose(&app, url.as_deref()).await {
        Ok((brain, made)) => {
            crate::server::send_brain(&app);
            let body = json!({"ok": true, "id": brain.id, "choice": choice_of(&brain.id), "name": brain.name, "model": brain.model, "baseUrl": brain.base_url, "made": made});
            (StatusCode::OK, axum::Json(body)).into_response()
        }
        Err(error) => (StatusCode::BAD_GATEWAY, axum::Json(json!({"ok": false, "error": error}))).into_response(),
    }
}

pub fn saved(app: &App) -> Option<SavedBrain> {
    app.brains.list().into_iter().find(|b| b.kind == Kind::Ollama)
}

pub fn free_name(saved: &[SavedBrain]) -> String {
    let taken = |name: &str| saved.iter().any(|b| b.name.to_lowercase() == name.to_lowercase());
    (1..).map(|n| if n == 1 { NAME.to_string() } else { format!("{NAME} {n}") }).find(|name| !taken(name)).unwrap_or_default()
}

pub fn first_model(body: &Value) -> Option<String> {
    let list = body.get("data")?.as_array()?;
    list.iter().find_map(|item| item.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())).map(str::to_string)
}

pub async fn make_or_choose(app: &App, url: Option<&str>) -> Result<(SavedBrain, bool), String> {
    let wanted = url.map(|u| u.trim().trim_end_matches('/')).filter(|u| !u.is_empty());
    let (brain, made) = match saved(app) {
        Some(brain) => (choose_saved(app, brain, wanted).await?, false),
        None => make(app, wanted.unwrap_or(Kind::Ollama.default_url())).await?,
    };
    crate::agent::pick(app, &choice_of(&brain.id))?;
    Ok((brain, made))
}

async fn reach(app: &App, endpoint: &Endpoint) -> Result<Option<Value>, String> {
    match provider::models_body(&app.http, endpoint).await {
        Ok(body) => Ok(Some(body)),
        Err(error) if error.status.is_none() => Err(format!("Ollama is not reachable at {}. {error}", endpoint.base_url)),
        Err(_) => Ok(None),
    }
}

async fn choose_saved(app: &App, brain: SavedBrain, wanted: Option<&str>) -> Result<SavedBrain, String> {
    let mut endpoint = crate::agent::endpoint_of(&brain).await;
    let moved = wanted.filter(|url| *url != brain.base_url);
    if let Some(url) = moved {
        let input = BrainInput {
            name: brain.name.clone(),
            kind: Kind::Ollama.word().into(),
            base_url: url.into(),
            key_env: brain.key_env.clone().unwrap_or_default(),
            model: brain.model.clone(),
            ..BrainInput::default()
        };
        endpoint.base_url = check_input(Some(&brain.id), &input, &app.brains.list(), !endpoint.api_key.is_empty())?.base_url;
        endpoint.api_key = crate::keys::env_key(&brain).unwrap_or_default();
    }
    reach(app, &endpoint).await?;
    if moved.is_none() {
        return Ok(brain);
    }
    let changed = app.brains.update(&brain.id, |b| b.base_url = endpoint.base_url.clone()).map_err(|e| e.to_string())?;
    crate::keys::delete_keychain_key(&brain.id).await;
    changed.ok_or_else(|| "This brain is not saved any more.".to_string())
}

async fn make(app: &App, url: &str) -> Result<(SavedBrain, bool), String> {
    let list = app.brains.list();
    let input = BrainInput { name: free_name(&list), kind: Kind::Ollama.word().into(), base_url: url.into(), ..BrainInput::default() };
    let checked = check_input(None, &input, &list, false)?;
    let endpoint = Endpoint { kind: Kind::Ollama, base_url: checked.base_url.clone(), api_key: String::new() };
    let model = reach(app, &endpoint).await?.as_ref().and_then(first_model).unwrap_or_default();
    if let Some(brain) = saved(app) {
        return Ok((brain, false));
    }
    match app.brains.add(Checked { model, ..checked }) {
        Ok(brain) => Ok((brain, true)),
        Err(error) => saved(app).map(|brain| (brain, false)).ok_or_else(|| error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fake_server(body: &'static str) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                tx.send(String::from_utf8_lossy(&buf[..n]).to_string()).ok();
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).await.ok();
            }
        });
        (format!("http://127.0.0.1:{port}/v1"), rx)
    }

    #[tokio::test]
    async fn makes_a_new_brain_with_the_first_model() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (url, mut asked) = fake_server(r#"{"object":"list","data":[{"id":"small:1b","object":"model"},{"id":"big:9b","object":"model"}]}"#).await;
        let (brain, made) = make_or_choose(&app, Some(&url)).await.unwrap();
        assert!(made);
        assert_eq!(brain.name, "Ollama");
        assert_eq!(brain.kind, Kind::Ollama);
        assert_eq!(brain.model, "small:1b");
        assert_eq!(brain.base_url, url);
        assert_eq!(app.settings().choice, Some(choice_of(&brain.id)));
        let request = asked.recv().await.unwrap().to_lowercase();
        assert!(request.starts_with("get /v1/models"), "{request}");
        assert!(!request.contains("authorization"));
    }

    #[tokio::test]
    async fn chooses_the_saved_brain() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (url, _asked) = fake_server(r#"{"data":[{"id":"other"}]}"#).await;
        let input = BrainInput { name: "Mine".into(), kind: "ollama".into(), base_url: url.clone(), model: "kept".into(), ..BrainInput::default() };
        let first = app.brains.add(check_input(None, &input, &[], false).unwrap()).unwrap();
        let (brain, made) = make_or_choose(&app, None).await.unwrap();
        assert!(!made);
        assert_eq!(brain.id, first.id);
        assert_eq!(brain.model, "kept");
        assert_eq!(app.brains.list().len(), 1);
        assert_eq!(app.settings().choice, Some(choice_of(&first.id)));
    }

    #[tokio::test]
    async fn server_down_leaves_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        app.save_settings(|s| s.choice = Some("codex".into())).unwrap();
        let error = make_or_choose(&app, Some("http://127.0.0.1:1/v1")).await.unwrap_err();
        assert!(error.starts_with("Ollama is not reachable at http://127.0.0.1:1/v1."), "{error}");
        assert!(app.brains.list().is_empty());
        assert_eq!(app.settings().choice, Some("codex".into()));
    }

    #[tokio::test]
    async fn picks_a_free_name_when_ollama_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (url, _asked) = fake_server(r#"{"data":[{"id":"m1"}]}"#).await;
        let input = BrainInput { name: "Ollama".into(), kind: "openai".into(), base_url: url.clone(), ..BrainInput::default() };
        app.brains.add(check_input(None, &input, &[], false).unwrap()).unwrap();
        let (brain, made) = make_or_choose(&app, Some(&url)).await.unwrap();
        assert!(made);
        assert_eq!(brain.name, "Ollama 2");
        assert_eq!(brain.kind, Kind::Ollama);
        let (again, made) = make_or_choose(&app, Some(&url)).await.unwrap();
        assert!(!made);
        assert_eq!(again.id, brain.id);
        assert_eq!(app.brains.list().len(), 2);
    }

    #[tokio::test]
    async fn remote_http_is_refused_before_any_call() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        app.save_settings(|s| s.choice = Some("codex".into())).unwrap();
        let error = make_or_choose(&app, Some("http://192.0.2.1:11434/v1")).await.unwrap_err();
        assert_eq!(error, "Use https:// for a server that is not on this Mac.");
        assert!(app.brains.list().is_empty());
        assert_eq!(app.settings().choice, Some("codex".into()));
    }

    #[tokio::test]
    async fn a_new_url_moves_the_saved_brain() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (url, mut asked) = fake_server(r#"{"data":[{"id":"m1"}]}"#).await;
        let input = BrainInput { name: "Mine".into(), kind: "ollama".into(), base_url: "http://127.0.0.1:1/v1".into(), model: "kept".into(), ..BrainInput::default() };
        let first = app.brains.add(check_input(None, &input, &[], false).unwrap()).unwrap();
        let (brain, made) = make_or_choose(&app, Some(&format!("{url}/"))).await.unwrap();
        assert!(!made);
        assert_eq!(brain.id, first.id);
        assert_eq!(brain.base_url, url);
        assert_eq!(brain.model, "kept");
        assert_eq!(app.brains.get(&first.id).unwrap().base_url, url);
        assert!(asked.recv().await.unwrap().to_lowercase().starts_with("get /v1/models"));
    }

    #[tokio::test]
    async fn a_new_url_that_is_down_keeps_the_saved_brain() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        app.save_settings(|s| s.choice = Some("codex".into())).unwrap();
        let input = BrainInput { name: "Mine".into(), kind: "ollama".into(), base_url: "http://127.0.0.1:2/v1".into(), ..BrainInput::default() };
        let first = app.brains.add(check_input(None, &input, &[], false).unwrap()).unwrap();
        let error = make_or_choose(&app, Some("http://127.0.0.1:1/v1")).await.unwrap_err();
        assert!(error.starts_with("Ollama is not reachable at http://127.0.0.1:1/v1."), "{error}");
        assert_eq!(app.brains.get(&first.id).unwrap().base_url, "http://127.0.0.1:2/v1");
        assert_eq!(app.settings().choice, Some("codex".into()));
    }

    #[test]
    fn reads_the_first_model_in_server_order() {
        assert_eq!(first_model(&json!({"data": [{"id": "zeta"}, {"id": "alpha"}]})), Some("zeta".into()));
        assert_eq!(first_model(&json!({"data": []})), None);
        assert_eq!(first_model(&json!({"error": "x"})), None);
    }
}
