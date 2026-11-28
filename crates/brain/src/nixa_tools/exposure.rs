use crate::app::App;
use crate::cli_brains::{self, Kind};
use crate::tools::{Args, ToolResult, text};
use crate::util::now_ms;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const API: &str = "api";
const GATEWAY: &str = "nixa_call";
const SHOWN: usize = 12;
const NOTICE: &str = "Configured is what Nixa's code gives each brain. Observed is the list Nixa itself last sent or served to that brain since the Nixa brain started. What a brain program shows its model after its own filters is not seen by Nixa. Read only: nothing was started or changed, apart from the usual tool journal row.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    Provider,
    Bridge,
    Gateway,
    Text,
}

#[derive(Clone)]
struct Seen {
    names: Vec<String>,
    at: u64,
    label: String,
}

type Book = Mutex<HashMap<(PathBuf, String), Seen>>;

fn book() -> &'static Book {
    static BOOK: OnceLock<Book> = OnceLock::new();
    BOOK.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn saw(app: &App, brain: &str, names: Vec<String>, label: &str) {
    let mut book = book().lock().unwrap_or_else(|e| e.into_inner());
    book.insert((app.dir.clone(), brain.to_string()), Seen { names, at: now_ms(), label: label.to_string() });
}

pub fn saw_api(app: &App, names: Vec<String>, model: &str) {
    saw(app, API, names, &format!("sent with an API chat turn that ended without an error, model {model}"));
}

pub fn saw_bridge(app: &App, names: Vec<String>) {
    let (brain, when) = match (cli_brains::running(app), cli_brains::chosen(app)) {
        (Some(kind), _) => (kind.word(), "running"),
        (None, Some(kind)) => (kind.word(), "chosen but not running"),
        (None, None) => ("cli", "unknown"),
    };
    let label = format!("served to nixa-mcp bridge; the bridge shares one token, so it is credited to the CLI brain that was {when} at that moment");
    saw(app, brain, names, &label);
}

fn seen(app: &App, brain: &str) -> Option<Seen> {
    book().lock().unwrap_or_else(|e| e.into_inner()).get(&(app.dir.clone(), brain.to_string())).cloned()
}

fn rule(kind: Option<Kind>) -> &'static str {
    match kind {
        None => "every built-in tool spec is sent with each turn; the provider sees them as sent",
        Some(Kind::Claude) => "--strict-mcp-config and --allowedTools mcp__nixa_brain__*: only Nixa tools can run",
        Some(Kind::Codex) => "Nixa tools are pre-approved; the user's other MCP servers are turned off for the chat brain",
        Some(Kind::Opencode) => "a tool call runs only when its name starts with nixa_brain_",
        Some(Kind::Cursor) => "allowlist Mcp(nixa_brain:*); shell, read, write and web fetch are denied; a guard runs before each tool",
        Some(Kind::Agy) => "Antigravity may load only some Nixa tools as its own; every other one works through nixa_call; permission asks pass only for Nixa tool names",
        Some(Kind::Kimi | Kind::Dim | Kind::Jcode) => "permission asks pass only for Nixa tool names; built-in base tools are disabled on Jcode",
        Some(Kind::Omp | Kind::Omo) => "its own tools and MCP are off; Nixa tools run as text tool calls from its reply",
        Some(Kind::Crush) => "built-in tools are disabled; only the Nixa MCP server is configured and pre-approved",
    }
}

fn route(kind: Kind) -> (Route, &'static str) {
    match kind {
        Kind::Agy => (Route::Gateway, "nixa-mcp bridge with the nixa_call gateway tool"),
        Kind::Codex | Kind::Opencode | Kind::Cursor | Kind::Kimi | Kind::Claude | Kind::Dim | Kind::Jcode | Kind::Crush => (Route::Bridge, "nixa-mcp bridge"),
        Kind::Omp | Kind::Omo => (Route::Text, "text tool calls: the brain writes a calls JSON object and Nixa runs the tools"),
    }
}

fn names_of(app: &App) -> Vec<String> {
    crate::tools::specs(app).into_iter().map(|s| s.name).collect()
}

