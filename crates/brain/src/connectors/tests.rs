use super::call::{Ctx, Outcome};
use super::http::{Net, build, encode, fill};
use super::manifest::{Kind, Permission, check_input, parse};
use super::secrets::Keychain;
use super::*;
use crate::app::App;
use axum::routing::post;
use serde_json::Map;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;
use std::time::Duration;

const FAKE_SECURITY: &str = r#"#!/bin/sh
D="$(dirname "$0")/store"
mkdir -p "$D"
if [ "$1" = "-i" ]; then
  read -r line
  set -- $line
  acct=""; hex=""
  while [ $# -gt 0 ]; do case "$1" in -a) acct="$2"; shift 2;; -X) hex="$2"; shift 2;; *) shift;; esac; done
  printf '%s' "$hex" | xxd -r -p > "$D/$acct"
  exit 0
fi
cmd="$1"; shift
acct=""
while [ $# -gt 0 ]; do case "$1" in -a) acct="$2"; shift 2;; *) shift;; esac; done
case "$cmd" in
  find-generic-password) [ -f "$D/$acct" ] || exit 44; cat "$D/$acct"; echo; exit 0;;
  delete-generic-password) [ -f "$D/$acct" ] || exit 44; rm "$D/$acct"; exit 0;;
esac
exit 1
"#;

const SCRIPT: &str = r#"#!/bin/sh
input=$(cat)
case "$1" in
  echo) printf 'got %s key %s' "$input" "$NIXA_SECRET_SHOP_KEY";;
  fail) echo "bad thing" >&2; exit 3;;
  slow) sleep 5;;
  pic) printf 'PNGDATA' > "$NIXA_CONNECTOR_DIR/a.png"; printf '{"path":"%s","note":"a pic"}' "$NIXA_CONNECTOR_DIR/a.png";;
  away) printf '{"path":"/etc/hosts"}';;
  env) env;;
esac
"#;

fn args(value: Value) -> Args {
    value.as_object().cloned().unwrap_or_default()
}

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn install(root: &Path, name: &str, manifest: Value) -> PathBuf {
    let folder = root.join(format!("connector-{name}"));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("connector.json"), manifest.to_string()).unwrap();
    folder
}

fn script(folder: &Path, name: &str, text: &str) -> PathBuf {
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn fake_keychain(dir: &Path) -> Keychain {
    let bin = script(dir, "security", FAKE_SECURITY);
    Keychain { bin, file: None }
}

fn http_manifest() -> Value {
    json!({
        "connector": "shop",
        "version": 1,
        "description": "A test shop",
        "hosts": ["api.shop.test", "localhost"],
        "secrets": ["shop-key"],
        "optionalSecrets": ["shop-extra"],
        "actions": [
            {"name": "items", "description": "List items", "permission": "read", "reach": "outside",
             "inputSchema": {"type": "object", "properties": {"kind": {"type": "string", "description": "item kind"}, "count": {"type": "integer"}}, "required": ["kind"]},
             "request": {"url": "https://localhost/items/{kind}", "query": {"count": "{count}", "fixed": "{mode}"}, "headers": {"Authorization": "Bearer {secret:shop-key}"}, "pick": "/items"},
             "fixedArgs": {"mode": "all"}, "public": true},
            {"name": "order", "description": "Place an order", "permission": "write", "reach": "outside",
             "inputSchema": {"type": "object", "properties": {"item": {"type": "string"}, "size": {"type": "string", "enum": ["s", "m"]}}},
             "request": {"method": "POST", "url": "https://localhost/order", "body": {"item": "{item}", "size": "{size}", "note": "for {item}"}}},
            {"name": "away", "description": "Goes away", "permission": "read", "reach": "outside", "request": {"url": "https://localhost/away"}},
            {"name": "echo", "description": "Echo", "permission": "read", "reach": "outside", "request": {"url": "https://localhost/echo"}, "public": true}
        ]
    })
}

fn command_manifest(name: &str) -> Value {
    json!({
        "connector": name,
        "version": 1,
        "kind": "command",
        "description": "A test tool",
        "secrets": [],
        "optionalSecrets": [format!("{name}-key")],
        "actions": [
            {"name": "echo", "description": "Echo input", "permission": "read", "reach": "local", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}, "public": true},
            {"name": "fail", "description": "Fails", "permission": "read", "reach": "local"},
            {"name": "slow", "description": "Slow", "permission": "read", "reach": "local", "timeoutMs": 1000},
            {"name": "pic", "description": "Picture", "permission": "read", "reach": "local", "returns": "image"},
            {"name": "away", "description": "Picture outside", "permission": "read", "reach": "local", "returns": "image"},
            {"name": "env", "description": "Env", "permission": "read", "reach": "local", "public": true},
            {"name": "post", "description": "Post a note", "permission": "write", "reach": "outside", "tool": "echo", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}},
            {"name": "ship", "description": "Ship it", "permission": "write", "reach": "outside", "tool": "echo", "stop": true}
        ]
    })
}

