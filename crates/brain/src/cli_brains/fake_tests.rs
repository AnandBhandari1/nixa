use super::*;
use crate::app::App;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use tokio::sync::broadcast::Receiver;

const CODEX: &str = r#"#!/bin/sh
read line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
read line
read line
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"th1"},"model":"gpt-fake"}}'
read line
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"tu1"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"th1","turn":{"id":"tu1"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"th1","item":{"type":"agentMessage","phase":"commentary","text":"Looking now"}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":50,"method":"item/commandExecution/requestApproval","params":{}}'
read answer
case "$answer" in *decline*) ;; *) echo "bad answer $answer" >&2; exit 9;; esac
printf '%s\n' '{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"threadId":"th1","tokenUsage":{"last":{"inputTokens":7,"outputTokens":2,"totalTokens":9},"modelContextWindow":100}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"th1","item":{"type":"agentMessage","phase":"final_answer","text":"{\"speak\":true,\"text\":\"m1 is idle.\",\"heard\":\"me\"}"}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","status":"completed"}}}'
read line
exit 3
"#;

const ACP: &str = r#"#!/bin/sh
read line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
read line
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","models":{"availableModels":[{"modelId":"k2","name":"K2"}],"currentModelId":"k2"}}}'
read line
case "$line" in *"You are"*) ;; *) echo "no instructions in the first prompt" >&2; exit 8;; esac
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"{\"speak\":true,"}}}}'
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"\"text\":\"Hi from Kimi\",\"heard\":\"me\"}"}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn","usage":{"inputTokens":11,"outputTokens":4}}}'
read line
"#;

const OMP: &str = r#"#!/bin/sh
case " $* " in *" --mode rpc "*) ;; *) exit 4;; esac
case " $* " in *" --no-tools "*) ;; *) exit 5;; esac
turns=0
while IFS= read -r line; do
  case "$line" in
    *'"type":"get_state"'*) printf '%s\n' '{"type":"response","id":"get_state","success":true,"data":{"model":{"provider":"example","id":"first"}}}';;
    *'"type":"get_available_models"'*) printf '%s\n' '{"type":"response","id":"get_available_models","success":true,"data":{"models":[{"provider":"example","id":"first","name":"First","contextWindow":999}]}}';;
    *'"type":"set_model"'*)
      case "$line" in
        *'"modelId":"first"'*) printf '%s\n' '{"type":"response","id":"set_model","success":true,"data":{"provider":"example","id":"first"}}';;
        *) printf '%s\n' '{"type":"response","id":"set_model","success":false,"error":"Provider no longer offers this model."}';;
      esac;;
    *'"type":"prompt"'*)
      case "$line" in *'Chat message:'*) ;; *) exit 6;; esac
      turns=$((turns + 1))
      printf '%s\n' "$line" >> prompt-sent
      case "$line" in
        *'not done yet'*) printf '{"type":"prompt_result","id":%s,"status":"completed","yielded":false}\n' "$turns";;
        *'abort this'*) printf '{"type":"prompt_result","id":%s,"status":"aborted","yielded":false,"error":{"message":"Provider aborted the turn."}}\n' "$turns"; continue;;
        *'error this'*) printf '{"type":"prompt_result","id":%s,"status":"error","yielded":false,"error":{"message":"Provider returned a test error."}}\n' "$turns"; continue;;
      esac
      printf '%s\n' '{"type":"message_update","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_delta","delta":"{\"speak\":false,"}}'
      printf '%s\n' '{"type":"message_update","message":{"role":"assistant"},"assistantMessageEvent":{"type":"text_delta","delta":"\"text\":\"RPC answer '"$turns"'\"}"}}'
      printf '%s\n' '{"type":"agent_end","yielded":true,"isTerminal":true}'
      printf '{"type":"prompt_result","id":%s,"status":"completed","yielded":true}\n' "$turns";;
  esac
done
"#;

const CLAUDE: &str = r#"#!/bin/sh
case " $* " in *" --print --input-format stream-json --output-format stream-json "*) ;; *) exit 4;; esac
case " $* " in *" --tools  --permission-mode dontAsk "*) ;; *) exit 5;; esac
case " $* " in *" --allowedTools mcp__nixa_brain__* "*) ;; *) exit 6;; esac
printf '%s\n' '{"type":"system","subtype":"init","model":"some-sonnet"}'
while IFS= read -r line; do
  case "$line" in *'Chat message:'*'hello'*) ;; *) exit 7;; esac
  printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"{\"speak\":false,"}}}'
  printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"\"text\":\"Claude answer\"}"}}}'
  printf '%s\n' '{"type":"result","subtype":"success","usage":{"input_tokens":8,"output_tokens":4}}'
done
"#;

fn fake(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

async fn wait_for(rx: &mut Receiver<Value>, kind: &str) -> Value {
    loop {
        let value = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.expect("no event in time").expect("channel closed");
        if value["type"] == kind {
            return value;
        }
    }
}

fn app_with(kind: Kind, bin: String) -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    app.cli.save(|s| {
        s.bins.insert(kind.word().into(), bin);
    })
    .unwrap();
    app.save_settings(|s| s.choice = Some(kind.word().into())).unwrap();
    (dir, app)
}

