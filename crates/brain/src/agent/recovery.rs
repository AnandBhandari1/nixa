use super::{App, ProviderError, StepInput, Turn, endpoint_of, provider};
use crate::brains::{SavedBrain, is_local_url};
use crate::reply::{AgentReply, extract_json, parse_reply};
use crate::util::{clip_flat, log};
use tokio_util::sync::CancellationToken;

const RECONCILE: &str = "The provider stopped after a tool or approval may have run. I did not retry. Check what happened before continuing.";
pub(crate) const NOT_RETRIED: &str = "The chat provider stopped. This turn was not retried.";
const NO_FALLBACK: &str = "The chat provider stopped and no other eligible chat brain is configured.";
const FALLBACK_FAILED: &str = "My fallback brain did not recover the turn.";
const FAILURE_STARTS: [&str; 4] = [
    "The provider stopped after a tool or approval may have run.",
    "The chat provider stopped.",
    "The chat provider stopped and no other eligible chat brain",
    FALLBACK_FAILED,
];

pub(crate) fn failure_line(text: &str) -> bool {
    let text = text.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').trim();
    if text.is_empty() {
        return false;
    }
    if FAILURE_STARTS.iter().any(|start| text.starts_with(start)) {
        return true;
    }
    let first = text.split('.').next().unwrap_or("").trim();
    first.starts_with("My ") && first.split_whitespace().count() <= 6
        && (first.ends_with(" brain failed") || first.ends_with(" brain is not running"))
}

pub(crate) fn drop_copied(tag: &str, who: &str, reply: AgentReply) -> AgentReply {
    if reply.failed || !failure_line(&reply.text) {
        return reply;
    }
    log(tag, &format!("{who} copied a failure line, dropped"));
    AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: reply.heard, failed: false }
}

pub(crate) fn retryable(status: Option<u16>, message: &str) -> bool {
    if let Some(code) = status {
        return matches!(code, 429 | 500 | 502 | 503 | 504 | 529);
    }
    let message = message.to_ascii_lowercase();
    ["rate limit", "rate_limit", "usage limit", "quota exceeded", "insufficient_quota",
        "too many requests", "overloaded", "service unavailable", "connection refused",
        "error sending request", "dns error", "failed: 429", "failed: 503"]
        .iter().any(|phrase| message.contains(phrase))
}

pub(crate) fn outcome_uncertain(problem: &str, terminal_failure: bool, tool_seen: bool) -> bool {
    let rejected = problem.starts_with("OpenCode POST /api/session/")
        && problem.split_once("/prompt failed: ").is_some_and(|(_, reason)| {
            matches!(reason.split_whitespace().next(), Some("429" | "503"))
        });
    tool_seen || (!terminal_failure && !rejected)
}

pub(crate) async fn recover(
    app: &App, primary: Option<&SavedBrain>, request: &str, uncertain: bool, cancel: &CancellationToken,
) -> AgentReply {
    let reply = recover_turn(app, primary, request, uncertain, cancel).await;
    if reply.failed {
        log("brain", &format!("turn not recovered: {}", clip_flat(&reply.text, 300)));
    }
    if reply.failed && !super::from_user(request) { AgentReply { speak: false, ..reply } } else { reply }
}

async fn recover_turn(
    app: &App, primary: Option<&SavedBrain>, request: &str, uncertain: bool, cancel: &CancellationToken,
) -> AgentReply {
    if uncertain { return AgentReply::failed(RECONCILE); }
    if !super::from_user(request) || cancel.is_cancelled() {
        return AgentReply::failed(NOT_RETRIED);
    }
    for brain in app.brains.list() {
        let local = is_local_url(&brain.base_url);
        if brain.model.trim().is_empty() || primary.is_some_and(|source| {
            source.id == brain.id || source.base_url.trim_end_matches('/') == brain.base_url.trim_end_matches('/')
                || (is_local_url(&source.base_url) && !local)
        }) { continue; }
        let endpoint = endpoint_of(&brain).await;
        if !local && endpoint.api_key.is_empty() { continue; }
        let result = attempt(app, &endpoint, &brain.model, request, cancel).await;
        return match result {
            Ok(reply) => reply,
            Err(error) => {
                log("brain", &format!("fallback brain {} failed: {}", brain.name, clip_flat(&error.message, 300)));
                AgentReply::failed(format!("{FALLBACK_FAILED} {error}"))
            }
        };
    }
    AgentReply::failed(NO_FALLBACK)
}