fn ctx<'a>(root: &'a Path, kc: &'a Keychain, net: &'a Net) -> Ctx<'a> {
    Ctx { root, keychain: kc, net, private_blocked: false }
}

#[test]
fn parses_a_good_manifest() {
    let m = parse(&http_manifest(), "connector-shop").unwrap();
    assert_eq!(m.kind, Kind::Http);
    assert_eq!(m.actions.len(), 4);
    assert_eq!(m.all_secrets(), vec!["shop-key", "shop-extra"]);
    let items = &m.actions[0];
    assert!(items.public && !items.private);
    assert_eq!(items.request.as_ref().unwrap().pick.as_deref(), Some("/items"));
    assert_eq!(items.fixed.get("mode"), Some(&json!("all")));
    assert_eq!(m.actions[1].permission, Permission::Write);
}

#[test]
fn refuses_bad_manifests() {
    let cases: Vec<(Value, &str)> = vec![
        (json!({"kind": "module"}), "JavaScript"),
        (json!({"kind": "mcp"}), "MCP"),
        (json!({"kind": "shell"}), "not supported"),
        (json!({"version": 0}), "version"),
        (json!({"hosts": ["https://x.test"]}), "plain host"),
        (json!({"secrets": ["other-key"]}), "must be \"shop\""),
        (json!({"private": true, "public": true}), "both private and public"),
        (json!({"actions": []}), "at least one action"),
        (json!({"command": "run"}), "only for command"),
    ];
    for (change, want) in cases {
        let mut raw = http_manifest();
        for (k, v) in change.as_object().unwrap() {
            raw[k] = v.clone();
        }
        let error = parse(&raw, "connector-shop").unwrap_err();
        assert!(error.contains(want), "{want}: {error}");
    }
    assert!(parse(&http_manifest(), "connector-other").unwrap_err().contains("folder must be named"));
    let action_cases: Vec<(Value, &str)> = vec![
        (json!({"request": {"url": "https://evil.test/x"}}), "not in hosts"),
        (json!({"request": {"url": "http://localhost/x"}}), "https://"),
        (json!({"request": {"url": "https://localhost:8080/x"}}), "plain host name"),
        (json!({"request": {"url": "https://{kind}.localhost/x"}}), "plain host name"),
        (json!({"request": {"url": "https://localhost/x", "method": "POST"}}), "only use GET or HEAD"),
        (json!({"request": {"url": "https://localhost/{nope}"}}), "not an input"),
        (json!({"request": {"url": "https://localhost/x", "headers": {"X-Key": "{secret:other}"}}}), "secret \"other\""),
        (json!({"request": {"url": "https://localhost/x", "headers": {"Host": "x"}}}), "not allowed"),
        (json!({"reach": "local"}), "reach must be \"outside\""),
        (json!({"returns": "image"}), "returns text"),
        (json!({"timeoutMs": 500}), "timeoutMs"),
        (json!({"name": "Bad-Name"}), "not allowed"),
        (json!({"permission": "admin"}), "permission"),
        (json!({"request": null}), "needs a request"),
        (json!({"inputSchema": {"type": "object", "properties": {"x": {"type": "date"}}}}), "needs a type"),
        (json!({"inputSchema": {"type": "object", "properties": {}, "required": ["y"]}}), "not in properties"),
    ];
    for (change, want) in action_cases {
        let mut raw = http_manifest();
        for (k, v) in change.as_object().unwrap() {
            raw["actions"][0][k] = v.clone();
        }
        let error = parse(&raw, "connector-shop").unwrap_err();
        assert!(error.contains(want), "{want}: {error}");
    }
    let mut twice = http_manifest();
    twice["actions"][1]["name"] = json!("items");
    assert!(parse(&twice, "connector-shop").unwrap_err().contains("listed twice"));
    let mut command = command_manifest("tool");
    command["actions"][0]["request"] = json!({"url": "https://x"});
    assert!(parse(&command, "connector-tool").unwrap_err().contains("only for http"));
    command = command_manifest("tool");
    command["command"] = json!("../run");
    assert!(parse(&command, "connector-tool").unwrap_err().contains("plain file name"));
}

#[test]
fn checks_inputs() {
    let m = parse(&http_manifest(), "connector-shop").unwrap();
    let items = &m.actions[0];
    let order = &m.actions[1];
    assert_eq!(check_input(items, Some(&json!({"kind": "tea", "count": 3}))).unwrap(), obj(json!({"kind": "tea", "count": 3, "mode": "all"})));
    assert!(check_input(items, Some(&json!({"kind": "tea", "other": 1}))).unwrap_err().contains("Unknown input \"other\""));
    assert!(check_input(items, Some(&json!({}))).unwrap_err().contains("required"));
    assert!(check_input(items, Some(&json!({"kind": ""}))).unwrap_err().contains("required"));
    assert!(check_input(items, Some(&json!({"kind": "tea", "count": 1.5}))).unwrap_err().contains("integer"));
    assert!(check_input(items, Some(&json!({"kind": 3}))).unwrap_err().contains("string"));
    assert!(check_input(items, Some(&json!("x"))).unwrap_err().contains("object"));
    assert!(check_input(order, Some(&json!({"size": "xl"}))).unwrap_err().contains("one of: s, m"));
    let long = "a".repeat(manifest::MAX_TEXT + 1);
    assert!(check_input(order, Some(&json!({"item": long}))).is_err());
    assert_eq!(check_input(order, None).unwrap(), Map::new());
}

