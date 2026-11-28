use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub const PREFIX: &str = "connector-";
pub const FILE: &str = "connector.json";
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 120_000;
pub const MAX_TEXT: usize = 10_000;
pub const DEFAULT_PROGRAM: &str = "run";
pub const METHODS: [&str; 6] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE"];

static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9-]{1,30}$").expect("name"));
static ACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]{0,40}$").expect("action"));
static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]{0,40}$").expect("field"));
static PROGRAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_.-]{0,60}$").expect("program"));
static HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$").expect("host"));
static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9-]{1,64}$").expect("header"));
pub static SLOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(secret:)?([A-Za-z_][A-Za-z0-9_-]*)\}").expect("slot"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Http,
    Command,
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Local,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    String,
    Number,
    Integer,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Returns {
    Text,
    Image,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub kind: FieldType,
    pub description: String,
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
    pub pick: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub name: String,
    pub tool: String,
    pub description: String,
    pub permission: Permission,
    pub reach: Reach,
    pub stop: bool,
    pub timeout_ms: u64,
    pub properties: BTreeMap<String, Field>,
    pub required: Vec<String>,
    pub fixed: Map<String, Value>,
    pub result_shape: String,
    pub returns: Returns,
    pub private: bool,
    pub public: bool,
    pub request: Option<Request>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub connector: String,
    pub version: u64,
    pub description: String,
    pub kind: Kind,
    pub program: String,
    pub hosts: Vec<String>,
    pub secrets: Vec<String>,
    pub optional_secrets: Vec<String>,
    pub private: bool,
    pub public: bool,
    pub actions: Vec<Action>,
}

impl Permission {
    pub fn word(self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Write => "write",
        }
    }
}

impl Reach {
    pub fn word(self) -> &'static str {
        match self {
            Reach::Local => "local",
            Reach::Outside => "outside",
        }
    }
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Http => "http",
            Kind::Command => "command",
            Kind::Mcp => "mcp",
        }
    }
}

impl FieldType {
    pub fn word(self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Number => "number",
            FieldType::Integer => "integer",
            FieldType::Boolean => "boolean",
        }
    }
}

pub fn valid_name(name: &str) -> bool {
    NAME.is_match(name)
}

impl Manifest {
    pub fn all_secrets(&self) -> Vec<String> {
        self.secrets.iter().chain(self.optional_secrets.iter()).cloned().collect()
    }
}

fn record(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn text(value: Option<&Value>, label: &str) -> Result<String, String> {
    match value.and_then(Value::as_str).map(str::trim) {
        Some(t) if !t.is_empty() => Ok(t.to_string()),
        _ => Err(format!("{label} must be a non-empty text.")),
    }
}

fn list<'a>(value: Option<&'a Value>, label: &str) -> Result<Vec<&'a Value>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.iter().collect()),
        Some(_) => Err(format!("{label} must be a list.")),
    }
}

fn flag(value: Option<&Value>, fallback: bool, label: &str) -> Result<bool, String> {
    match value {
        None | Some(Value::Null) => Ok(fallback),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{label} must be true or false.")),
    }
}

fn entries(value: Option<&Value>, label: &str, connector: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for item in list(value, label)? {
        let entry = text(Some(item), &format!("{label} entry"))?;
        if !NAME.is_match(&entry) || (entry != connector && !entry.starts_with(&format!("{connector}-"))) {
            return Err(format!("{label} entry \"{entry}\" must be \"{connector}\" or start with \"{connector}-\"."));
        }
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    Ok(out)
}

fn strings(value: Option<&Value>, label: &str) -> Result<Vec<(String, String)>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())).ok_or_else(|| format!("{label} \"{k}\" must be a text.")))
            .collect(),
        Some(_) => Err(format!("{label} must be an object of texts.")),
    }
}

pub fn slots(template: &str) -> Vec<(bool, String)> {
    SLOT.captures_iter(template).map(|c| (c.get(1).is_some(), c[2].to_string())).collect()
}

fn value_slots(value: &Value, out: &mut Vec<(bool, String)>) {
    match value {
        Value::String(s) => out.extend(slots(s)),
        Value::Array(items) => items.iter().for_each(|v| value_slots(v, out)),
        Value::Object(map) => map.values().for_each(|v| value_slots(v, out)),
        _ => {}
    }
}

pub fn whole_slot(template: &str) -> Option<String> {
    SLOT.captures(template).filter(|c| c.get(0).is_some_and(|m| m.as_str() == template) && c.get(1).is_none()).map(|c| c[2].to_string())
}

fn authority(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(&rest[..end])
}

struct Names<'a> {
    inputs: Vec<&'a str>,
    secrets: &'a [String],
}