#[tokio::test]
async fn codex_turn_reply_and_revive() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Codex, fake(bins.path(), "codex", CODEX));
    let mut rx = app.out.subscribe();
    crate::agent::tell(&app, "Typed: \"hi\"".into());
    assert!(crate::agent::busy(&app));
    let said = wait_for(&mut rx, "commentary").await;
    assert_eq!(said["text"], "Looking now");
    let reply = wait_for(&mut rx, "reply").await;
    assert_eq!(reply["reply"]["text"], "m1 is idle.");
    let idle = wait_for(&mut rx, "busy").await;
    assert_eq!(idle["busy"], false);
    assert_eq!(lock(&app.brain).usage.input, 7);
    assert_eq!(context(&app).map(|c| c.window), Some(100));
    assert_eq!(model_now(&app, Kind::Codex).as_deref(), Some("gpt-fake"));
    interrupt(&app);
    send(&app, Cmd::Tell("Typed: \"end\"".into()));
    let note = wait_for(&mut rx, "chat").await;
    assert!(note["item"]["text"].as_str().unwrap().contains("stopped"), "{note}");
    assert!(lock(&app.cli.live).need_recap);
    tick(&app);
    assert_eq!(running(&app), Some(Kind::Codex));
    stop(&app);
}

#[tokio::test]
async fn acp_kimi_turn_and_models() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Kimi, fake(bins.path(), "kimi", ACP));
    let mut rx = app.out.subscribe();
    crate::agent::tell(&app, "Typed: \"hello\"".into());
    let reply = wait_for(&mut rx, "reply").await;
    assert_eq!(reply["reply"]["text"], "Hi from Kimi");
    assert_eq!(lock(&app.brain).usage.input, 11);
    let models = crate::agent::load_models(&app).await.unwrap();
    assert_eq!(models[0].id, "k2");
    assert_eq!(model_now(&app, Kind::Kimi).as_deref(), Some("k2"));
    stop(&app);
}

#[tokio::test]
async fn jcode_acp_enables_nixa_mcp_tools_and_lists_models() {
    let bins = tempfile::tempdir().unwrap();
    let body = ACP.replacen(
        "#!/bin/sh\n",
        "#!/bin/sh\n[ \"$*\" = 'acp --no-update --tool-profile none --disable-base-tools --mcp-tools eager' ] || exit 9\n",
        1,
    ).replace("Hi from Kimi", "Hi from Jcode");
    let (_dir, app) = app_with(Kind::Jcode, fake(bins.path(), "jcode", &body));
    let mut events = app.out.subscribe();
    crate::agent::tell(&app, "Typed: \"hello\"".into());
    assert_eq!(wait_for(&mut events, "reply").await["reply"]["text"], "Hi from Jcode");
    assert_eq!(crate::agent::load_models(&app).await.unwrap()[0].id, "k2");
    assert!(acp::tool_rule(Kind::Jcode).contains("nixa_brain"));
    stop(&app);
}

#[tokio::test]
async fn dimcode_stays_selectable_without_provider_credentials() {
    let bins = tempfile::tempdir().unwrap();
    let body = r#"#!/bin/sh
[ "$*" = acp ] && [ "$DIMCODE_UPDATE_CHECK" = 0 ] || exit 9
read line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
read line
case "$line" in *'"mcpServers":'*'"name":"nixa_brain"'*) ;; *) exit 8;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required: Provider credentials are required"}}'
read line
"#;
    let (_dir, app) = app_with(Kind::Dim, fake(bins.path(), "dimcode", body));
    let listed = crate::agent::cli_list(&app);
    assert!(!listed.iter().any(|row| row["choice"] == "dim"));
    assert_eq!(crate::agent::brain_info(&app).unwrap()["kind"], "dim");
    let error = crate::agent::load_models(&app).await.unwrap_err();
    assert!(error.contains("DimCode needs provider credentials"), "{error}");
    assert_eq!(chosen(&app), Some(Kind::Dim));
}

#[tokio::test]
async fn omp_gets_nixa_tools_as_text_calls() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Omp, fake(bins.path(), "omp", OMP));
    assert!(text_tools::uses(&app, Kind::Omp));
    let rule = text_tools::rule(&app);
    assert!(rule.contains("- nixa_term_send("), "{rule}");
    assert!(rule.contains("- nixa_task("), "{rule}");
}

#[tokio::test]
async fn omp_rpc_streams_two_turns_in_one_session_and_lists_provider_models() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Omp, fake(bins.path(), "omp", OMP));
    let mut events = app.out.subscribe();
    let models = crate::agent::load_models(&app).await.unwrap();
    assert_eq!(models[0].id, "example/first");
    assert_eq!(models[0].max_tokens, Some(999));
    assert_eq!(model_now(&app, Kind::Omp).as_deref(), Some("example/first"));
    for (turn, text) in [(1, "hello"), (2, "hello again")] {
        crate::agent::tell(&app, format!("Typed: {text}"));
        let reply = wait_for(&mut events, "reply").await;
        assert_eq!(reply["reply"]["text"], format!("RPC answer {turn}"));
        assert_eq!(reply["reply"]["speak"], false);
        assert_eq!(wait_for(&mut events, "busy").await["busy"], false);
    }
    stop(&app);
}