fn expected(all: &[String], route: Route) -> Vec<String> {
    match route {
        Route::Provider | Route::Bridge | Route::Text => all.to_vec(),
        Route::Gateway => all.iter().cloned().chain(std::iter::once(GATEWAY.to_string())).collect(),
    }
}


fn cut(list: Vec<String>) -> Value {
    let more = list.len().saturating_sub(SHOWN);
    let mut shown: Vec<Value> = list.into_iter().take(SHOWN).map(Value::String).collect();
    if more > 0 {
        shown.push(json!(format!("and {more} more")));
    }
    Value::Array(shown)
}

fn observed(app: &App, brain: &str, route: Route, want: &[String], full: bool) -> Value {
    let Some(seen) = seen(app, brain) else {
        let why = match route {
            Route::Text => "not observed: no text tool call ran since the Nixa brain started",
            Route::Provider => "not observed: no API chat turn ran since the Nixa brain started",
            _ => "not observed: this brain has not asked Nixa for its tool list since the Nixa brain started",
        };
        return json!({"state": "unobserved", "note": why});
    };
    let served = seen.names.clone();
    let missing: Vec<String> = want.iter().filter(|n| *n != GATEWAY && !served.contains(n)).cloned().collect();
    let extra: Vec<String> = served.iter().filter(|n| !want.contains(n)).cloned().collect();
    let state = if missing.is_empty() && extra.is_empty() { "matches" } else { "differs" };
    let mut out = json!({"state": state, "count": served.len(), "at": seen.at, "how": seen.label, "missing": cut(missing), "extra": cut(extra)});
    if route == Route::Gateway {
        out["gateway"] = json!(format!("{GATEWAY} is added by nixa-mcp bridge in gateway mode, so it is not in count or names; Nixa does not see the bridge's own list"));
    }
    if full {
        out["names"] = json!(seen.names);
    }
    out
}

struct Row<'a> {
    brain: &'a str,
    title: &'a str,
    kind: Option<Kind>,
    route: Route,
    how: &'a str,
    current: bool,
}

fn row(app: &App, r: &Row, all: &[String], full: bool) -> Value {
    let want = expected(all, r.route);
    json!({
        "brain": r.brain, "title": r.title, "current": r.current, "path": r.how, "filters": rule(r.kind),
        "configured": want.len(), "observed": observed(app, r.brain, r.route, &want, full),
    })
}