#[test]
fn money_words_are_found() {
    assert!(gate::money_text("pay $5 now"));
    assert!(gate::money_text("120 MYR"));
    assert!(!gate::money_text("read the payload"));
}

#[test]
fn fills_requests() {
    let m = parse(&http_manifest(), "connector-shop").unwrap();
    let secrets: HashMap<String, String> = [("shop-key".to_string(), "s3cret-value".to_string())].into();
    let input = check_input(&m.actions[0], Some(&json!({"kind": "a b/c"}))).unwrap();
    let built = build(m.actions[0].request.as_ref().unwrap(), &input, &secrets).unwrap();
    assert_eq!(built.url.as_str(), "https://localhost/items/a%20b%2Fc?fixed=all");
    assert_eq!(built.headers, vec![("Authorization".to_string(), "Bearer s3cret-value".to_string())]);
    let order = check_input(&m.actions[1], Some(&json!({"item": "tea"}))).unwrap();
    let built = build(m.actions[1].request.as_ref().unwrap(), &order, &secrets).unwrap();
    assert_eq!(built.body, Some(json!({"item": "tea", "note": "for tea"})));
    assert_eq!(encode("a-b_c.d~e f"), "a-b_c.d~e%20f");
    assert!(fill("https://x/{a}/z", &Map::new(), &secrets, true).unwrap_err().contains("needs the input a"));
    assert_eq!(fill("{a}", &Map::new(), &secrets, false).unwrap(), None);
    assert_eq!(fill("x{a}y", &Map::new(), &secrets, false).unwrap(), Some("xy".into()));
}

#[test]
fn only_listed_https_hosts() {
    let hosts = vec!["api.shop.test".to_string()];
    let url = |s: &str| reqwest::Url::parse(s).unwrap();
    assert!(http::allowed(&url("https://api.shop.test/x"), &hosts, Permission::Read, "GET").is_ok());
    assert!(http::allowed(&url("http://api.shop.test/x"), &hosts, Permission::Read, "GET").is_err());
    assert!(http::allowed(&url("https://api.shop.test:8443/x"), &hosts, Permission::Read, "GET").is_err());
    assert!(http::allowed(&url("https://u:p@api.shop.test/x"), &hosts, Permission::Read, "GET").is_err());
    assert!(http::allowed(&url("https://other.test/x"), &hosts, Permission::Read, "GET").unwrap_err().contains("not in this connector's hosts"));
    assert!(http::allowed(&url("https://api.shop.test/x"), &hosts, Permission::Read, "POST").unwrap_err().contains("GET or HEAD"));
    assert!(http::allowed(&url("https://api.shop.test/x"), &hosts, Permission::Write, "POST").is_ok());
}

async fn serve(router: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    port
}

fn shop_router(seen: Arc<Mutex<Vec<String>>>) -> Router {
    let items = {
        let seen = seen.clone();
        move |headers: HeaderMap, uri: axum::http::Uri| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(format!("{} {}", uri, headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("")));
                axum::Json(json!({"items": [{"name": "tea"}], "other": 1}))
            }
        }
    };
    let order = {
        let seen = seen.clone();
        move |body: String| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(body);
                (StatusCode::PAYMENT_REQUIRED, "no money")
            }
        }
    };
    Router::new()
        .route("/items/{kind}", get(items))
        .route("/order", post(order))
        .route("/away", get(|| async { (StatusCode::FOUND, [("location", "https://evil.test/steal")], "") }))
        .route("/echo", get(|headers: HeaderMap| async move { headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("none").to_string() }))
}