#[tokio::test]
async fn omp_rpc_refuses_to_send_to_the_old_model_after_selection_fails() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Omp, fake(bins.path(), "omp", OMP));
    app.cli.save(|s| {
        s.models.insert(Kind::Omp.word().into(), "example/removed".into());
    }).unwrap();
    let mut events = app.out.subscribe();
    for text in ["Typed: hello", "Typed: keep this on the chosen model"] {
        crate::agent::tell(&app, text.into());
        let reply = wait_for(&mut events, "reply").await;
        assert_eq!(reply["reply"]["failed"], true);
        assert_eq!(reply["reply"]["text"], "omp could not select example/removed: Provider no longer offers this model.");
        assert_eq!(wait_for(&mut events, "busy").await["busy"], false);
    }
    crate::agent::load_models(&app).await.unwrap();
    assert!(!app.cli.work_dir.join("prompt-sent").exists());
    assert_eq!(model_now(&app, Kind::Omp).as_deref(), Some("example/first"));

    set_model(&app, Kind::Omp, "example/first").unwrap();
    crate::agent::tell(&app, "Typed: selected a different model".into());
    assert_eq!(wait_for(&mut events, "reply").await["reply"]["text"], "RPC answer 1");
    assert!(app.cli.work_dir.join("prompt-sent").is_file());
    stop(&app);
}

#[tokio::test]
async fn omp_rpc_waits_for_a_yielded_result() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Omp, fake(bins.path(), "omp", OMP));
    let mut events = app.out.subscribe();
    crate::agent::tell(&app, "Typed: not done yet".into());
    assert_eq!(wait_for(&mut events, "reply").await["reply"]["text"], "RPC answer 1");
    assert_eq!(wait_for(&mut events, "busy").await["busy"], false);
    stop(&app);
}

#[tokio::test]
async fn omp_rpc_ends_aborted_and_failed_turns_with_the_provider_error() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Omp, fake(bins.path(), "omp", OMP));
    let mut events = app.out.subscribe();
    for (text, error) in [
        ("Typed: abort this", "Provider aborted the turn."),
        ("Typed: error this", "Provider returned a test error."),
    ] {
        crate::agent::tell(&app, text.into());
        let reply = wait_for(&mut events, "reply").await;
        assert_eq!(reply["reply"]["failed"], true);
        assert_eq!(reply["reply"]["text"], error);
        assert_eq!(wait_for(&mut events, "busy").await["busy"], false);
    }
    crate::agent::tell(&app, "Typed: a follow-up after the errors".into());
    assert_eq!(wait_for(&mut events, "reply").await["reply"]["text"], "RPC answer 3");
    stop(&app);
}

#[tokio::test]
async fn claude_stream_json_keeps_a_session_for_successive_prompts() {
    let bins = tempfile::tempdir().unwrap();
    let (_dir, app) = app_with(Kind::Claude, fake(bins.path(), "claude", CLAUDE));
    let mut events = app.out.subscribe();
    let models = crate::agent::load_models(&app).await.unwrap();
    assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["sonnet", "opus", "haiku"]);
    for prompt in ["hello", "hello again"] {
        crate::agent::tell(&app, format!("Typed: {prompt}"));
        assert_eq!(wait_for(&mut events, "reply").await["reply"]["text"], "Claude answer");
    }
    assert_eq!(model_now(&app, Kind::Claude).as_deref(), Some("some-sonnet"));
    stop(&app);
}

#[tokio::test]
async fn missing_bin_says_so_once_and_waits() {
    let (_dir, app) = app_with(Kind::Agy, "/nonexistent/agy".into());
    let mut rx = app.out.subscribe();
    if find_bin(Kind::Agy, &app.cli.saved()).is_some() {
        return;
    }
    crate::agent::tell(&app, "Typed: \"x\"".into());
    let reply = wait_for(&mut rx, "reply").await;
    assert!(reply["reply"]["failed"].as_bool().unwrap());
    let note = wait_for(&mut rx, "chat").await;
    let note = note["item"]["text"].as_str().unwrap().to_string();
    let note = if note.contains("did not start") { note } else { wait_for(&mut rx, "chat").await["item"]["text"].as_str().unwrap().to_string() };
    assert!(note.contains("did not start"), "{note}");
    assert!(lock(&app.cli.live).retry_at > now_ms());
    tick(&app);
    assert_eq!(running(&app), None);
    assert!(!crate::agent::busy(&app));
}

#[test]
fn reply_line_shows_time_and_tokens() {
    assert_eq!(super::reply_line("opencode", 42, Some(3.26), (100, 20, 5)), "opencode reply (42 chars, took 3.3s, tokens in 100 out 20 cached 5)");
    assert_eq!(super::reply_line("codex", 1, None, (0, 0, 0)), "codex reply (1 chars, tokens in 0 out 0 cached 0)");
}
