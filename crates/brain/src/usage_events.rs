use crate::app::{App, lock};
use crate::extras::usage::{Counts, TurnUsage};
use crate::util::now_ms;
use serde_json::{Map, Value, json};

pub const KIND: &str = "usage";

pub fn event(snapshot: Value) -> Value {
    let mut out = match snapshot {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    out.insert("type".into(), json!(KIND));
    Value::Object(out)
}

pub fn current(app: &App, now: u64) -> Value {
    event(lock(&app.extras.usage).snapshot(now))
}

fn brain_counts(app: &App) -> (bool, Counts) {
    let brain = lock(&app.brain);
    (brain.busy, Counts { input: brain.usage.input, output: brain.usage.output, cached: brain.usage.cached })
}

pub fn sample(app: &App, busy: Option<bool>, now: u64) -> Option<TurnUsage> {
    let (ended, view) = {
        let mut store = lock(&app.extras.usage);
        let (live, counts) = brain_counts(app);
        let ended = store.sample(busy.unwrap_or(live), counts, now);
        (ended, ended.map(|_| store.snapshot(now)))
    };
    if let Some(view) = view {
        let mut out = event(view);
        if let Some(turn) = &ended {
            add_turn(&mut out, &turn_fields(app, turn));
        }
        app.send_out(out);
    }
    ended
}

pub fn context_pct(context: Option<crate::compaction::Context>) -> Option<f32> {
    let context = context.filter(|c| c.window > 0)?;
    Some(((context.used as f64 / context.window as f64) * 100.0).min(100.0) as f32)
}

pub fn turn_fields(app: &App, turn: &TurnUsage) -> Value {
    let model = crate::agent::brain_info(app).and_then(|b| b.get("model").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    json!({"model": model, "input": turn.input, "output": turn.output, "context_pct": context_pct(crate::agent::context(app))})
}

fn add_turn(out: &mut Value, fields: &Value) {
    if let (Value::Object(out), Value::Object(fields)) = (out, fields) {
        out.extend(fields.clone());
    }
}

pub fn turn_started(app: &App) {
    sample(app, Some(true), now_ms());
}

pub fn turn_ended(app: &App) {
    sample(app, Some(false), now_ms());
}

pub fn busy_changed(app: &App, on: bool) {
    sample(app, Some(on), now_ms());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brains::{Checked, choice_of};
    use crate::provider::Kind;
    use axum::Router;
    use axum::routing::post;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::broadcast::Receiver;

    fn app() -> (tempfile::TempDir, Arc<App>) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        (dir, app)
    }

    fn add(app: &App, input: u64, output: u64, cached: u64) {
        let mut brain = lock(&app.brain);
        brain.usage.input += input;
        brain.usage.output += output;
        brain.usage.cached += cached;
    }

    fn usage_events(rx: &mut Receiver<Value>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(value) = rx.try_recv() {
            if value["type"] == KIND {
                out.push(value);
            }
        }
        out
    }

    async fn next_usage(rx: &mut Receiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let value = rx.recv().await.unwrap();
                if value["type"] == KIND {
                    return value;
                }
            }
        })
        .await
        .expect("no usage event")
    }

    #[test]
    fn the_event_has_the_fields_the_chat_reads() {
        let out = event(json!({"day": "2026-11-23", "today": {"tokens": 5, "cached": 1, "turns": 2}, "turn": {"ms": 900, "tokens": 5}}));
        assert_eq!(out["type"], "usage");
        assert_eq!(out["today"]["turns"], 2);
        assert_eq!(out["turn"]["tokens"], 5);
        assert_eq!(event(Value::Null), json!({"type": "usage"}));
    }

    #[test]
    fn a_turn_sends_one_usage_event_with_its_counts() {
        let (_dir, app) = app();
        let mut rx = app.out.subscribe();
        add(&app, 7, 0, 0);
        sample(&app, Some(true), 1000);
        add(&app, 120, 30, 50);
        assert!(usage_events(&mut rx).is_empty());
        let turn = sample(&app, Some(false), 3500).unwrap();
        assert_eq!((turn.input, turn.output, turn.cached, turn.tokens, turn.ms), (120, 30, 50, 200, 2500));
        let sent = usage_events(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0]["input"].as_u64(), sent[0]["output"].as_u64()), (Some(120), Some(30)));
        assert!(sent[0]["model"].is_string());
        assert!(sent[0].get("context_pct").is_some());
        assert_eq!(sent[0]["today"]["turns"], 1);
        assert_eq!(sent[0]["today"]["tokens"], 200);
        assert_eq!(sent[0]["today"]["cached"], 50);
        assert_eq!(sent[0]["turn"]["tokens"], 200);
        assert_eq!(sent[0]["turn"]["ms"], 2500);
        assert_eq!(sent[0]["turnText"], "2.5 s, 200");
        assert!(sample(&app, Some(false), 4000).is_none());
        assert!(sample(&app, None, 4500).is_none());
        assert!(usage_events(&mut rx).is_empty());
        assert_eq!(current(&app, 4500)["today"]["turns"], 1);
    }

    #[test]
    fn context_pct_is_a_share_of_the_window() {
        use crate::compaction::Context;
        assert_eq!(context_pct(Some(Context { used: 50, window: 200, estimate: true })), Some(25.0));
        assert_eq!(context_pct(Some(Context { used: 5, window: 0, estimate: true })), None);
        assert_eq!(context_pct(None), None);
    }

    #[test]
    fn the_tick_and_the_turn_end_never_count_twice() {
        let (_dir, app) = app();
        let mut rx = app.out.subscribe();
        lock(&app.brain).busy = true;
        sample(&app, None, 1000);
        turn_started(&app);
        add(&app, 10, 2, 0);
        lock(&app.brain).busy = false;
        assert!(sample(&app, None, 2000).is_some());
        turn_ended(&app);
        let sent = usage_events(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["today"]["turns"], 1);
        assert_eq!(sent[0]["today"]["tokens"], 12);
    }

    #[test]
    fn each_queued_turn_ends_on_its_own() {
        let (_dir, app) = app();
        let mut rx = app.out.subscribe();
        lock(&app.brain).busy = true;
        turn_started(&app);
        add(&app, 10, 1, 0);
        turn_ended(&app);
        turn_started(&app);
        add(&app, 20, 2, 0);
        turn_ended(&app);
        let sent = usage_events(&mut rx);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["turn"]["tokens"], 11);
        assert_eq!(sent[1]["turn"]["tokens"], 22);
        assert_eq!(sent[1]["today"]["turns"], 2);
        assert_eq!(sent[1]["today"]["tokens"], 33);
    }

    #[test]
    fn busy_changes_from_cli_brains_end_a_turn() {
        let (_dir, app) = app();
        let mut rx = app.out.subscribe();
        crate::agent::set_busy(&app, true);
        add(&app, 11, 4, 0);
        crate::agent::set_busy(&app, false);
        let sent = usage_events(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["today"]["tokens"], 15);
        crate::agent::set_busy(&app, false);
        assert!(usage_events(&mut rx).is_empty());
    }

    async fn fake_model(reply: &str) -> String {
        let text = json!({"speak": false, "text": reply}).to_string();
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices": [{"delta": {"content": text}}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 1200, "completion_tokens": 34, "prompt_tokens_details": {"cached_tokens": 800}}})
        );
        let router = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let body = body.clone();
                async move { ([("content-type", "text/event-stream")], body) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.ok() });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn a_real_brain_turn_sends_the_model_counts() {
        let (_dir, app) = app();
        let url = fake_model("Hello there.").await;
        let id = app.brains.add(Checked { name: "Local".into(), kind: Kind::Openai, base_url: url, key_env: None, model: "small".into() }).unwrap().id;
        app.save_settings(|s| s.choice = Some(choice_of(&id))).unwrap();
        let mut rx = app.out.subscribe();
        crate::agent::tell(&app, "Typed: hello".into());
        let first = next_usage(&mut rx).await;
        assert_eq!(first["model"], "small");
        assert_eq!((first["input"].as_u64(), first["output"].as_u64()), (Some(1200), Some(34)));
        assert!(first["context_pct"].as_f64().is_some_and(|p| p > 0.0 && p <= 100.0), "{first}");
        assert_eq!(first["today"]["turns"], 1);
        assert_eq!(first["today"]["tokens"], 2034);
        assert_eq!(first["today"]["cached"], 800);
        assert_eq!(first["turn"]["tokens"], 2034);
        crate::agent::tell(&app, "Typed: again".into());
        let second = next_usage(&mut rx).await;
        assert_eq!(second["today"]["turns"], 2);
        assert_eq!(second["today"]["tokens"], 4068);
        assert_eq!(second["turn"]["tokens"], 2034);
        assert!(lock(&app.chat).list().iter().any(|i| i.text == "Hello there."));
    }
}
