use super::manifest::{FILE, PREFIX};
use crate::util::atomic_write;
use serde_json::{Map, Value, json};
use std::path::Path;

pub const SEEDED_FILE: &str = "builtin-seeded.txt";
pub const HTTP_GET: &str = "http-get";
pub const WEB_SEARCH: &str = "web-search";
pub const BROWSER_OWN: &str = "browser-own";
pub const BROWSER_MINE: &str = "browser-mine";
pub const WIFI: &str = "wifi";
pub const CAMERA: &str = "cctv";
pub const NAMES: [&str; 6] = [HTTP_GET, WEB_SEARCH, BROWSER_OWN, BROWSER_MINE, WIFI, CAMERA];
pub const DEFAULT_HOSTS: [&str; 1] = ["api.github.com"];

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

pub fn http_get() -> Value {
    json!({
        "connector": HTTP_GET,
        "version": 1,
        "kind": "http",
        "description": "Read-only HTTPS GET to the hosts listed in this file. Add a host to hosts to allow it. Returns the body text.",
        "hosts": DEFAULT_HOSTS,
        "secrets": [],
        "optionalSecrets": [HTTP_GET],
        "actions": [{
            "name": "get",
            "description": "GET one https address on a listed host and return the body text.",
            "permission": "read",
            "reach": "outside",
            "timeoutMs": 15000,
            "inputSchema": object(json!({"url": {"type": "string", "description": "Full https address on a listed host, for example https://api.github.com/zen"}}), &["url"]),
            "request": {
                "method": "GET",
                "url": "{url}",
                "headers": {"Accept": "application/json, text/plain;q=0.8, */*;q=0.5", "Authorization": format!("Bearer {{secret:{HTTP_GET}}}")}
            },
            "resultShape": "the body text"
        }]
    })
}

pub fn web_search() -> Value {
    json!({
        "connector": WEB_SEARCH,
        "version": 1,
        "kind": "mcp",
        "public": true,
        "description": "The web search set up in Settings, and public page text.",
        "secrets": [],
        "optionalSecrets": [],
        "actions": [
            {
                "name": "search",
                "tool": "searxng_web_search",
                "description": "Search the public web with a short public question.",
                "permission": "read",
                "reach": "outside",
                "timeoutMs": 45000,
                "inputSchema": object(json!({"query": {"type": "string"}, "count": {"type": "integer"}}), &["query"]),
                "resultShape": "Untrusted text data"
            },
            {
                "name": "read",
                "tool": "web_url_read",
                "description": "Read one public page as text.",
                "permission": "read",
                "reach": "outside",
                "timeoutMs": 45000,
                "inputSchema": object(json!({"url": {"type": "string"}, "maxLength": {"type": "integer"}, "startChar": {"type": "integer"}}), &["url"]),
                "resultShape": "Untrusted text data"
            }
        ]
    })
}

pub fn step_inputs(action: &str) -> Value {
    let text = json!({"type": "string"});
    let refs = json!({"type": "string", "description": "Element ref from look, for example e12"});
    let mut props = Map::new();
    let mut required: Vec<&str> = Vec::new();
    match action {
        "find" | "key" => {
            props.insert("text".into(), text.clone());
            required.push("text");
        }
        "open" | "tab_new" => {
            props.insert("url".into(), text.clone());
            required.push("url");
        }
        "click" | "hover" => {
            props.insert("ref".into(), refs.clone());
            required.push("ref");
        }
        "type" | "fill" | "choose" => {
            props.insert("ref".into(), refs.clone());
            props.insert("text".into(), text.clone());
            required.extend(["ref", "text"]);
        }
        "dialog" => {
            props.insert("accept".into(), json!({"type": "boolean", "description": "Must be false: a dialog can only be dismissed"}));
            required.push("accept");
        }
        "tab_switch" | "tab_close" => {
            props.insert("index".into(), json!({"type": "integer"}));
            required.push("index");
        }
        _ => {}
    }
    object(Value::Object(props), &required)
}

pub fn browser(name: &str) -> Value {
    let description = if name == BROWSER_MINE {
        "The user's own Chrome tab. Privacy must be off. Steps run at once."
    } else {
        "Nixa's own isolated Chrome. Steps run at once."
    };
    let actions: Vec<Value> = crate::browser::ACTIONS
        .iter()
        .map(|(action, tool, permission)| {
            json!({
                "name": action,
                "tool": tool,
                "description": format!("Browser {}", action.replace('_', " ")),
                "permission": if *permission == crate::browser::Permission::Read { "read" } else { "write" },
                "reach": "outside",
                "timeoutMs": 45000,
                "inputSchema": step_inputs(action),
                "resultShape": "Untrusted text data"
            })
        })
        .collect();
    json!({
        "connector": name,
        "version": 1,
        "kind": "mcp",
        "description": description,
        "secrets": [],
        "optionalSecrets": [],
        "actions": actions
    })
}