#[tokio::test]
async fn http_calls_go_only_to_listed_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    let mut raw = http_manifest();
    raw["actions"][3]["request"]["headers"] = json!({"Authorization": "Bearer {secret:shop-key}"});
    install(&root, "shop", raw);
    let kc = fake_keychain(dir.path());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = serve(shop_router(seen.clone())).await;
    let net = Net { client: http::client(), plain_port: Some(port) };
    let c = ctx(&root, &kc, &net);
    let missing = call::prepare(&c, "shop", "items", Some(&json!({"kind": "tea"})), 1).await.unwrap_err();
    assert!(missing.text.contains("needs its secret first (shop-key)") && missing.text.contains("nixa connector secret shop-key"), "{}", missing.text);
    secrets::save(&kc, "shop", "shop-key", "s3cret-value").await.unwrap();
    let plan = call::prepare(&c, "shop", "items", Some(&json!({"kind": "tea", "count": 2})), 1).await.unwrap();
    let text = call::execute(&c, &plan).await.unwrap();
    assert_eq!(text, format!("{}[{{\"name\":\"tea\"}}]", call::DATA_PREFIX));
    assert_eq!(seen.lock().unwrap()[0], "/items/tea?count=2&fixed=all Bearer s3cret-value");
    let plan = call::prepare(&c, "shop", "order", Some(&json!({"item": "tea"})), 1).await.unwrap();
    let error = call::execute(&c, &plan).await.unwrap_err();
    assert!(error.contains("answered 402: no money"), "{error}");
    assert_eq!(seen.lock().unwrap()[1], "{\"item\":\"tea\",\"note\":\"for tea\"}");
    let plan = call::prepare(&c, "shop", "away", None, 1).await.unwrap();
    let error = call::execute(&c, &plan).await.unwrap_err();
    assert!(error.contains("evil.test is not in this connector's hosts"), "{error}");
    let plan = call::prepare(&c, "shop", "echo", None, 1).await.unwrap();
    let text = call::execute(&c, &plan).await.unwrap();
    let done = call::finish(&root, plan.row.clone(), Outcome { verdict: "ran", by: By::None, text: &text, log_result: plan.log_result }, &plan.secrets, 5);
    assert!(done.text.ends_with("Bearer <secret>") && !done.text.contains("s3cret"), "{}", done.text);
    let rows = store::calls(&root, 5, Some("shop"));
    assert_eq!(rows[0].verdict, "ran");
    assert!(!rows[0].result.contains("s3cret"));
}

#[tokio::test]
async fn command_connectors_run_their_own_program() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    let folder = install(&root, "tool", command_manifest("tool"));
    let kc = fake_keychain(dir.path());
    let net = http::live();
    let c = ctx(&root, &kc, &net);
    let error = call::execute(&c, &call::prepare(&c, "tool", "echo", None, 1).await.unwrap()).await.unwrap_err();
    assert!(error.contains("program run was not found"), "{error}");
    let program = script(&folder, "run", SCRIPT);
    let plan = call::prepare(&c, "tool", "echo", Some(&json!({"text": "hi"})), 1).await.unwrap();
    let text = call::execute(&c, &plan).await.unwrap();
    assert_eq!(text, format!("{}got {{\"action\":\"echo\",\"input\":{{\"text\":\"hi\"}}}} key", call::DATA_PREFIX));
    let error = call::execute(&c, &call::prepare(&c, "tool", "fail", None, 1).await.unwrap()).await.unwrap_err();
    assert!(error.contains("stopped with code 3: bad thing"), "{error}");
    let started = std::time::Instant::now();
    let error = call::execute(&c, &call::prepare(&c, "tool", "slow", None, 1).await.unwrap()).await.unwrap_err();
    assert!(error.contains("took more than 1 seconds"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4));
    let text = call::execute(&c, &call::prepare(&c, "tool", "pic", None, 1).await.unwrap()).await.unwrap();
    assert!(text.starts_with(call::PRIVATE_PREFIX) && text.contains("Image file: ") && text.contains("Type: image/png, 1 KB") && text.contains("a pic"), "{text}");
    let private = root.join("private").join("tool");
    assert_eq!(std::fs::metadata(&private).unwrap().permissions().mode() & 0o777, 0o700);
    let error = call::execute(&c, &call::prepare(&c, "tool", "away", None, 1).await.unwrap()).await.unwrap_err();
    assert!(error.contains("outside the connector's private folder"), "{error}");
    let env = call::execute(&c, &call::prepare(&c, "tool", "env", None, 1).await.unwrap()).await.unwrap();
    assert!(env.contains("NIXA_CONNECTOR_ACTION=env") && !env.contains("CARGO_PKG_NAME"), "{env}");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o777)).unwrap();
    let error = call::execute(&c, &call::prepare(&c, "tool", "echo", None, 1).await.unwrap()).await.unwrap_err();
    assert!(error.contains("can be changed by other users"), "{error}");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = call::execute(&c, &call::prepare(&c, "tool", "echo", None, 1).await.unwrap()).await.unwrap_err();
    assert!(error.contains("not a runnable file"), "{error}");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    secrets::save(&kc, "tool", "tool-key", "tok-123456").await.unwrap();
    let mut raw = command_manifest("tool");
    raw["optionalSecrets"] = json!(["tool-key"]);
    install(&root, "tool", raw);
    let plan = call::prepare(&c, "tool", "echo", None, 1).await.unwrap();
    assert_eq!(command::env_name("tool-key"), "NIXA_SECRET_TOOL_KEY");
    assert!(plan.secrets.contains_key("tool-key"));
}