pub fn call(app: &App, args: &Args) -> ToolResult {
    let only = text(args, "brain").map(|b| b.trim().to_lowercase()).filter(|b| !b.is_empty());
    if let Some(word) = &only
        && word != API
        && Kind::parse(word).is_none()
    {
        let known: Vec<&str> = std::iter::once(API).chain(Kind::ALL.iter().map(|k| k.word())).collect();
        return Err(format!("There is no chat brain {word}. Use one of: {}.", known.join(", ")));
    }
    let all = names_of(app);
    let now = cli_brains::chosen(app);
    let full = only.is_some();
    let mut rows = Vec::new();
    if only.as_deref().is_none_or(|w| w == API) {
        let r = Row { brain: API, title: "API brain", kind: None, route: Route::Provider, how: "tool list sent with each API chat turn", current: now.is_none() };
        rows.push(row(app, &r, &all, full));
    }
    for kind in Kind::ALL {
        if only.as_deref().is_some_and(|w| w != kind.word()) {
            continue;
        }
        let (route, how) = route(kind);
        let r = Row { brain: kind.word(), title: kind.title(), kind: Some(kind), route, how, current: now == Some(kind) };
        rows.push(row(app, &r, &all, full));
    }
    Ok(json!({"builtinTools": all.len(), "brains": rows, "notice": NOTICE}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(app: &App, brain: Option<&str>) -> Value {
        let mut args = Args::new();
        if let Some(b) = brain {
            args.insert("brain".into(), json!(b));
        }
        serde_json::from_str(&call(app, &args).unwrap()).unwrap()
    }

    #[test]
    fn every_brain_has_a_path_and_unobserved_paths_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let all = ask(&app, None);
        let brains = all["brains"].as_array().unwrap();
        assert_eq!(brains.len(), Kind::ALL.len() + 1);
        let total = all["builtinTools"].as_u64().unwrap();
        let by = |w: &str| brains.iter().find(|b| b["brain"] == w).unwrap().clone();
        assert_eq!(by(API)["current"], true);
        assert_eq!(by(API)["configured"].as_u64(), Some(total));
        assert_eq!(by("agy")["configured"].as_u64(), Some(total + 1));
        assert_eq!(by("omp")["configured"].as_u64(), Some(total));
        assert_eq!(by("omp")["observed"]["state"], "unobserved");
        assert!(by("omp")["observed"]["note"].as_str().unwrap().contains("no text tool call"));
        assert!(by("codex")["observed"]["note"].as_str().unwrap().contains("has not asked Nixa"));
        let bad: Args = serde_json::from_value(json!({"brain": "nope"})).unwrap();
        assert!(call(&app, &bad).unwrap_err().contains("There is no chat brain nope"));
    }

    #[tokio::test]
    async fn the_bridge_list_and_the_chat_registry_use_the_same_record() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let served = crate::cli_brains::door::served_list(&app);
        assert_eq!(served.as_array().unwrap().len(), names_of(&app).len());
        let call = crate::turns::ToolCall { id: "x".into(), name: "nixa_tools".into(), args: serde_json::from_value(json!({"action": "exposure", "brain": "cli"})).unwrap() };
        let (refused, error) = crate::tools::call(&app, &call).await;
        assert!(error && refused.contains("There is no chat brain cli"), "{refused}");
        let seen = seen(&app, "cli").expect("a bridge list with no CLI brain chosen is kept as cli");
        assert_eq!(seen.names.len(), names_of(&app).len());
        let call = crate::turns::ToolCall { id: "y".into(), name: "nixa_tools".into(), args: serde_json::from_value(json!({"action": "exposure", "brain": "codex"})).unwrap() };
        let (text, error) = crate::tools::call(&app, &call).await;
        assert!(!error, "{text}");
        assert!(text.contains("\"path\":\"nixa-mcp bridge\""), "{text}");
    }

    #[test]
    fn a_served_list_is_compared_with_the_configured_one() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let mut names = names_of(&app);
        saw(&app, "codex", names.clone(), "test");
        assert_eq!(ask(&app, Some("codex"))["brains"][0]["observed"]["state"], "matches");
        saw(&app, "agy", names.clone(), "test");
        assert_eq!(ask(&app, Some("agy"))["brains"][0]["observed"]["state"], "matches", "the gateway tool is added by the bridge");
        let gone = names.pop().unwrap();
        names.push("old_tool".into());
        saw_api(&app, names, "m1");
        let api = &ask(&app, Some("api"))["brains"][0]["observed"];
        assert_eq!(api["state"], "differs");
        assert_eq!(api["missing"], json!([gone]));
        assert_eq!(api["extra"], json!(["old_tool"]));
        assert!(api["how"].as_str().unwrap().contains("model m1"));
        assert!(api["names"].as_array().unwrap().iter().any(|n| n == "old_tool"), "one brain asked shows the whole list");
        assert!(ask(&app, None)["brains"][0]["observed"].get("names").is_none());
        let agy = &ask(&app, Some("agy"))["brains"][0];
        assert_eq!(agy["observed"]["count"].as_u64(), Some(agy["observed"]["names"].as_array().unwrap().len() as u64));
        assert!(agy["observed"]["gateway"].as_str().unwrap().contains("Nixa does not see"));
        assert!(agy["filters"].as_str().unwrap().contains("nixa_call"));
        let other = tempfile::tempdir().unwrap();
        let fresh = App::open(other.path()).unwrap();
        assert_eq!(ask(&fresh, Some("codex"))["brains"][0]["observed"]["state"], "unobserved", "each Nixa data folder keeps its own record");
    }
}