fn check_slots(found: &[(bool, String)], names: &Names, where_: &str) -> Result<(), String> {
    for (secret, name) in found {
        if *secret && !names.secrets.contains(name) {
            return Err(format!("{where_}: the secret \"{name}\" is not in secrets or optionalSecrets."));
        }
        if !*secret && !names.inputs.contains(&name.as_str()) {
            return Err(format!("{where_}: \"{{{name}}}\" is not an input or a fixed argument."));
        }
    }
    Ok(())
}

fn parse_request(value: Option<&Value>, permission: Permission, hosts: &[String], names: &Names, where_: &str) -> Result<Request, String> {
    let item = value.and_then(record).ok_or_else(|| format!("{where_}: an http action needs a request object."))?;
    let method = item.get("method").and_then(Value::as_str).unwrap_or("GET").trim().to_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(format!("{where_}: method must be one of {}.", METHODS.join(", ")));
    }
    if permission == Permission::Read && method != "GET" && method != "HEAD" {
        return Err(format!("{where_}: a read action can only use GET or HEAD."));
    }
    let url = text(item.get("url"), &format!("{where_}: url"))?;
    if whole_slot(&url).is_some() {
        if permission != Permission::Read {
            return Err(format!("{where_}: only a read action can take its whole url from an input."));
        }
        if hosts.is_empty() {
            return Err(format!("{where_}: a url from an input needs hosts, the only hosts it may reach."));
        }
    } else {
        let host_part = authority(&url).ok_or_else(|| format!("{where_}: url must start with https://."))?;
        if host_part.is_empty() || host_part.contains(['{', '@', ':']) {
            return Err(format!("{where_}: the url host must be a plain host name, with no port, login or template."));
        }
        let host = host_part.to_lowercase();
        if !hosts.contains(&host) {
            return Err(format!("{where_}: the url host {host} is not in hosts."));
        }
    }
    let query = strings(item.get("query"), &format!("{where_}: query"))?;
    let headers = strings(item.get("headers"), &format!("{where_}: headers"))?;
    for (name, _) in &headers {
        if !HEADER.is_match(name) || matches!(name.to_ascii_lowercase().as_str(), "host" | "content-length" | "transfer-encoding" | "connection") {
            return Err(format!("{where_}: the header \"{name}\" is not allowed."));
        }
    }
    let body = item.get("body").filter(|v| !v.is_null()).cloned();
    if body.is_some() && (method == "GET" || method == "HEAD") {
        return Err(format!("{where_}: a GET or HEAD request has no body."));
    }
    let pick = match item.get("pick") {
        None | Some(Value::Null) => None,
        Some(Value::String(p)) if p.starts_with('/') => Some(p.clone()),
        Some(_) => return Err(format!("{where_}: pick must be a JSON pointer such as /items.")),
    };
    let mut found = slots(&url);
    for (_, v) in query.iter().chain(headers.iter()) {
        found.extend(slots(v));
    }
    if let Some(body) = &body {
        value_slots(body, &mut found);
    }
    check_slots(&found, names, where_)?;
    Ok(Request { method, url, query, headers, body, pick })
}

struct Shared<'a> {
    kind: Kind,
    hosts: &'a [String],
    secrets: &'a [String],
    private: bool,
    public: bool,
}