#[tokio::test]
async fn prepare_refuses_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    install(&root, "tool", command_manifest("tool"));
    let broken = root.join("connector-bad");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("connector.json"), "{not json").unwrap();
    let kc = fake_keychain(dir.path());
    let net = http::live();
    let mut c = ctx(&root, &kc, &net);
    assert!(call::prepare(&c, "nope", "x", None, 1).await.unwrap_err().text.contains("Installed: bad, tool."));
    assert!(call::prepare(&c, "bad", "x", None, 1).await.unwrap_err().text.contains("is broken"));
    assert!(call::prepare(&c, "tool", "x", None, 1).await.unwrap_err().text.contains("has no action \"x\""));
    let refused = call::prepare(&c, "tool", "echo", Some(&json!({"zzz": 1})), 1).await.unwrap_err();
    assert!(refused.text.contains("Unknown input"));
    assert_eq!(refused.row.permission, "read");
    assert!(call::prepare(&c, "tool", "post", None, 1).await.is_ok());
    assert!(call::prepare(&c, "tool", "echo", None, 1).await.is_ok());
    let mut raw = command_manifest("tool");
    raw["actions"][0]["private"] = json!(true);
    raw["actions"][0]["public"] = json!(false);
    install(&root, "tool", raw);
    c.private_blocked = true;
    assert_eq!(call::prepare(&c, "tool", "echo", None, 1).await.unwrap_err().text, call::PRIVATE_BLOCKED);
    c.private_blocked = false;
    let plan = call::prepare(&c, "tool", "echo", Some(&json!({"text": "my private words"})), 1).await.unwrap();
    assert_eq!(plan.row.input, "{}");
    assert!(!plan.log_result);
    let view = call::view(&root, &kc).await;
    assert_eq!(view[0]["status"].as_str().unwrap().split(':').next(), Some("broken"));
    assert_eq!(view[1]["status"], "ready");
    assert_eq!(view[1]["hostsChecked"], false);
    assert!(view[1]["actions"][6].get("asksEveryTime").is_none());
    assert_eq!(view[1]["secrets"][0], json!({"entry": "tool-key", "set": false, "required": false}));
}

#[test]
fn trust_and_call_log() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    install(&root, "tool", command_manifest("tool"));
    let mut read_only = http_manifest();
    read_only["connector"] = json!("reader");
    read_only["secrets"] = json!([]);
    read_only["optionalSecrets"] = json!([]);
    read_only["actions"] = json!([{"name": "page", "description": "Read", "permission": "read", "reach": "outside", "request": {"url": "https://localhost/x"}}]);
    install(&root, "reader", read_only);
    let mut local_tool = command_manifest("local");
    local_tool["optionalSecrets"] = json!([]);
    local_tool["actions"] = json!([{"name": "echo", "description": "Echo", "permission": "read", "reach": "local"}]);
    install(&root, "local", local_tool);
    assert!(call::set_trust(&root, "local", true, 1).unwrap().contains("changes nothing now"));
    assert!(call::set_trust(&root, "local", false, 1).unwrap().contains("not trusted"));
    assert!(!store::trusted(&root, "tool"));
    assert!(call::set_trust(&root, "tool", true, 1).unwrap().contains("is trusted"));
    assert!(store::trusted(&root, "tool"));
    assert!(call::set_trust(&root, "reader", true, 1).unwrap_err().contains("trust changes nothing"));
    assert!(call::set_trust(&root, "ghost", true, 1).unwrap_err().contains("No working connector"));
    assert!(call::set_trust(&root, "tool", false, 2).unwrap().contains("not trusted"));
    assert!(!store::trusted(&root, "tool"));
    assert_eq!(std::fs::metadata(root.join(store::TRUST_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
    for i in 0..(store::TRIM_AT + 5) {
        let row = store::CallRow { at: i as u64, connector: if i % 2 == 0 { "tool".into() } else { "reader".into() }, action: "echo".into(), verdict: "ran".into(), ..Default::default() };
        store::add_call(&root, &row).unwrap();
    }
    let text = std::fs::read_to_string(root.join(store::CALLS_FILE)).unwrap();
    assert!(text.lines().count() <= store::TRIM_AT);
    let last = store::calls(&root, 3, Some("tool"));
    assert_eq!(last.len(), 3);
    assert!(last.iter().all(|r| r.connector == "tool"));
    assert!(last[0].at > last[1].at);
    let row = call::base_row("tool", "echo", 10);
    let done = call::finish(&root, row, Outcome { verdict: "ran", by: By::None, text: &format!("{}secret words", call::PRIVATE_PREFIX), log_result: false }, &HashMap::new(), 15);
    assert!(done.ok);
    let last = &store::calls(&root, 1, None)[0];
    assert_eq!((last.result.as_str(), last.ms), ("", 5));
}

#[tokio::test]
async fn keychain_entries_use_hex_and_stay_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let kc = fake_keychain(dir.path());
    let line = secrets::save_line(&kc, "shop", "shop-key", "a \"b\"").unwrap();
    assert_eq!(line, "add-generic-password -U -s nixa-connector -a shop:shop-key -X 6120226222\n");
    assert!(secrets::save_line(&kc, "shop", "Bad Name", "x").is_err());
    assert!(secrets::check_secret("two\nlines").is_err());
    assert!(secrets::check_secret("").is_err());
    assert!(secrets::check_secret("ok value").is_ok());
    assert_eq!(secrets::get(&kc, "shop", "shop-key").await, None);
    secrets::save(&kc, "shop", "shop-key", "a \"b\" c").await.unwrap();
    assert_eq!(secrets::get(&kc, "shop", "shop-key").await.as_deref(), Some("a \"b\" c"));
    assert_eq!(secrets::get(&kc, "other", "shop-key").await, None);
    assert!(secrets::remove(&kc, "shop", "shop-key").await);
    assert!(!secrets::remove(&kc, "shop", "shop-key").await);
    let filed = Keychain { bin: kc.bin.clone(), file: Some(PathBuf::from("/tmp/a\"b")) };
    assert!(secrets::save_line(&filed, "shop", "shop-key", "x").is_err());
}

#[tokio::test]
async fn secrets_stay_with_their_connector() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    let kc = fake_keychain(dir.path());
    let mut a = command_manifest("alpha");
    a["optionalSecrets"] = json!(["alpha-key"]);
    install(&root, "alpha", a);
    secrets::save(&kc, "alpha", "alpha-key", "alpha-s3cret").await.unwrap();
    let mut b = command_manifest("alpha-key");
    b["optionalSecrets"] = json!(["alpha-key"]);
    install(&root, "alpha-key", b);
    let beta = call::scan(&root).into_iter().find(|i| i.name == "alpha-key").unwrap().manifest.unwrap();
    assert!(call::secrets_of(&kc, &beta).await.is_empty());
    assert!(call::remove_secret(&root, &kc, "alpha-key:alpha-key").await.unwrap().contains("was not set"));
    assert!(call::remove_secret(&root, &kc, "alpha-key").await.unwrap_err().contains("Several connectors"));
    assert!(call::remove_secret(&root, &kc, "gamma:alpha-key").await.is_err());
    assert_eq!(secrets::get(&kc, "alpha", "alpha-key").await.as_deref(), Some("alpha-s3cret"));
}

