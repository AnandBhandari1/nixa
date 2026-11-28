mod camera;

use super::call::{DATA_PREFIX, MAX_RESULT, PRIVATE_PREFIX, Plan};
use crate::app::App;
use crate::tools::Args;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub const WEB: &str = "web-search";
pub const NAMES: [&str; 5] = ["web-search", "browser-own", "browser-mine", super::builtin::WIFI, super::builtin::CAMERA];
pub const WEB_TOOLS: [&str; 2] = ["searxng_web_search", "web_url_read"];
pub const RENAMES: [(&str, &str); 2] = [("startChar", "start_char"), ("maxLength", "max_length")];

pub fn allowed_tool(connector: &str, action: &str, tool: &str) -> bool {
    match connector {
        WEB => WEB_TOOLS.contains(&tool),
        super::builtin::CAMERA => matches!(action, "status" | "snapshot") && action == tool,
        super::builtin::WIFI => matches!(action, "network" | "devices") && action == tool,
        "browser-own" | "browser-mine" => crate::browser::action_of(action).is_some_and(|(found, _)| found == tool),
        _ => false,
    }
}

pub fn web_args(tool: &str, input: &Map<String, Value>) -> (&'static str, Args) {
    let args: Args = input
        .iter()
        .map(|(key, value)| (RENAMES.iter().find(|(from, _)| *from == key.as_str()).map(|(_, to)| to.to_string()).unwrap_or_else(|| key.clone()), value.clone()))
        .collect();
    let name = if tool == WEB_TOOLS[0] { crate::web::SEARCH } else { crate::web::READ };
    (name, args)
}

pub fn browser_args(connector: &str, action: &str, input: &Map<String, Value>) -> (&'static str, Args) {
    let read = matches!(crate::browser::action_of(action), Some((_, crate::browser::Permission::Read)));
    let mut args: Args = input.clone();
    args.insert("browser".into(), json!(if connector == "browser-mine" { "mine" } else { "own" }));
    let key = if read { "what" } else { "action" };
    args.insert(key.into(), json!(action));
    (if read { crate::browser::READ } else { crate::browser::STEP }, args)
}

pub async fn run(app: &Arc<App>, plan: &Plan) -> Result<String, String> {
    let action = plan.action();
    let connector = plan.manifest.connector.as_str();
    let label = format!("Connector {connector} {}", action.name);
    if !allowed_tool(connector, &action.name, &action.tool) {
        return Err(format!("{label}: this built-in MCP tool is not allowed."));
    }
    let found = if connector == super::builtin::WIFI {
        Some(wifi(app, &action.name).await)
    } else if connector == super::builtin::CAMERA {
        return camera::run(app, plan).await.map(|text| format!("{PRIVATE_PREFIX}{text}"));
    } else if connector == WEB {
        let (name, args) = web_args(&action.tool, &plan.input);
        crate::web::call(app, name, &args).await
    } else {
        let (name, args) = browser_args(connector, &action.name, &plan.input);
        crate::browser::call(app, name, &args).await
    };
    let prefix = if action.public { DATA_PREFIX } else { PRIVATE_PREFIX };
    match found {
        Some(Ok(text)) => Ok(format!("{prefix}{}", crate::util::clip(&text, MAX_RESULT))),
        Some(Err(error)) => Err(format!("{label} failed: {error}")),
        None => Err(format!("{label} has no built-in tool.")),
    }
}

async fn wifi(app: &App, action: &str) -> Result<String, String> {
    if crate::privacy::cloud_brain(app) {
        return Err("Network details are only available to a local brain.".into());
    }
    let ports = networksetup(&["-listallhardwareports"]).await?;
    if action == "devices" {
        return Ok(ports.lines().filter(|line| line.starts_with("Hardware Port: ") || line.starts_with("Device: ")).collect::<Vec<_>>().join("\n"));
    }
    let device = ports.split("\n\n").find_map(|port| {
        let mut lines = port.lines();
        (lines.next()? == "Hardware Port: Wi-Fi").then_some(())?;
        lines.next()?.strip_prefix("Device: ")
    }).ok_or("No Wi-Fi interface was found.")?;
    let state = networksetup(&["-getairportpower", device]).await?;
    Ok(state.trim().to_owned())
}

async fn networksetup(args: &[&str]) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("Wi-Fi inspection is available on macOS only.".into());
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(2), tokio::process::Command::new("/usr/sbin/networksetup")
        .args(args).kill_on_drop(true).output()).await.map_err(|_| "Network settings did not answer in time.")?
        .map_err(|_| "Could not read macOS network settings.")?;
    if !output.status.success() {
        return Err("macOS did not return network settings.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn maps_connector_actions_to_built_in_tools() {
        assert!(allowed_tool("web-search", "search", "searxng_web_search"));
        assert!(!allowed_tool("web-search", "search", "browser_click"));
        assert!(allowed_tool("browser-own", "fill", "browser_type"));
        assert!(!allowed_tool("browser-own", "fill", "browser_click"));
        assert!(!allowed_tool("browser-own", "run_js", "browser_evaluate"));
        assert!(!allowed_tool("other", "look", "browser_snapshot"));
        let (name, args) = web_args("web_url_read", &input(json!({"url": "https://example.org", "maxLength": 100, "startChar": 5})));
        assert_eq!(name, crate::web::READ);
        assert_eq!(Value::Object(args), json!({"url": "https://example.org", "max_length": 100, "start_char": 5}));
        let (name, args) = web_args("searxng_web_search", &input(json!({"query": "rust release", "count": 3})));
        assert_eq!(name, crate::web::SEARCH);
        assert_eq!(args["query"], "rust release");
        let (name, args) = browser_args("browser-mine", "look", &input(json!({})));
        assert_eq!(name, crate::browser::READ);
        assert_eq!(Value::Object(args), json!({"browser": "mine", "what": "look"}));
        let (name, args) = browser_args("browser-own", "click", &input(json!({"ref": "e12"})));
        assert_eq!(name, crate::browser::STEP);
        assert_eq!(Value::Object(args), json!({"browser": "own", "action": "click", "ref": "e12"}));
    }
}
