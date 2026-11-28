use crate::brains::{SavedBrain, id_of_choice};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;
use std::sync::LazyLock;

pub const LOCAL_PROVIDERS: [&str; 10] = ["ollama", "lmstudio", "llamacpp", "llama.cpp", "llama-cpp", "mlx", "vllm", "sglang", "exo", "local"];
pub const MCP_AREA: &str = "MCP servers";
static URL_IN_TEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s"'\]]+"#).expect("url pattern"));
static TOML_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[mcp_servers\.([^\].]+)\]$").expect("toml head"));
static TOML_PAIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_]+)\s*=\s*(.*)$").expect("toml pair"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Reach {
    #[serde(rename = "this Mac")]
    ThisMac,
    #[serde(rename = "local network")]
    LocalNetwork,
    #[serde(rename = "internet")]
    Internet,
    #[serde(rename = "unknown")]
    Unknown,
}

impl Reach {
    pub fn word(self) -> &'static str {
        match self {
            Reach::ThisMac => "this Mac",
            Reach::LocalNetwork => "local network",
            Reach::Internet => "internet",
            Reach::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataPath {
    pub area: String,
    pub name: String,
    pub reach: Reach,
    pub in_use: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub privacy_mode: bool,
    pub leaves: usize,
    pub paths: Vec<DataPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
}

#[derive(Debug, Clone)]
pub struct McpFile {
    pub agent: String,
    pub format: Format,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhoneDoor {
    pub on: bool,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraLooks {
    pub allow: bool,
    pub cloud: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearch {
    pub on: bool,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    pub name: String,
    pub hosts: Vec<String>,
    pub command: bool,
    pub builtin: Option<Builtin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Builtin {
    pub reach: Reach,
    pub in_use: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModel {
    pub on: bool,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeSoap {
    pub url: String,
    pub model: String,
    pub used: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub privacy_on: bool,
    pub choice: String,
    pub opencode_model: Option<String>,
    pub brains: Vec<SavedBrain>,
    pub check_brain: Option<String>,
    pub mcp_files: Vec<McpFile>,
    pub phone: Option<PhoneDoor>,
    pub web: Option<WebSearch>,
    pub browser: Option<bool>,
    pub connectors: Vec<Connector>,
    pub office_soap: Option<OfficeSoap>,
    pub camera: Option<CameraLooks>,
    pub decision: Option<DecisionModel>,
    pub model_scores: Option<bool>,
}

fn rank(reach: Reach) -> u8 {
    match reach {
        Reach::ThisMac => 0,
        Reach::LocalNetwork => 1,
        Reach::Internet => 2,
        Reach::Unknown => 3,
    }
}

pub fn web_path(web: &WebSearch) -> DataPath {
    if !web.on {
        return path("Web", "Web search", Reach::Internet, false, "Off. No search question leaves this Mac.");
    }
    if web.provider == "exa" {
        return path("Web", "Web search (Exa)", Reach::Internet, true, "Each search question goes to Exa with the user's own key.");
    }
    path("Web", "Web search (local SearXNG)", Reach::Internet, true, "SearXNG runs on this Mac and sends each search question to public search engines.")
}

pub fn camera_path(camera: CameraLooks, privacy_on: bool, brains: &[SavedBrain]) -> DataPath {
    const NAME: &str = "Phone camera photos";
    if !camera.allow {
        return path("Phone", NAME, Reach::ThisMac, false, "Off. The Mac asks no phone for pictures.");
    }
    let mut hosts: Vec<String> = Vec::new();
    let mut far = Reach::LocalNetwork;
    if camera.cloud && !privacy_on {
        for brain in brains {
            let (reach, host) = url_reach(&brain.base_url);
            if reach != Reach::ThisMac && !hosts.contains(&host) {
                hosts.push(host);
                if rank(reach) > rank(far) {
                    far = reach;
                }
            }
        }
    }
    if hosts.is_empty() {
        return path("Phone", NAME, Reach::LocalNetwork, true, "Photos come from the phone over the local network and are read by a local vision model on this Mac, then deleted after the answer. Only a text log is kept.");
    }
    let hosts: Vec<&str> = hosts.iter().map(|h| host_or_bad(h)).collect();
    path("Phone", NAME, far, true, format!("Photos come from the phone over the local network and may go to {} when Privacy is off. They are deleted on this Mac after the answer.", hosts.join(", ")))
}

pub fn browser_path(on: bool) -> DataPath {
    if on {
        path("Web", "Browser actions", Reach::Internet, true, "A local Chrome opens public pages; each step runs and goes into the decision journal.")
    } else {
        path("Web", "Browser actions", Reach::Internet, false, "Off. No page is opened.")
    }
}

pub fn soap_path(soap: &OfficeSoap) -> DataPath {
    let (reach, host) = url_reach(&soap.url);
    let detail = format!(
        "The Office view sends agent names, space names and short task titles to the model {} at {}, only while the soap switch is on and Nixa is idle.",
        soap.model,
        host_or_bad(&host)
    );
    path("Office", "Office soap (Ollama)", reach, soap.used, detail)
}

pub fn decision_path(d: &DecisionModel, privacy_on: bool) -> DataPath {
    let name = format!("Decision model ({}/{})", d.provider, d.model);
    if !d.on {
        return path("Decisions", name, Reach::Internet, false, "Off. Nixa asks the usual way.");
    }
    if privacy_on {
        return path("Decisions", name, Reach::Internet, false, "Paused while Privacy mode is on. Nothing is sent; Nixa asks the usual way.");
    }
    path("Decisions", name, Reach::Internet, true, format!("Small decisions go to {} first: chat lines, task details, agent prompts and terminal screens.", d.provider))
}

pub fn model_scores_path(online: bool, privacy_on: bool) -> DataPath {
    const NAME: &str = "Model list and public scores";
    if privacy_on {
        return path("Models", NAME, Reach::Internet, false, "Paused while Privacy mode is on.");
    }
    if !online {
        return path("Models", NAME, Reach::Internet, true, "Every hour the picked agent program is asked for its model list, which it may read from its provider. Public scores are off. No chat or task text is sent.");
    }
    path("Models", NAME, Reach::Internet, true, "Every hour Nixa reads public model scores and asks the picked agent program for its model list. No chat or task text is sent.")
}

pub fn connector_path(c: &Connector) -> DataPath {
    if let Some(b) = &c.builtin {
        return path("Connectors", c.name.clone(), b.reach, b.in_use, b.detail.clone());
    }
    if c.command {
        return path("Connectors", c.name.clone(), Reach::Unknown, true, "Local program the user added. It runs here; what it sends out depends on the program.");
    }
    let reach = c.hosts.iter().map(|h| host_reach(h)).max_by_key(|r| rank(*r)).unwrap_or(Reach::Unknown);
    path("Connectors", c.name.clone(), reach, true, format!("Calls go to {}.", if c.hosts.is_empty() { "no listed host".to_string() } else { c.hosts.join(", ") }))
}

fn path(area: &str, name: impl Into<String>, reach: Reach, in_use: bool, detail: impl Into<String>) -> DataPath {
    DataPath { area: area.to_string(), name: name.into(), reach, in_use, detail: detail.into() }
}

fn ip_reach(ip: IpAddr) -> Reach {
    if ip.is_loopback() || ip.is_unspecified() {
        return Reach::ThisMac;
    }
    let local = match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            v4.is_private() || v4.is_link_local() || (a == 100 && (64..=127).contains(&b))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_reach(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
        }
    };
    if local { Reach::LocalNetwork } else { Reach::Internet }
}

pub fn host_reach(host: &str) -> Reach {
    let lower = host.to_lowercase();
    let name = lower.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.');
    if name.is_empty() {
        return Reach::Unknown;
    }
    if let Ok(ip) = name.parse::<IpAddr>() {
        return ip_reach(ip);
    }
    if name == "localhost" || name.ends_with(".localhost") {
        return Reach::ThisMac;
    }
    if name.ends_with(".local") || name.ends_with(".lan") {
        return Reach::LocalNetwork;
    }
    Reach::Internet
}

pub fn url_reach(url: &str) -> (Reach, String) {
    match reqwest::Url::parse(url.trim()).ok().and_then(|u| u.host_str().map(str::to_string)) {
        Some(host) if !host.is_empty() => (host_reach(&host), host),
        _ => (Reach::Unknown, String::new()),
    }
}

fn host_or_bad(host: &str) -> &str {
    if host.is_empty() { "a bad URL" } else { host }
}

fn cli_brain(word: &str) -> Option<&'static str> {
    match word {
        "codex" => Some("Codex (OpenAI)"),
        "agy" | "antigravity" => Some("Antigravity (Google)"),
        "cursor" => Some("Cursor"),
        "kimi" => Some("Kimi (Moonshot)"),
        "claude" => Some("Claude Code (Anthropic)"),
        _ => crate::cli_brains::Kind::parse(word).filter(|k| *k != crate::cli_brains::Kind::Opencode).map(crate::cli_brains::Kind::title),
    }
}

pub fn brain_path(area: &str, choice: &str, inputs: &Inputs, in_use: bool) -> DataPath {
    let choice = choice.trim();
    if choice.is_empty() {
        return path(area, "none", Reach::Unknown, in_use, "No brain is picked yet.");
    }
    if let Some(name) = cli_brain(choice) {
        return path(area, name, Reach::Internet, in_use, format!("The {choice} program sends the brain's prompts, chat and tool results to its provider."));
    }
    if choice == "opencode" {
        let model = inputs.opencode_model.clone().unwrap_or_default();
        let provider = model.split_once('/').map(|(p, _)| p.to_string()).unwrap_or_else(|| "opencode".into());
        let local = LOCAL_PROVIDERS.contains(&provider.to_lowercase().as_str());
        let name = if model.is_empty() { "OpenCode".to_string() } else { format!("OpenCode ({model})") };
        return if local {
            path(area, name, Reach::ThisMac, in_use, format!("Provider {provider} runs on this Mac."))
        } else {
            path(area, name, Reach::Internet, in_use, format!("Provider {provider}: prompts, chat and tool results go to it."))
        };
    }
    if let Some(id) = id_of_choice(choice) {
        let Some(saved) = inputs.brains.iter().find(|b| b.id == id) else {
            return path(area, choice, Reach::Unknown, in_use, "This saved brain is not in the list any more.");
        };
        let (reach, host) = url_reach(&saved.base_url);
        return path(area, saved.name.clone(), reach, in_use, format!("Saved {} brain at {}.", saved.kind.title(), host_or_bad(&host)));
    }
    path(area, choice, Reach::Unknown, in_use, "Unknown brain kind.")
}

#[derive(Debug, Default)]
struct Server {
    name: String,
    url: String,
    command: String,
    args: String,
    enabled: bool,
}

fn unquote(text: &str) -> String {
    let text = text.strip_prefix('"').unwrap_or(text);
    text.strip_suffix('"').unwrap_or(text).to_string()
}

fn toml_servers(source: &str) -> Vec<Server> {
    let mut servers: Vec<Server> = Vec::new();
    let mut open = false;
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(head) = TOML_HEAD.captures(line) {
            servers.push(Server { name: unquote(&head[1]), enabled: true, ..Server::default() });
            open = true;
            continue;
        }
        if line.starts_with('[') {
            open = false;
            continue;
        }
        let (true, Some(current)) = (open, servers.last_mut()) else { continue };
        let Some(pair) = TOML_PAIR.captures(line) else { continue };
        let value = unquote(&pair[2]);
        match &pair[1] {
            "url" => current.url = value,
            "command" => current.command = value,
            "args" => current.args = value,
            "enabled" => current.enabled = value != "false",
            _ => {}
        }
    }
    servers
}

fn base_name(text: &str) -> String {
    Path::new(text).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "?".into())
}

fn mcp_path(agent: &str, server: &Server) -> DataPath {
    let name = format!("{} ({agent})", server.name);
    if !server.url.is_empty() {
        let (reach, host) = url_reach(&server.url);
        return path(MCP_AREA, name, reach, server.enabled, format!("Server at {}. Tool calls and their answers go there.", host_or_bad(&host)));
    }
    if let Some(linked) = URL_IN_TEXT.find(&server.args) {
        let (reach, host) = url_reach(linked.as_str());
        return path(MCP_AREA, name, reach, server.enabled, format!("Local program {} that connects to {}.", base_name(&server.command), host_or_bad(&host)));
    }
    path(MCP_AREA, name, Reach::Unknown, server.enabled, format!("Local program {}. It runs here; what it sends out depends on the program.", base_name(&server.command)))
}

fn text_at(map: &Map<String, Value>, key: &str) -> String {
    map.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string()
}

fn json_servers(agent: &str, servers: Option<&Value>) -> Vec<DataPath> {
    let Some(Value::Object(servers)) = servers else { return Vec::new() };
    servers
        .iter()
        .map(|(name, raw)| {
            let empty = Map::new();
            let row = raw.as_object().unwrap_or(&empty);
            let url = Some(text_at(row, "url")).filter(|u| !u.is_empty()).unwrap_or_else(|| text_at(row, "serverUrl"));
            let args = row.get("args").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default();
            let enabled = row.get("disabled") != Some(&Value::Bool(true)) && row.get("enabled") != Some(&Value::Bool(false));
            mcp_path(agent, &Server { name: name.clone(), url, command: text_at(row, "command"), args, enabled })
        })
        .collect()
}

pub fn mcp_paths(files: &[McpFile]) -> Vec<DataPath> {
    let mut paths = Vec::new();
    for source in files {
        let Some(text) = &source.text else { continue };
        if source.format == Format::Toml {
            paths.extend(toml_servers(text).iter().map(|s| mcp_path(&source.agent, s)));
            continue;
        }
        let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(text) else {
            paths.push(path(MCP_AREA, source.agent.clone(), Reach::Unknown, false, "Its settings file could not be read."));
            continue;
        };
        paths.extend(json_servers(&source.agent, parsed.get("mcpServers")));
        if let Some(Value::Object(projects)) = parsed.get("projects") {
            for (folder, entry) in projects {
                paths.extend(json_servers(&format!("{}, project {}", source.agent, base_name(folder)), entry.get("mcpServers")));
            }
        }
    }
    let mut seen = HashSet::new();
    paths.retain(|p| seen.insert(format!("{}|{}", p.name, p.detail)));
    paths
}

pub fn mcp_files(home: &Path) -> Vec<McpFile> {
    let read = |p: &Path| std::fs::read_to_string(p).ok();
    vec![McpFile { agent: "Codex".into(), format: Format::Toml, text: read(&home.join(".codex").join("config.toml")) }]
}

pub fn report(inputs: &Inputs) -> Report {
    let chosen = id_of_choice(&inputs.choice).map(str::to_string);
    let mut paths = vec![brain_path("Chat brain", &inputs.choice, inputs, true)];
    if let Some(check) = &inputs.check_brain {
        paths.push(brain_path("Message check brain", check, inputs, true));
    }
    for brain in &inputs.brains {
        let (reach, host) = url_reach(&brain.base_url);
        paths.push(path("Saved brains", brain.name.clone(), reach, chosen.as_deref() == Some(brain.id.as_str()), format!("{} brain at {}.", brain.kind.title(), host_or_bad(&host))));
    }
    paths.push(path(MCP_AREA, "nixa-mcp (Nixa's own)", Reach::ThisMac, true, "Nixa's terminal and app tools, run on this Mac."));
    paths.extend(mcp_paths(&inputs.mcp_files));
    if let Some(phone) = inputs.phone {
        paths.push(if phone.on {
            path("Phone", "Phone door", Reach::LocalNetwork, true, format!("HTTPS on the Wi-Fi network, port {}. Chat, replies and voice go to paired phones.", phone.port))
        } else {
            path("Phone", "Phone door", Reach::LocalNetwork, false, "Off. Nothing goes to a phone.")
        });
    }
    if let Some(camera) = inputs.camera {
        paths.push(camera_path(camera, inputs.privacy_on, &inputs.brains));
    }
    if let Some(web) = &inputs.web {
        paths.push(web_path(web));
    }
    if let Some(on) = inputs.browser {
        paths.push(browser_path(on));
    }
    paths.extend(inputs.connectors.iter().map(connector_path));
    if let Some(soap) = &inputs.office_soap {
        paths.push(soap_path(soap));
    }
    if let Some(decision) = &inputs.decision {
        paths.push(decision_path(decision, inputs.privacy_on));
    }
    if let Some(online) = inputs.model_scores {
        paths.push(model_scores_path(online, inputs.privacy_on));
    }
    paths.push(path("Voice", "Speech to text and speech", Reach::ThisMac, true, "Hearing and speaking run on local models."));
    let leaves = paths.iter().filter(|p| p.in_use && p.reach != Reach::ThisMac).count();
    Report { privacy_mode: inputs.privacy_on, leaves, paths }
}

pub fn text(report: &Report) -> String {
    let mut areas: Vec<&str> = Vec::new();
    for p in &report.paths {
        if !areas.contains(&p.area.as_str()) {
            areas.push(&p.area);
        }
    }
    let mut lines = vec![
        format!("Privacy check. Read from settings only, no content. Privacy mode is {}.", if report.privacy_mode { "on" } else { "off" }),
        format!("{} paths in use can send data off this Mac (internet, local network or unknown).", report.leaves),
    ];
    for area in areas {
        lines.push(String::new());
        lines.push(area.to_string());
        for p in report.paths.iter().filter(|p| p.area == area) {
            lines.push(format!("  - {}: {}{}. {}", p.name, p.reach.word(), if p.in_use { "" } else { ", not in use" }, p.detail));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Kind;

    fn saved(id: &str, name: &str, url: &str) -> SavedBrain {
        SavedBrain { id: id.into(), name: name.into(), kind: Kind::Openai, base_url: url.into(), model: "m".into(), effort: None, key_env: None, added_at: 0 }
    }

    #[test]
    fn web_browser_and_connectors_are_listed() {
        let off = Inputs { web: Some(WebSearch { on: false, provider: "exa".into() }), browser: Some(false), ..Inputs::default() };
        let report_off = report(&off);
        let web = report_off.paths.iter().find(|p| p.name == "Web search").unwrap();
        assert!(!web.in_use);
        assert!(report_off.paths.iter().any(|p| p.name == "Browser actions" && !p.in_use));
        let on = Inputs {
            web: Some(WebSearch { on: true, provider: "searxng".into() }),
            browser: Some(true),
            connectors: vec![
                Connector { name: "shop".into(), hosts: vec!["127.0.0.1".into(), "api.shop.test".into()], command: false, builtin: None },
                Connector { name: "tool".into(), hosts: Vec::new(), command: true, builtin: None },
            ],
            ..Inputs::default()
        };
        let report_on = report(&on);
        let find = |name: &str| report_on.paths.iter().find(|p| p.name == name).cloned().unwrap_or_else(|| panic!("{name}"));
        assert_eq!(find("Web search (local SearXNG)").reach, Reach::Internet);
        assert!(find("Browser actions").in_use);
        assert_eq!(find("shop").reach, Reach::Internet);
        assert_eq!(find("tool").reach, Reach::Unknown);
        assert!(report_on.leaves > report_off.leaves);
        assert_eq!(web_path(&WebSearch { on: true, provider: "exa".into() }).name, "Web search (Exa)");
    }

    #[test]
    fn office_soap_is_listed() {
        let local = OfficeSoap { url: "http://127.0.0.1:11434".into(), model: "m".into(), used: true };
        let found = report(&Inputs { office_soap: Some(local), ..Inputs::default() });
        let soap = found.paths.iter().find(|p| p.area == "Office").unwrap();
        assert_eq!(soap.reach, Reach::ThisMac);
        assert!(soap.in_use && soap.detail.contains("task titles"));
        let far = soap_path(&OfficeSoap { url: "https://llm.example.com".into(), model: "m".into(), used: false });
        assert_eq!(far.reach, Reach::Internet);
        assert!(!far.in_use);
    }

    #[test]
    fn hosts_are_sorted_by_reach() {
        for (host, reach) in [
            ("localhost", Reach::ThisMac),
            ("a.localhost", Reach::ThisMac),
            ("127.0.0.2", Reach::ThisMac),
            ("[::1]", Reach::ThisMac),
            ("10.1.2.3", Reach::LocalNetwork),
            ("172.20.0.1", Reach::LocalNetwork),
            ("172.40.0.1", Reach::Internet),
            ("192.168.1.9", Reach::LocalNetwork),
            ("100.64.0.1", Reach::LocalNetwork),
            ("mac.local", Reach::LocalNetwork),
            ("fd12::1", Reach::LocalNetwork),
            ("fe80::1", Reach::LocalNetwork),
            ("0.0.0.0", Reach::ThisMac),
            ("::ffff:127.0.0.1", Reach::ThisMac),
            ("127.0.0.1.nip.io", Reach::Internet),
            ("127.example.com", Reach::Internet),
            ("10.example.com", Reach::Internet),
            ("fd.example.com", Reach::Internet),
            ("fe80.example.com", Reach::Internet),
            ("8.8.8.8", Reach::Internet),
            ("api.openai.com", Reach::Internet),
            ("", Reach::Unknown),
        ] {
            assert_eq!(host_reach(host), reach, "{host}");
        }
        assert_eq!(url_reach("http://127.0.0.1:11434/v1"), (Reach::ThisMac, "127.0.0.1".into()));
        assert_eq!(url_reach("not a url"), (Reach::Unknown, String::new()));
        assert_eq!(url_reach("https://127.0.0.1.nip.io/v1"), (Reach::Internet, "127.0.0.1.nip.io".into()));
        assert_eq!(url_reach("http://[::1]:8080/"), (Reach::ThisMac, "[::1]".into()));
    }

    #[test]
    fn brains_mcp_files_and_text() {
        let claude = r#"{"mcpServers": {"web": {"url": "https://mcp.example.com/x"}, "local": {"command": "/usr/bin/tool", "args": ["--to", "http://192.168.1.4:9000"]}, "off": {"command": "x", "disabled": true}}, "projects": {"/Users/a/site": {"mcpServers": {"db": {"command": "/opt/db"}}}}}"#;
        let codex = "[mcp_servers.search]\nurl = \"https://search.example.org\"\n[mcp_servers.\"pad\"]\ncommand = \"pad\"\nenabled = false\n[other]\nurl = \"https://skip.example\"\n";
        let inputs = Inputs {
            privacy_on: true,
            choice: "api:0123abcd".into(),
            brains: vec![saved("0123abcd", "Home", "http://localhost:11434/v1"), saved("89abcdef", "Cloud", "https://api.example.com")],
            check_brain: Some("api:89abcdef".into()),
            mcp_files: vec![
                McpFile { agent: "Claude Code".into(), format: Format::Json, text: Some(claude.into()) },
                McpFile { agent: "Codex".into(), format: Format::Toml, text: Some(codex.into()) },
                McpFile { agent: "Cursor".into(), format: Format::Json, text: Some("{bad".into()) },
                McpFile { agent: "None".into(), format: Format::Json, text: None },
            ],
            phone: Some(PhoneDoor { on: true, port: 7430 }),
            ..Inputs::default()
        };
        let report = report(&inputs);
        let find = |name: &str| report.paths.iter().find(|p| p.name == name).cloned().unwrap_or_else(|| panic!("{name}"));
        assert_eq!(report.paths[0].area, "Chat brain");
        assert_eq!(report.paths[0].reach, Reach::ThisMac);
        assert_eq!(report.paths[1].area, "Message check brain");
        assert_eq!(report.paths[1].reach, Reach::Internet);
        let saved = |name: &str| report.paths.iter().find(|p| p.area == "Saved brains" && p.name == name).map(|p| p.in_use);
        assert_eq!(saved("Home"), Some(true));
        assert_eq!(saved("Cloud"), Some(false));
        assert_eq!(find("web (Claude Code)").reach, Reach::Internet);
        assert_eq!(find("local (Claude Code)").detail, "Local program tool that connects to 192.168.1.4.");
        assert!(!find("off (Claude Code)").in_use);
        assert_eq!(find("db (Claude Code, project site)").reach, Reach::Unknown);
        assert_eq!(find("search (Codex)").reach, Reach::Internet);
        assert!(!find("pad (Codex)").in_use);
        assert_eq!(find("Cursor").detail, "Its settings file could not be read.");
        assert!(report.paths.iter().all(|p| !p.detail.contains("skip.example")));
        assert_eq!(find("Phone door").reach, Reach::LocalNetwork);
        assert_eq!(report.leaves, 6);
        let text = text(&report);
        assert!(text.starts_with("Privacy check. Read from settings only, no content. Privacy mode is on.\n6 paths in use"));
        assert!(text.contains("\nSaved brains\n  - Home: this Mac. OpenAI brain at localhost.\n"), "{text}");
        assert!(text.contains("  - Cloud: internet, not in use. OpenAI brain at api.example.com."), "{text}");
    }

    #[test]
    fn cli_brains_and_missing_brains() {
        let inputs = Inputs { opencode_model: Some("ollama/qwen".into()), ..Inputs::default() };
        assert_eq!(brain_path("Chat brain", "codex", &inputs, true).reach, Reach::Internet);
        assert_eq!(brain_path("Chat brain", "kimi", &inputs, true).name, "Kimi (Moonshot)");
        let open = brain_path("Chat brain", "opencode", &inputs, true);
        assert_eq!((open.name.as_str(), open.reach), ("OpenCode (ollama/qwen)", Reach::ThisMac));
        assert_eq!(brain_path("Chat brain", "api:00000000", &inputs, true).detail, "This saved brain is not in the list any more.");
        assert_eq!(brain_path("Chat brain", "", &inputs, true).name, "none");
        assert_eq!(brain_path("Chat brain", "other", &inputs, true).detail, "Unknown brain kind.");
        let report = report(&Inputs { choice: "codex".into(), ..Inputs::default() });
        assert_eq!(report.leaves, 1);
        assert!(!report.paths.iter().any(|p| p.area == "Phone"));
    }

    #[test]
    fn every_cli_brain_and_new_path_is_named() {
        let inputs = Inputs::default();
        for word in ["claude", "crush", "omp", "omo", "jcode", "dim", "agy", "cursor", "kimi", "codex"] {
            let p = brain_path("main", word, &inputs, true);
            assert_eq!(p.reach, Reach::Internet, "{word}");
            assert_ne!(p.detail, "Unknown brain kind.", "{word}");
        }
        assert_eq!(brain_path("main", "claude", &inputs, true).name, "Claude Code (Anthropic)");
        let on = DecisionModel { on: true, provider: "opencode-go".into(), model: "jev-1.14".into() };
        assert!(decision_path(&on, false).in_use);
        assert!(!decision_path(&on, true).in_use);
        assert!(!model_scores_path(true, true).in_use);
        assert!(model_scores_path(false, false).in_use);
        let built = Connector { name: "web-search".into(), hosts: Vec::new(), command: false, builtin: Some(Builtin { reach: Reach::Internet, in_use: false, detail: "x".into() }) };
        assert_eq!((connector_path(&built).reach, connector_path(&built).in_use), (Reach::Internet, false));
        let full = report(&Inputs { decision: Some(on), model_scores: Some(true), ..Inputs::default() });
        assert!(full.paths.iter().any(|p| p.area == "Decisions" && p.in_use));
        assert!(full.paths.iter().any(|p| p.area == "Models" && p.in_use));
    }

    #[test]
    fn other_agents_private_config_is_not_read() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".claude.json"), r#"{"mcpServers": {"x": {"url": "https://a.example"}}}"#).unwrap();
        std::fs::create_dir_all(home.path().join(".cursor")).unwrap();
        std::fs::write(home.path().join(".cursor").join("mcp.json"), "{}").unwrap();
        let files = mcp_files(home.path());
        assert!(files.iter().all(|f| f.agent != "Claude Code" && f.agent != "Cursor"), "{files:?}");
    }

    #[test]
    fn ollama_on_a_local_url_is_this_mac() {
        let brain = SavedBrain { kind: Kind::Ollama, ..saved("0a0b0c0d", "Ollama", Kind::Ollama.default_url()) };
        let inputs = Inputs { choice: "api:0a0b0c0d".into(), brains: vec![brain], ..Inputs::default() };
        let chosen = brain_path("Chat brain", &inputs.choice, &inputs, true);
        assert_eq!((chosen.name.as_str(), chosen.reach), ("Ollama", Reach::ThisMac));
        assert_eq!(chosen.detail, "Saved Ollama brain at 127.0.0.1.");
    }
}