#[tokio::test]
async fn old_shared_entries_move_to_their_only_owner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    let kc = fake_keychain(dir.path());
    let mut a = command_manifest("alpha");
    a["optionalSecrets"] = json!(["alpha-key"]);
    install(&root, "alpha", a);
    let legacy = format!("add-generic-password -U -s {} -a alpha-key -X {}\n", secrets::SERVICE, secrets::hex("old-value"));
    let mut child = tokio::process::Command::new(&kc.bin).arg("-i").stdin(std::process::Stdio::piped()).spawn().unwrap();
    tokio::io::AsyncWriteExt::write_all(child.stdin.as_mut().unwrap(), legacy.as_bytes()).await.unwrap();
    drop(child.stdin.take());
    child.wait().await.unwrap();
    assert_eq!(secrets::get_legacy(&kc, "alpha-key").await.as_deref(), Some("old-value"));
    call::migrate(&root, &kc).await;
    assert_eq!(secrets::get(&kc, "alpha", "alpha-key").await.as_deref(), Some("old-value"));
    assert_eq!(secrets::get_legacy(&kc, "alpha-key").await, None);
}

#[tokio::test]
async fn cli_lists_trusts_logs_and_saves_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    let kc = fake_keychain(dir.path());
    let run = |words: &'static [&'static str], typed: &'static str| {
        let root = root.clone();
        let kc = kc.clone();
        async move {
            let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            let mut out = Vec::new();
            let mut read = |_: &str| typed.to_string();
            let code = cli::run(&argv, &root, &kc, &mut read, &mut out).await;
            (code, String::from_utf8(out).unwrap())
        }
    };
    let (code, text) = run(&["list"], "").await;
    assert_eq!(code, 0);
    assert!(text.starts_with("No connectors installed."));
    install(&root, "tool", command_manifest("tool"));
    let (code, text) = run(&["list"], "").await;
    assert_eq!(code, 0);
    assert!(text.contains("tool (command): echo (read, local)"), "{text}");
    assert!(text.contains("secret tool-key: missing (optional)"), "{text}");
    assert_eq!(run(&["secret", "ghost-key"], "x").await.0, 1);
    assert_eq!(run(&["secret", "tool-key"], "  \n").await.0, 1);
    let (code, text) = run(&["secret", "tool-key"], "tok-abc\n").await;
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Saved tool-key for connector tool"));
    assert!(!text.contains("tok-abc"));
    assert!(run(&["list"], "").await.1.contains("secret tool-key: set (optional)"));
    assert!(run(&["secret", "tool-key", "--remove"], "").await.1.contains("Removed tool-key."));
    assert!(run(&["trust", "tool"], "").await.1.contains("is trusted"));
    assert!(run(&["list"], "").await.1.contains("[trusted]"));
    assert!(run(&["untrust", "tool"], "").await.1.contains("not trusted"));
    assert!(run(&["log"], "").await.1.contains("No calls logged."));
    store::add_call(&root, &store::CallRow { at: 1_795_305_600_000, connector: "tool".into(), action: "echo".into(), permission: "read".into(), reach: "local".into(), input: "{}".into(), verdict: "ran".into(), approved_by: "none".into(), ok: true, ms: 7, ..Default::default() }).unwrap();
    let (code, text) = run(&["log", "tool", "--limit", "5"], "").await;
    assert_eq!(code, 0);
    assert_eq!(text, "2026-11-22T00:00:00Z tool.echo read/local ran by none 7ms input {} result \n");
    assert_eq!(run(&["log", "--limit"], "").await.0, 2);
    assert_eq!(run(&["nope"], "").await.0, 2);
    assert!(run(&["help"], "").await.1.contains("nixa connector secret") || run(&["help"], "").await.1.contains("secret ENTRY"));
    assert_eq!(run(&["folder"], "").await.1.trim(), root.display().to_string());
}