fn parse_action(value: &Value, shared: &Shared) -> Result<Action, String> {
    let item = record(value).ok_or("Each action must be an object.")?;
    let name = text(item.get("name"), "action name")?;
    if !ACTION.is_match(&name) {
        return Err(format!("Action name \"{name}\" is not allowed. Use lowercase letters, digits and _."));
    }
    let where_ = format!("Action {name}");
    let permission = match item.get("permission").and_then(Value::as_str) {
        Some("read") => Permission::Read,
        Some("write") => Permission::Write,
        _ => return Err(format!("{where_}: permission must be \"read\" or \"write\".")),
    };
    let reach = match item.get("reach").and_then(Value::as_str) {
        Some("local") => Reach::Local,
        Some("outside") => Reach::Outside,
        _ => return Err(format!("{where_}: reach must be \"local\" or \"outside\".")),
    };
    let empty = Map::new();
    let schema = match item.get("inputSchema") {
        None | Some(Value::Null) => &empty,
        Some(v) => {
            let s = record(v).ok_or_else(|| format!("{where_}: inputSchema must be an object schema."))?;
            if s.get("type").and_then(Value::as_str) != Some("object") {
                return Err(format!("{where_}: inputSchema must be an object schema."));
            }
            s
        }
    };
    let mut properties = BTreeMap::new();
    if let Some(given) = schema.get("properties").and_then(Value::as_object) {
        for (key, raw) in given {
            if !FIELD.is_match(key) {
                return Err(format!("{where_}: input name \"{key}\" is not allowed. Use letters, digits and _."));
            }
            let field = record(raw);
            let kind = match field.and_then(|f| f.get("type")).and_then(Value::as_str) {
                Some("string") => FieldType::String,
                Some("number") => FieldType::Number,
                Some("integer") => FieldType::Integer,
                Some("boolean") => FieldType::Boolean,
                _ => return Err(format!("{where_}: input \"{key}\" needs a type: string, number, integer, boolean.")),
            };
            let description = field.and_then(|f| f.get("description")).and_then(Value::as_str).unwrap_or_default().to_string();
            let choices = field
                .and_then(|f| f.get("enum"))
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            properties.insert(key.clone(), Field { kind, description, choices });
        }
    }
    let mut required = Vec::new();
    for key in list(schema.get("required"), &format!("{where_}: required"))? {
        let key = text(Some(key), &format!("{where_}: required entry"))?;
        if !properties.contains_key(&key) {
            return Err(format!("{where_}: required input \"{key}\" is not in properties."));
        }
        required.push(key);
    }
    let timeout_ms = match item.get("timeoutMs") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_MS,
        Some(v) => v.as_u64().filter(|t| (MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(t)).ok_or_else(|| format!("{where_}: timeoutMs must be a whole number from {MIN_TIMEOUT_MS} to {MAX_TIMEOUT_MS}."))?,
    };
    let fixed = match item.get("fixedArgs") {
        None | Some(Value::Null) => Map::new(),
        Some(v) => record(v).cloned().ok_or_else(|| format!("{where_}: fixedArgs must be an object."))?,
    };
    for key in fixed.keys() {
        if properties.contains_key(key) {
            return Err(format!("{where_}: fixedArgs \"{key}\" is also an input."));
        }
    }
    let tool = match item.get("tool") {
        None | Some(Value::Null) => name.clone(),
        v => text(v, &format!("{where_}: tool"))?,
    };
    if !ACTION.is_match(&tool) {
        return Err(format!("{where_}: tool \"{tool}\" is not allowed."));
    }
    let returns = match item.get("returns").and_then(Value::as_str) {
        None => Returns::Text,
        Some("text") => Returns::Text,
        Some("image") => Returns::Image,
        Some(_) => return Err(format!("{where_}: returns must be \"text\" or \"image\".")),
    };
    let private = flag(item.get("private"), shared.private, &format!("{where_}: private"))?;
    let public = flag(item.get("public"), shared.public && !private, &format!("{where_}: public"))?;
    if private && public {
        return Err(format!("{where_}: an action cannot be both private and public."));
    }
    let names = Names { inputs: properties.keys().chain(fixed.keys()).map(String::as_str).collect(), secrets: shared.secrets };
    let request = match shared.kind {
        Kind::Http => {
            if reach == Reach::Local {
                return Err(format!("{where_}: an http action goes to the internet, so reach must be \"outside\"."));
            }
            if returns == Returns::Image {
                return Err(format!("{where_}: an http action returns text. Use a command connector for images."));
            }
            Some(parse_request(item.get("request"), permission, shared.hosts, &names, &where_)?)
        }
        Kind::Command => {
            if item.get("request").is_some_and(|v| !v.is_null()) {
                return Err(format!("{where_}: request is only for http connectors."));
            }
            None
        }
        Kind::Mcp => {
            if item.get("request").is_some_and(|v| !v.is_null()) {
                return Err(format!("{where_}: request is only for http connectors."));
            }
            if returns == Returns::Image {
                return Err(format!("{where_}: a built-in MCP action returns text."));
            }
            None
        }
    };
    Ok(Action {
        name,
        tool,
        description: text(item.get("description"), &format!("{where_}: description"))?,
        permission,
        reach,
        stop: item.get("stop").and_then(Value::as_bool).unwrap_or(false),
        timeout_ms,
        properties,
        required,
        fixed,
        result_shape: item.get("resultShape").and_then(Value::as_str).unwrap_or_default().to_string(),
        returns,
        private,
        public,
        request,
    })
}

