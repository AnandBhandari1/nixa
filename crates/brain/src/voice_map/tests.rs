use super::*;
use std::collections::HashMap;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn deps() -> Deps {
    Deps { known: None, fallback: builtin_list(), nixa_voice: "Pippa".into(), numbers: HashMap::new(), personas: HashMap::new() }
}

fn ctx(pairs: &[(&str, &str)]) -> Ctx {
    let mut value = json!({});
    for (k, v) in pairs {
        value[*k] = json!(v);
    }
    Ctx::of(&value)
}

#[test]
fn agent_names_and_keys_are_normalized() {
    assert_eq!(normalize_agent(" Claude Code "), "claude");
    assert_eq!(normalize_agent("cursor-agent"), "cursor");
    assert_eq!(normalize_agent("Antigravity"), "agy");
    assert_eq!(normalize_agent("omp"), "omp");
    assert_eq!(normalize_agent("omp-cli"), "omp");
    assert_eq!(normalize_agent("ompx"), "ompx");
    assert_eq!(normalize_agent("Oh My Pi"), "omp");
    assert_eq!(normalize_agent("pi"), "omp");
    assert_eq!(normalize_agent("aider"), "aider");
    assert_eq!(normalize_agent(""), "");
    assert_eq!(voice_key(Level::Terminal, " M3 "), "M3");
    assert_eq!(voice_key(Level::Space, " SHOP "), "shop");
    assert_eq!(voice_key(Level::Model, "GPT-5"), "gpt-5");
    assert_eq!(voice_key(Level::Space, &"x".repeat(300)).len(), 200);
}

#[test]
fn a_saved_map_is_cleaned() {
    let raw = json!({
        "default": " Harper ",
        "terminals": {"m1": "Zoey", "m2": "", "m3": 4},
        "agents": {"Claude Code": "Miles", " ": "Toby"},
        "spaces": {"SHOP": "Freya"},
        "kinds": {"Chat": "Hugh"},
        "models": "nope"
    });
    let map = clean(&raw);
    assert_eq!(map.default.as_deref(), Some("Harper"));
    assert_eq!(map.terminals.len(), 1);
    assert_eq!(map.agents.get("claude").map(String::as_str), Some("Miles"));
    assert_eq!(map.spaces.get("shop").map(String::as_str), Some("Freya"));
    assert_eq!(map.kinds.get("chat").map(String::as_str), Some("Hugh"));
    assert!(map.models.is_empty());
    assert!(!is_empty(&map));
    assert!(is_empty(&clean(&json!(null))));
}