#[tokio::test]
#[ignore]
async fn live_https_call() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    install(&root, "page", json!({"connector": "page", "version": 1, "description": "A public page", "hosts": ["example.com"], "actions": [{"name": "home", "description": "Read the page", "permission": "read", "reach": "outside", "public": true, "request": {"url": "https://example.com/"}}]}));
    let kc = fake_keychain(dir.path());
    let net = http::live();
    let c = ctx(&root, &kc, &net);
    let text = call::execute(&c, &call::prepare(&c, "page", "home", None, 1).await.unwrap()).await.unwrap();
    assert!(text.starts_with(call::DATA_PREFIX) && text.contains("Example Domain"), "{text}");
}

fn app_in(dir: &Path) -> Arc<App> {
    App::open(&dir.join("data")).unwrap()
}

#[tokio::test]
async fn outside_writes_wait_for_a_yes_unless_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    app.decision.set_choice("nixa").unwrap();
    let root = app.connectors.root.clone();
    let folder = install(&root, "tool", command_manifest("tool"));
    script(&folder, "run", SCRIPT);
    let listed: Value = serde_json::from_str(&call(&app, LIST, &Args::new()).await.unwrap().unwrap()).unwrap();
    assert_eq!(listed[0]["connector"], "tool");
    let ran = call(&app, CALL, &args(json!({"connector": "tool", "action": "echo", "input": {"text": "hi"}}))).await.unwrap().unwrap();
    assert!(ran.contains("got {\"action\":\"echo\""), "{ran}");
    let refused = call(&app, CALL, &args(json!({"connector": "tool", "action": "nope"}))).await.unwrap().unwrap_err();
    assert!(refused.contains("has no action"));
    let post = args(json!({"connector": "tool", "action": "post", "input": {"text": "hello"}}));
    let held: Value = serde_json::from_str(&call(&app, CALL, &post).await.unwrap().unwrap()).unwrap();
    assert_eq!(held["outcome"], "held", "{held}");
    crate::app::lock(&app.brain).last_user = Some(("yes tool".into(), crate::util::now_ms() + 1));
    let ran = call(&app, CALL, &post).await.unwrap().unwrap();
    assert!(ran.starts_with(call::PRIVATE_PREFIX), "{ran}");
    call::set_trust(&root, "tool", true, 1).unwrap();
    assert!(call(&app, CALL, &post).await.unwrap().unwrap().starts_with(call::PRIVATE_PREFIX));
    let ship = args(json!({"connector": "tool", "action": "ship"}));
    let held: Value = serde_json::from_str(&call(&app, CALL, &ship).await.unwrap().unwrap()).unwrap();
    assert_eq!(held["outcome"], "held", "a stop action asks even when trusted");
    let rows = store::calls(&root, 20, Some("tool"));
    let words: Vec<(&str, &str)> = rows.iter().rev().map(|r| (r.verdict.as_str(), r.approved_by.as_str())).collect();
    assert_eq!(words, vec![("ran", "none"), ("refused", "none"), ("ran", "user"), ("ran", "trusted")]);
    assert_eq!(rows.iter().find(|r| r.action == "echo" && r.verdict == "ran").unwrap().input, "{\"text\":\"hi\"}");
}