pub fn wifi() -> Value {
    let actions: Vec<_> = [("network", "Show the local Wi-Fi interface and connection state"), ("devices", "List network hardware interfaces on this Mac without scanning the network")]
        .into_iter().map(|(name, description)| json!({
            "name": name, "tool": name, "description": description,
            "permission": "read", "reach": "local", "timeoutMs": 5000,
            "inputSchema": object(json!({}), &[]), "resultShape": "Local interface status"
        })).collect();
    json!({"connector": WIFI, "version": 1, "kind": "mcp", "private": true,
        "description": "Inspect Wi-Fi and local network interfaces without probing other devices.", "actions": actions})
}

pub fn camera() -> Value {
    let actions: Vec<_> = [("status", "Check whether the saved camera URL responds."), ("snapshot", "Save one JPEG or PNG frame from the saved camera URL.")]
        .into_iter().map(|(name, description)| json!({
            "name": name, "tool": name, "description": description, "permission": "read",
            "reach": "local", "timeoutMs": 12000, "inputSchema": object(json!({}), &[]),
            "resultShape": "Private camera status or a local image receipt"
        })).collect();
    json!({"connector": CAMERA, "version": 1, "kind": "mcp", "private": true,
        "secrets": [CAMERA], "description": "Set the camera snapshot URL in Settings, Connectors. Chat cannot choose or read the URL.", "actions": actions})
}

pub fn all() -> Vec<(&'static str, Value)> {
    vec![(HTTP_GET, http_get()), (WEB_SEARCH, web_search()), (BROWSER_OWN, browser(BROWSER_OWN)), (BROWSER_MINE, browser(BROWSER_MINE)), (WIFI, wifi()), (CAMERA, camera())]
}

pub fn seeded(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(SEEDED_FILE)).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

pub fn seed(root: &Path) -> Vec<String> {
    let mut done = seeded(root);
    let mut made = Vec::new();
    for (name, raw) in all().into_iter().filter(|(name, _)| *name != HTTP_GET) {
        if done.iter().any(|d| d == name) {
            continue;
        }
        let file = root.join(format!("{PREFIX}{name}")).join(FILE);
        if !file.exists() {
            let text = format!("{}\n", serde_json::to_string_pretty(&raw).unwrap_or_default());
            if let Err(error) = atomic_write(&file, &text) {
                crate::util::log("connector", &format!("built-in connector {name} was not added: {error}"));
                continue;
            }
            made.push(name.to_string());
        }
        done.push(name.to_string());
    }
    if let Err(error) = atomic_write(&root.join(SEEDED_FILE), &format!("{}\n", done.join("\n"))) {
        crate::util::log("connector", &format!("the built-in connector list was not saved: {error}"));
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::manifest::{Kind, Permission, parse};

    #[test]
    fn built_in_manifests_parse() {
        for (name, raw) in all() {
            let manifest = parse(&raw, &format!("{PREFIX}{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(manifest.connector, name);
        }
        let get = parse(&http_get(), "connector-http-get").unwrap();
        assert_eq!(get.kind, Kind::Http);
        assert_eq!(get.hosts, vec!["api.github.com"]);
        assert_eq!(get.actions[0].permission, Permission::Read);
        let own = parse(&browser(BROWSER_OWN), "connector-browser-own").unwrap();
        assert_eq!(own.kind, Kind::Mcp);
        assert_eq!(own.actions.len(), crate::browser::ACTIONS.len());
        assert_eq!(own.actions.iter().find(|a| a.name == "look").unwrap().permission, Permission::Read);
        assert_eq!(own.actions.iter().find(|a| a.name == "click").unwrap().required, vec!["ref"]);
        assert!(parse(&web_search(), "connector-web-search").unwrap().public);
    }

    #[test]
    fn camera_actions_only_use_the_settings_secret() {
        let manifest = parse(&camera(), "connector-cctv").unwrap();
        assert!(manifest.private);
        assert_eq!(manifest.secrets, vec![CAMERA]);
        assert_eq!(manifest.actions.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(), ["status", "snapshot"]);
        for action in manifest.actions {
            assert_eq!(action.permission, Permission::Read);
            assert!(crate::connectors::manifest::check_input(&action, Some(&json!({"url":"https://example.invalid/frame"}))).is_err());
            assert!(crate::connectors::manifest::check_input(&action, None).is_ok());
        }
    }

    #[test]
    fn seeds_once_and_keeps_removals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("connectors");
        let fresh: Vec<&str> = NAMES.iter().copied().filter(|n| *n != HTTP_GET).collect();
        assert_eq!(seed(&root), fresh);
        assert!(!root.join("connector-http-get").exists(), "a fresh install gets no http-get");
        assert!(seed(&root).is_empty());
        std::fs::remove_dir_all(root.join("connector-browser-mine")).unwrap();
        assert!(seed(&root).is_empty());
        assert!(!root.join("connector-browser-mine").exists());
        let mut edited = http_get();
        edited["hosts"] = json!(["api.example.org"]);
        std::fs::create_dir_all(root.join("connector-http-get")).unwrap();
        std::fs::write(root.join("connector-http-get").join(FILE), edited.to_string()).unwrap();
        std::fs::remove_file(root.join(SEEDED_FILE)).unwrap();
        assert_eq!(seed(&root), vec![BROWSER_MINE.to_string()]);
        let kept: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("connector-http-get").join(FILE)).unwrap()).unwrap();
        assert_eq!(parse(&kept, "connector-http-get").unwrap().hosts, vec!["api.example.org"]);
    }
}