#[test]
fn the_most_specific_rule_wins() {
    let mut map = VoiceMap::default();
    set_default(&mut map, Some("Delia"));
    set(&mut map, Level::Kind, "finished", Some("Hugh"));
    set(&mut map, Level::Space, "SHOP", Some("Freya"));
    set(&mut map, Level::Agent, "claude", Some("Miles"));
    set(&mut map, Level::Model, "opus", Some("Pippa"));
    set(&mut map, Level::Terminal, "m4", Some("Zoey"));
    let d = deps();
    let full = ctx(&[("terminal", "m4"), ("model", "Opus"), ("agent", "Claude"), ("space", "shop"), ("kind", "finished")]);
    let r = resolve(&map, &full, &d);
    assert_eq!((r.voice.as_str(), r.rule.level, r.rule.text.as_str()), ("Zoey", 1, "Level 1: agent m4"));
    let r = resolve(&map, &ctx(&[("terminal", "m5"), ("model", "opus"), ("agent", "claude")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.tier), ("Pippa", Tier::Model));
    let r = resolve(&map, &ctx(&[("agent", "claude"), ("space", "shop")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Miles", "Level 2: agent claude"));
    let r = resolve(&map, &ctx(&[("agent", "codex"), ("space", "SHOP"), ("kind", "finished")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Freya", "Level 3: space shop"));
    let r = resolve(&map, &ctx(&[("kind", "finished")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Hugh", "Level 4: kind of speech finished"));
    let r = resolve(&map, &ctx(&[("kind", "alarm")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Delia", "Level 5: default voice"));
}

#[test]
fn deep_lines_use_the_chat_voice_and_broken_voices_are_skipped() {
    let mut map = VoiceMap::default();
    set(&mut map, Level::Kind, "chat", Some("Hugh"));
    set(&mut map, Level::Agent, "claude", Some("Gone"));
    let r = resolve(&map, &ctx(&[("kind", "deep")]), &deps());
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Hugh", "Level 4: kind of speech chat"));
    let known = Deps { known: Some(vec!["hugh".into(), "Zoey".into()]), ..deps() };
    let r = resolve(&map, &ctx(&[("agent", "claude"), ("kind", "chat")]), &known);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("hugh", "Level 4: kind of speech chat; Gone is not available"));
}

#[test]
fn numbered_agents_get_voices_in_order_and_others_the_built_in_voice() {
    let mut d = deps();
    d.numbers.insert("m7".into(), 2);
    d.numbers.insert("m9".into(), 27);
    let map = VoiceMap::default();
    let r = resolve(&map, &ctx(&[("terminal", "m7"), ("agent", "claude")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.tier, r.rule.text.as_str()), ("Juniper", Tier::Automatic, "Automatic: agent 2 uses voice 2"));
    let r = resolve(&map, &ctx(&[("terminal", "m9")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Juniper", "Automatic: agent 27 uses voice 2"));
    let r = resolve(&map, &ctx(&[("terminal", "m8"), ("agent", "Codex")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Miles", "Built-in voice for codex"));
    let r = resolve(&map, &Ctx::default(), &Deps { nixa_voice: "Harper".into(), ..d.clone() });
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Harper", "Built-in voice for nixa"));
    let mut by_number = VoiceMap::default();
    set(&mut by_number, Level::Terminal, "m7", Some("Zoey"));
    let r = resolve(&by_number, &ctx(&[("terminal", "terminal 2")]), &d);
    assert_eq!((r.voice.as_str(), r.rule.text.as_str()), ("Zoey", "Level 1: agent m7"));
    let (moved, number) = terminal_number(&ctx(&[("terminal", "#27")]), &d.numbers);
    assert_eq!((moved.terminal.as_deref(), number), (Some("m9"), Some(27)));
    let (kept, number) = terminal_number(&ctx(&[("terminal", "5")]), &d.numbers);
    assert_eq!((kept.terminal.as_deref(), number), (Some("5"), Some(5)));
}

#[test]
fn agent_events_pick_the_kind_of_speech() {
    let (_dir, app) = app();
    assert_eq!(event_kind("blocked"), "attention");
    assert_eq!(event_kind("done"), "finished");
    assert_eq!(event_kind("weird"), "other");
    let event = crate::announce::AgentEvent { time: 0, kind: "done".into(), text: String::new(), terminal: "m3".into(), space: "Shop".into(), job: String::new(), agent_number: None, agent: "claude".into(), offer: None };
    assert_eq!(agent_voice(&app, &event), "Harper");
    let mut next = map(&app);
    set(&mut next, Level::Kind, "finished", Some("Hugh"));
    *lock(&app.voice_map.map) = next.clone();
    assert_eq!(agent_voice(&app, &event), "Hugh");
    set(&mut next, Level::Space, "shop", Some("Freya"));
    *lock(&app.voice_map.map) = next;
    assert_eq!(agent_voice(&app, &event), "Freya");
    lock(&app.announce.numbers).insert("m3".into(), 5);
    *lock(&app.voice_map.map) = VoiceMap::default();
    assert_eq!(agent_voice(&app, &event), "Hadley");
}

#[test]
fn live_terminals_are_listed_with_their_agent_and_space() {
    let (_dir, app) = app();
    let info: nixa_protocol::TermInfo = serde_json::from_value(json!({
        "id": "m2", "name": "Fix login", "labels": {}, "cmd": ["claude"], "cwd": "/tmp/nowhere", "running": true,
        "rows": 24, "cols": 80, "created_ms": 0, "seq": 0
    }))
    .unwrap();
    let mut gone = info.clone();
    gone.running = false;
    gone.id = nixa_protocol::TermId::from_number(3);
    lock(&app.announce.terms).insert("m2".into(), crate::announce::Track { info, agent: "claude".into(), exit: None });
    lock(&app.announce.terms).insert("m3".into(), crate::announce::Track { info: gone, agent: "codex".into(), exit: None });
    let out = live(&app);
    assert_eq!(out["terminals"], json!([{"address": "m2", "agent": "claude", "space": "", "label": "claude · no space · Fix login"}]));
}

#[test]
fn the_voice_list_knows_where_it_came_from() {
    let built = list_from(Vec::new());
    assert_eq!(built.source, Source::BuiltIn);
    assert_eq!(built.voices.len(), 25);
    assert_eq!(list_from(builtin_list()).source, Source::BuiltIn);
    let spoken = list_from(vec!["Zoey".into(), "Narrator".into(), "harper".into()]);
    assert_eq!(spoken.source, Source::Speak);
    assert_eq!(spoken.normal, vec!["Zoey".to_string(), "harper".to_string()]);
    assert_eq!(check_voice(&spoken.voices, " narrator "), Ok("Narrator".to_string()));
    assert!(check_voice(&spoken.voices, "Nobody").unwrap_err().contains("not one of the voices"));
    assert_eq!(check_voice(&spoken.voices, " "), Err("Pick a voice first.".to_string()));
}

#[test]
fn actions_are_read_from_the_window() {
    assert_eq!(parse(&json!({})), Ok(Action::State));
    assert_eq!(parse(&json!({"action": "humor", "value": "off"})), Ok(Action::Humor(Humor::Off)));
    assert!(parse(&json!({"action": "humor", "value": "loud"})).is_err());
    assert_eq!(parse(&json!({"action": "set", "level": "default", "voice": "Zoey"})), Ok(Action::Set { level: None, key: String::new(), voice: "Zoey".into() }));
    assert_eq!(parse(&json!({"action": "clear", "level": "agent", "key": "codex"})), Ok(Action::Clear { level: Some(Level::Agent), key: "codex".into() }));
    assert_eq!(parse(&json!({"action": "set", "level": "planet", "voice": "Zoey"})), Err("Unknown level.".to_string()));
    assert_eq!(parse(&json!({"action": "dance"})), Err("Unknown voices action \"dance\".".to_string()));
    let rows = rows_of(Some(&json!([{"id": "kind:chat", "ctx": {"agent": "nixa", "kind": "chat"}}, {"ctx": {}}, {"id": "default"}])));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].ctx, Ctx::nixa("chat"));
    assert_eq!(with_kind(&ctx(&[("agent", "Nixa")])).kind.as_deref(), Some("chat"));
    assert_eq!(with_kind(&ctx(&[("agent", "codex")])).kind.as_deref(), Some("other"));
    assert_eq!(models_of(&[("claude".into(), "opus".into()), ("codex".into(), "".into()), ("x".into(), "OPUS".into())]), vec![("opus".to_string(), "opus (claude)".to_string())]);
}

#[tokio::test]
async fn set_clear_reset_and_humor_are_saved() {
    let (dir, app) = app();
    lock(&app.voice.settings).voice_on = true;
    let rows = json!([{"id": "default", "ctx": {}}, {"id": "agent:codex", "ctx": {"agent": "codex", "kind": "finished"}}]);
    let out = run(&app, &json!({"action": "set", "level": "agent", "key": "Codex", "voice": "zoey", "rows": rows})).await.unwrap();
    assert_eq!(out["map"]["agents"]["codex"], "Zoey");
    assert_eq!(out["rows"][1]["voice"], "Zoey");
    assert_eq!(out["rows"][1]["rule"]["text"], "Level 2: agent codex");
    assert_eq!(out["rows"][0]["rule"]["tier"], "built-in");
    assert_eq!(out["source"], "built-in");
    assert_eq!(out["kinds"].as_array().unwrap().len(), 8);
    assert_eq!(out["humor"], "light");
    assert_eq!(out["live"]["terminals"], json!([]));
    assert!(out["live"]["spaces"].is_array());
    let path = dir.path().join(FILE);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(State::open(dir.path()).map.lock().unwrap().agents.get("codex").map(String::as_str), Some("Zoey"));
    assert!(run(&app, &json!({"action": "set", "level": "agent", "key": " ", "voice": "Zoey"})).await.unwrap_err().contains("Pick what"));
    assert!(run(&app, &json!({"action": "set", "level": "default", "voice": "Nobody"})).await.unwrap_err().contains("not one of"));
    run(&app, &json!({"action": "set", "level": "default", "voice": "Harper"})).await.unwrap();
    assert_eq!(pick(&app, &ctx(&[("agent", "omp")])), "Harper");
    let out = run(&app, &json!({"action": "clear", "level": "agent", "key": "codex"})).await.unwrap();
    assert!(out["map"]["agents"].as_object().unwrap().is_empty());
    let out = run(&app, &json!({"action": "humor", "value": "off"})).await.unwrap();
    assert_eq!(out["humor"], "off");
    assert_eq!(humor(&app), Humor::Off);
    assert_eq!(humor_rule(&app), "");
    run(&app, &json!({"action": "reset"})).await.unwrap();
    assert!(!path.exists());
    assert!(is_empty(&map(&app)));
    assert_eq!(pick(&app, &Ctx::nixa("chat")), "Pippa");
}

#[tokio::test]
async fn play_and_stop_send_the_idle_event() {
    let (_dir, app) = app();
    lock(&app.voice.settings).voice_on = true;
    let mut rx = app.out.subscribe();
    let out = run(&app, &json!({"action": "play", "voice": "harper"})).await.unwrap();
    assert_eq!(out["voice"], "Harper");
    let id = out["id"].as_u64().unwrap();
    assert_eq!(rx.recv().await.unwrap(), json!({"type": "voices_idle", "id": id}));
    let out = run(&app, &json!({"action": "play", "ctx": {"agent": "codex"}})).await.unwrap();
    assert_eq!(out["voice"], "Miles");
    assert_eq!(run(&app, &json!({"action": "stop"})).await.unwrap(), json!({"stopped": false}));
    lock(&app.voice_map.play).active = true;
    assert_eq!(run(&app, &json!({"action": "stop"})).await.unwrap(), json!({"stopped": true}));
    assert!(run(&app, &json!({"action": "play", "voice": "Nobody"})).await.is_err());
}

#[tokio::test]
async fn the_route_needs_the_window_token() {
    let (_dir, app) = app();
    let refused = voices_post(AxState(app.clone()), HeaderMap::new(), axum::Json(json!({"action": "reset"}))).await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let mut headers = HeaderMap::new();
    headers.insert(crate::window_key::HEADER, app.window_token.parse().unwrap());
    let ok = voices_post(AxState(app.clone()), headers.clone(), axum::Json(json!({"action": "state"}))).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let bad = voices_post(AxState(app.clone()), headers.clone(), axum::Json(json!({"action": "dance"}))).await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert_eq!(voices_get(AxState(app.clone()), HeaderMap::new()).await.status(), StatusCode::UNAUTHORIZED);
    let mut wrong = HeaderMap::new();
    wrong.insert(crate::window_key::HEADER, "wrong".parse().unwrap());
    assert_eq!(voices_get(AxState(app.clone()), wrong).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(voices_get(AxState(app), headers).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn start_does_not_list_voices_while_voice_is_off() {
    let (_dir, app) = app();
    warm(app.clone()).await;
    assert!(cached(&app).is_none());
    lock(&app.voice.settings).voice_on = true;
    warm(app.clone()).await;
    assert!(cached(&app).is_some());
}

async fn body_of(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn no_voice_tool_runs_while_voice_is_off() {
    let (_dir, app) = app();
    let mut headers = HeaderMap::new();
    headers.insert(crate::window_key::HEADER, app.window_token.parse().unwrap());
    let got = voices_get(AxState(app.clone()), headers.clone()).await;
    assert_eq!(got.status(), StatusCode::OK);
    let data = body_of(got).await["data"].clone();
    assert_eq!(data["source"], "off");
    assert_eq!(data["voiceOn"], false);
    assert_eq!(data["note"], OFF_LINE);
    assert_eq!(data["voices"].as_array().unwrap().len(), 25);
    let posted = voices_post(AxState(app.clone()), headers.clone(), axum::Json(json!({"action": "state"}))).await;
    assert_eq!(body_of(posted).await["data"]["source"], "off");
    run(&app, &json!({"action": "set", "level": "default", "voice": "Zoey"})).await.unwrap();
    assert_eq!(run(&app, &json!({"action": "play", "voice": "Zoey"})).await.unwrap_err(), OFF_PLAY);
    assert_eq!(run(&app, &json!({"action": "play", "ctx": {"agent": "codex"}})).await.unwrap_err(), OFF_PLAY);
    assert_eq!(play(&app, "Zoey"), Err(OFF_PLAY.to_string()));
    let refused = voices_post(AxState(app.clone()), headers.clone(), axum::Json(json!({"action": "play", "voice": "Zoey"}))).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    voice_list(&app, true).await;
    warm(app.clone()).await;
    assert_eq!(tool_runs(&app), 0);
    assert!(cached(&app).is_none());
    lock(&app.voice.settings).voice_on = true;
    let data = body_of(voices_get(AxState(app.clone()), headers.clone()).await).await["data"].clone();
    assert_eq!(data["voiceOn"], true);
    assert_eq!(data["note"], "");
    assert_eq!(data["source"], "built-in");
    assert_eq!(tool_runs(&app), 1);
    run(&app, &json!({"action": "play", "voice": "Zoey"})).await.unwrap();
    assert_eq!(tool_runs(&app), 2);
    lock(&app.voice.settings).voice_on = false;
    let data = body_of(voices_get(AxState(app.clone()), headers).await).await["data"].clone();
    assert_eq!(data["source"], "off");
    assert_eq!(tool_runs(&app), 2);
}

#[test]
fn own_agent_voice_comes_before_the_number_voice() {
    let d = Deps { personas: [("m3".to_string(), "Freya".to_string())].into_iter().collect(), numbers: [("m3".to_string(), 2)].into_iter().collect(), ..deps() };
    let ctx = Ctx { terminal: Some("m3".into()), ..Ctx::default() };
    let r = resolve(&VoiceMap::default(), &ctx, &d);
    assert_eq!(r.voice, "Freya");
    assert_eq!(r.rule.tier, Tier::Automatic);
}

#[test]
fn own_agent_voice_beats_kind_and_default_rules_but_not_agent_rules() {
    let d = Deps { personas: [("m3".to_string(), "Freya".to_string())].into_iter().collect(), ..deps() };
    let ctx = Ctx { terminal: Some("m3".into()), kind: Some("chat".into()), ..Ctx::default() };
    let mut map = VoiceMap::default();
    set_default(&mut map, Some("Delia"));
    set(&mut map, Level::Kind, "chat", Some("Zoey"));
    assert_eq!(resolve(&map, &ctx, &d).voice, "Freya");
    set(&mut map, Level::Terminal, "m3", Some("Hugh"));
    assert_eq!(resolve(&map, &ctx, &d).voice, "Hugh");
}