#[tokio::test]
async fn routes_need_the_window_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    install(&app.connectors.root, "tool", command_manifest("tool"));
    let port = serve(routes().with_state(app.clone())).await;
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    let refused = client.get(format!("{base}/connectors")).send().await.unwrap();
    assert_eq!(refused.status(), 401);
    let token = app.window_token.clone();
    let got: Value = client.get(format!("{base}/connectors")).header(nixa_protocol::paths::WINDOW_TOKEN_HEADER, &token).send().await.unwrap().json().await.unwrap();
    assert_eq!(got["connectors"]["connectors"][0]["connector"], "tool");
    assert_eq!(got["connectors"]["card"], Value::Null);
    let trust: Value = client.post(format!("{base}/connectors")).header(nixa_protocol::paths::WINDOW_TOKEN_HEADER, &token).json(&json!({"type": "trust", "connector": "tool"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(trust["ok"], true);
    assert_eq!(trust["connectors"]["connectors"][0]["trusted"], true);
    let bad = client.post(format!("{base}/connectors")).header(nixa_protocol::paths::WINDOW_TOKEN_HEADER, &token).json(&json!({"type": "secret", "entry": "ghost", "value": "x"})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let calls: Value = client.get(format!("{base}/connectors/calls?limit=5")).header(nixa_protocol::paths::WINDOW_TOKEN_HEADER, &token).send().await.unwrap().json().await.unwrap();
    assert_eq!(calls["calls"], json!([]));
    assert_eq!(client.get(format!("{base}/connectors/calls")).send().await.unwrap().status(), 401);
    assert!(ws(&app, "connector_secret", &json!({"entry": "tool-key", "value": "a\nb"})).unwrap().is_err());
    assert!(ws(&app, "other", &json!({})).is_none());
}

#[test]
fn built_in_mcp_and_url_connectors() {
    let web = parse(&builtin::web_search(), "connector-web-search").unwrap();
    assert_eq!(web.kind, Kind::Mcp);
    let mut other = builtin::web_search();
    other["connector"] = json!("search");
    assert!(parse(&other, "connector-search").unwrap_err().contains("Only the built-in"));
    let mut bad_tool = builtin::browser(builtin::BROWSER_OWN);
    bad_tool["actions"][0]["tool"] = json!("browser_evaluate");
    assert!(parse(&bad_tool, "connector-browser-own").unwrap_err().contains("is not allowed"));
    let mut with_request = builtin::web_search();
    with_request["actions"][0]["request"] = json!({"url": "https://x.test"});
    assert!(parse(&with_request, "connector-web-search").unwrap_err().contains("only for http"));
    let own = parse(&builtin::browser(builtin::BROWSER_OWN), "connector-browser-own").unwrap();
    assert!(own.actions.iter().any(|a| a.name == "look"));
    assert!(own.actions.iter().any(|a| a.name == "click"));
    let get = parse(&builtin::http_get(), "connector-http-get").unwrap();
    let action = &get.actions[0];
    let input = check_input(action, Some(&json!({"url": "https://api.github.com/zen?x=1"}))).unwrap();
    let built = build(action.request.as_ref().unwrap(), &input, &HashMap::new()).unwrap();
    assert_eq!(built.url.as_str(), "https://api.github.com/zen?x=1");
    assert!(built.headers.iter().all(|(k, _)| k != "Authorization"));
    let secrets: HashMap<String, String> = [("http-get".to_string(), "t0ken-value".to_string())].into();
    let built = build(action.request.as_ref().unwrap(), &input, &secrets).unwrap();
    assert!(built.headers.contains(&("Authorization".to_string(), "Bearer t0ken-value".to_string())));
    assert!(http::allowed(&built.url, &get.hosts, Permission::Read, "GET").is_ok());
    let away = check_input(action, Some(&json!({"url": "https://evil.test/x"}))).unwrap();
    let built = build(action.request.as_ref().unwrap(), &away, &HashMap::new()).unwrap();
    assert!(http::allowed(&built.url, &get.hosts, Permission::Read, "GET").is_err());
    let mut write = builtin::http_get();
    write["actions"][0]["permission"] = json!("write");
    write["actions"][0]["request"]["method"] = json!("POST");
    assert!(parse(&write, "connector-http-get").unwrap_err().contains("only a read action"));
    let mut no_hosts = builtin::http_get();
    no_hosts["hosts"] = json!([]);
    assert!(parse(&no_hosts, "connector-http-get").unwrap_err().contains("needs hosts"));
}

#[tokio::test]
async fn built_in_mcp_calls_keep_inputs_out_of_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("connectors");
    builtin::seed(&root);
    let kc = fake_keychain(dir.path());
    let net = http::live();
    let c = ctx(&root, &kc, &net);
    let plan = call::prepare(&c, "browser-own", "click", Some(&json!({"ref": "e1"})), 1).await.unwrap();
    assert_eq!(plan.row.input, "{}");
    assert!(!plan.log_result);
    let web = call::prepare(&c, "web-search", "search", Some(&json!({"query": "rust"})), 1).await.unwrap();
    assert!(!web.log_result);
    assert!(call::execute(&c, &web).await.unwrap_err().contains("own web and browser tools"));
    assert!(call::set_trust(&root, "browser-own", true, 1).unwrap_err().contains("built-in"));
    let view = call::view(&root, &kc).await;
    let own = view.iter().find(|v| v["connector"] == "browser-own").unwrap();
    assert_eq!(own["kind"], "mcp");
    assert_eq!(own["status"], "ready");
}