pub fn parse(raw: &Value, folder: &str) -> Result<Manifest, String> {
    let item = record(raw).ok_or(format!("{FILE} must be an object."))?;
    let connector = text(item.get("connector"), "connector")?;
    if !NAME.is_match(&connector) {
        return Err(format!("Connector name \"{connector}\" is not allowed. Use lowercase letters, digits and -."));
    }
    if folder != format!("{PREFIX}{connector}") {
        return Err(format!("The folder must be named {PREFIX}{connector}, not {folder}."));
    }
    let kind = match item.get("kind").and_then(Value::as_str) {
        None | Some("http") => Kind::Http,
        Some("command") => Kind::Command,
        Some("module") => return Err("JavaScript module connectors do not run in Nixa. Use kind \"http\" or \"command\".".into()),
        Some("mcp") => Kind::Mcp,
        Some(other) => return Err(format!("Connector kind \"{other}\" is not supported. Use \"http\", \"command\" or \"mcp\".")),
    };
    if kind == Kind::Mcp && !super::mcp_kind::NAMES.contains(&connector.as_str()) {
        return Err(format!("MCP connector \"{connector}\" is not supported. Only the built-in {} connectors may use kind mcp.", super::mcp_kind::NAMES.join(", ")));
    }
    let version = item.get("version").and_then(Value::as_u64).filter(|v| *v >= 1).ok_or("version must be a whole number, 1 or more.")?;
    let program = match item.get("command") {
        None | Some(Value::Null) => DEFAULT_PROGRAM.to_string(),
        v => text(v, "command")?,
    };
    if !PROGRAM.is_match(&program) {
        return Err(format!("command \"{program}\" must be a plain file name in the connector folder."));
    }
    if kind != Kind::Command && item.get("command").is_some_and(|v| !v.is_null()) {
        return Err("command is only for command connectors.".into());
    }
    let mut hosts = Vec::new();
    for host in list(item.get("hosts"), "hosts")? {
        let host = text(Some(host), "hosts entry")?.to_lowercase();
        if !HOST.is_match(&host) {
            return Err(format!("hosts entry \"{host}\" must be a plain host name, no scheme, port or path."));
        }
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    let private = flag(item.get("private"), false, "private")?;
    let public = flag(item.get("public"), false, "public")?;
    if private && public {
        return Err("A connector cannot be both private and public.".into());
    }
    let secrets = entries(item.get("secrets"), "secrets", &connector)?;
    let optional_secrets: Vec<String> = entries(item.get("optionalSecrets"), "optionalSecrets", &connector)?.into_iter().filter(|e| !secrets.contains(e)).collect();
    let all: Vec<String> = secrets.iter().chain(optional_secrets.iter()).cloned().collect();
    let shared = Shared { kind, hosts: &hosts, secrets: &all, private, public };
    let mut actions = Vec::new();
    for raw in list(item.get("actions"), "actions")? {
        let action = parse_action(raw, &shared)?;
        if actions.iter().any(|a: &Action| a.name == action.name) {
            return Err(format!("Action \"{}\" is listed twice.", action.name));
        }
        actions.push(action);
    }
    if actions.is_empty() {
        return Err("A connector needs at least one action.".into());
    }
    if kind == Kind::Mcp
        && let Some(bad) = actions.iter().find(|a| !super::mcp_kind::allowed_tool(&connector, &a.name, &a.tool))
    {
        return Err(format!("MCP tool \"{}\" is not allowed for action {} of {connector}.", bad.tool, bad.name));
    }
    Ok(Manifest {
        connector,
        version,
        description: text(item.get("description"), "description")?,
        kind,
        program,
        hosts,
        secrets,
        optional_secrets,
        private,
        public,
        actions,
    })
}

pub fn check_input(action: &Action, given: Option<&Value>) -> Result<Map<String, Value>, String> {
    let input = match given {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Err("input must be an object.".into()),
    };
    for key in input.keys() {
        if !action.properties.contains_key(key) {
            let allowed: Vec<&str> = action.properties.keys().map(String::as_str).collect();
            return Err(format!("Unknown input \"{key}\". Allowed: {}.", if allowed.is_empty() { "none".into() } else { allowed.join(", ") }));
        }
    }
    for key in &action.required {
        if matches!(input.get(key), None | Some(Value::Null)) || input.get(key).and_then(Value::as_str) == Some("") {
            return Err(format!("Input \"{key}\" is required."));
        }
    }
    let mut out = Map::new();
    for (key, value) in input {
        if value.is_null() {
            continue;
        }
        let field = &action.properties[&key];
        let ok = match field.kind {
            FieldType::String => value.as_str().is_some_and(|s| s.chars().count() <= MAX_TEXT),
            FieldType::Boolean => value.is_boolean(),
            FieldType::Integer => value.is_i64() || value.is_u64(),
            FieldType::Number => value.as_f64().is_some_and(f64::is_finite),
        };
        if !ok {
            let more = if field.kind == FieldType::String { format!(" of at most {MAX_TEXT} characters") } else { String::new() };
            return Err(format!("Input \"{key}\" must be {}{more}.", field.kind.word()));
        }
        if !field.choices.is_empty() && !value.as_str().is_some_and(|s| field.choices.iter().any(|c| c == s)) {
            return Err(format!("Input \"{key}\" must be one of: {}.", field.choices.join(", ")));
        }
        out.insert(key, value);
    }
    for (key, value) in &action.fixed {
        out.insert(key.clone(), value.clone());
    }
    Ok(out)
}