async fn attempt(app: &App, endpoint: &provider::Endpoint, model: &str, request: &str, cancel: &CancellationToken) -> Result<AgentReply, ProviderError> {
    let system = format!("Recover a chat response without taking any actions. No tools or approvals are available. Do not claim to have acted. If the request needs an action, explain that it has not been taken. Use the original space context and the current Decision context. {}", crate::reply::REPLY_RULE);
    let turns = [Turn::User { text: format!("{request}\n\n{}", crate::prompt::context_line(app)) }];
    let input = StepInput { system: &system, turns: &turns, tools: &[], model, effort: None, max_tokens: None };
    let result = provider::step(&app.http, endpoint, &input, cancel, |_| {}).await?;
    let reply = parse_reply(&extract_json(&result.text));
    if !result.calls.is_empty() || reply.text.is_empty() {
        return Err(ProviderError { status: None, message: "The fallback did not return a text-only answer.".into() });
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brains::Checked;
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    async fn save(app: &App, name: &str, status: StatusCode) -> Arc<Mutex<Vec<Value>>> {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let recorded = hits.clone();
        let router = Router::new().route("/chat/completions", post(move |Json(body): Json<Value>| {
            let recorded = recorded.clone();
            async move {
                recorded.lock().unwrap().push(body);
                (status, "data: {\"choices\":[{\"delta\":{\"content\":\"Recovered\"}}]}\n\ndata: [DONE]\n\n").into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        app.brains.add(Checked {
            name: name.into(), kind: provider::Kind::Openai,
            base_url: format!("http://{}", listener.local_addr().unwrap()),
            key_env: None, model: "fixture".into(),
        }).unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        hits
    }

    #[tokio::test]
    async fn opencode_503_recovers_once_but_unclear_outcomes_pause() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let hits = save(&app, "Backup", StatusCode::OK).await;
        let request = "Typed: \"Summarize\"";
        let token = CancellationToken::new();
        let rejection = "OpenCode POST /api/session/demo/prompt failed: 503 Service Unavailable";
        for uncertain in [
            outcome_uncertain(rejection, false, true),
            outcome_uncertain("OpenCode POST /api/session/demo/prompt failed: error sending request", false, false),
            outcome_uncertain("service unavailable", false, false),
        ] {
            let reply = recover(&app, None, request, uncertain, &token).await;
            assert_eq!(reply.text, RECONCILE);
        }
        assert!(hits.lock().unwrap().is_empty());
        let reply = recover(&app, None, request, outcome_uncertain(rejection, false, false), &token).await;
        assert_eq!(reply.text, "Recovered");
        assert_eq!(hits.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn recovery_preserves_context_and_offers_no_tools() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let hits = save(&app, "Backup", StatusCode::OK).await;
        let request = "Typed: \"Summarize\"\n\nContext: original space demo, Decision manual.";
        let reply = recover(&app, None, request, false, &CancellationToken::new()).await;
        assert_eq!(reply.text, "Recovered");
        assert!(!reply.failed);
        let calls = hits.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0]["messages"][1]["content"].as_str().unwrap().starts_with(request));
        assert!(calls[0].get("tools").is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)));
    }

    #[tokio::test]
    async fn recovery_stops_after_one_failure_and_never_replays_uncertain_work() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let failed = save(&app, "Unavailable", StatusCode::SERVICE_UNAVAILABLE).await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let unused = save(&app, "Unused", StatusCode::OK).await;
        let token = CancellationToken::new();
        let request = "Typed: \"Continue\"";
        let paused = recover(&app, None, request, true, &token).await;
        assert_eq!(paused.text, RECONCILE);
        assert!(failed.lock().unwrap().is_empty());
        let reply = recover(&app, None, request, false, &token).await;
        assert!(reply.failed && reply.text.contains("did not recover"));
        assert_eq!(failed.lock().unwrap().len(), 1);
        assert!(unused.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn api_turn_recovers_without_changing_the_selected_brain() {
        let dir = tempfile::tempdir().unwrap();
        let app = Arc::new(App::open(dir.path()).unwrap());
        let primary = save(&app, "Primary", StatusCode::TOO_MANY_REQUESTS).await;
        let choice = crate::brains::choice_of(&app.brains.list()[0].id);
        app.save_settings(|settings| settings.choice = Some(choice.clone())).unwrap();
        let fallback = save(&app, "Backup", StatusCode::OK).await;
        crate::app::lock(&app.brain).queue.push("Typed: \"Explain this\"\n\nContext: sandbox, Decision manual.".into());
        super::super::turn(&app, &CancellationToken::new()).await.unwrap();
        assert_eq!(primary.lock().unwrap().len(), 1);
        assert_eq!(fallback.lock().unwrap().len(), 1);
        assert_eq!(app.settings().choice, Some(choice));
        assert!(matches!(crate::app::lock(&app.brain).turns.last(),
            Some(Turn::Assistant { text, calls }) if text == "Recovered" && calls.is_empty()));
    }

    #[test]
    fn only_provider_failures_qualify() {
        assert!(retryable(Some(429), ""));
        assert!(retryable(None, "You reached the usage limit"));
        assert!(retryable(Some(503), ""));
        assert!(!retryable(Some(400), "rate limit"));
        assert!(!retryable(None, "aborted"));
    }

    #[test]
    fn failure_lines_are_known_and_copies_are_dropped() {
        for line in [RECONCILE, NOT_RETRIED, NO_FALLBACK, "My fallback brain did not recover the turn. 503",
            "My OpenCode brain failed. It stopped answering.", "My brain failed. boom", "\"My Codex brain is not running. I try to start it again.\""] {
            assert!(failure_line(line), "{line}");
        }
        for line in ["", "Done. Codex finished the tests.", "My plan failed because the tests are red.", "The provider is fine."] {
            assert!(!failure_line(line), "{line}");
        }
        let copied = drop_copied("cli-brain", "opencode", AgentReply::said(NOT_RETRIED));
        assert!(copied.text.is_empty() && !copied.speak && !copied.failed);
        let real = drop_copied("cli-brain", "opencode", AgentReply::failed(NOT_RETRIED));
        assert_eq!(real.text, NOT_RETRIED);
        let normal = drop_copied("cli-brain", "opencode", AgentReply::said("All good."));
        assert_eq!(normal.text, "All good.");
    }

    #[tokio::test]
    async fn a_failed_background_turn_is_not_spoken() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let cancel = CancellationToken::new();
        let quiet = recover(&app, None, "Event: Nixa was idle", false, &cancel).await;
        assert!(quiet.failed && !quiet.speak, "{quiet:?}");
        let unsure = recover(&app, None, "Event: Nixa was idle", true, &cancel).await;
        assert!(unsure.failed && !unsure.speak);
        let user = recover(&app, None, "Typed: hello", true, &cancel).await;
        assert!(user.failed && user.speak);
    }
}
