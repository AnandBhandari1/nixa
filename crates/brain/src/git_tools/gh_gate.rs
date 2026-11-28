use regex::Regex;
use std::sync::LazyLock;

pub const REAL_GH_ENV: &str = "NIXA_REAL_GH";
pub const PARENT_STEPS: usize = 15;
pub const ALWAYS_READ: [&str; 9] = ["search", "status", "browse", "help", "version", "--version", "--help", "-h", "completion"];
pub const FLAG_VALUES: [&str; 3] = ["-R", "--repo", "--hostname"];
pub const READ_ONLY: [(&str, &[&str]); 17] = [
    ("pr", &["view", "diff", "list", "checks", "status", "checkout"]),
    ("issue", &["view", "list", "status"]),
    ("repo", &["view", "list", "clone"]),
    ("run", &["view", "list", "watch", "download"]),
    ("workflow", &["view", "list"]),
    ("release", &["view", "list", "download"]),
    ("gist", &["view", "list", "clone"]),
    ("secret", &["list"]),
    ("variable", &["list", "get"]),
    ("label", &["list"]),
    ("cache", &["list"]),
    ("ruleset", &["list", "view", "check"]),
    ("project", &["list", "view", "field-list", "item-list"]),
    ("auth", &["status"]),
    ("config", &["get", "list"]),
    ("extension", &["list", "search", "browse"]),
    ("codespace", &["list", "view", "logs"]),
];

static AGENTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:claude|claude-code|codex|agy|antigravity|kimi|kimi-code|cursor-agent|opencode|omp|gemini|goose|aider|droid|amp|crush|jcode|hermes|dim|dima|dimcode|pi|copilot|qwen)$").expect("agents regex")
});
static METHOD_SHORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-X[A-Za-z]+$").expect("method regex"));
static READ_METHOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:GET|HEAD)$").expect("read method regex"));
static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:-f|-F|--field|--raw-field|--input)(?:=|$)").expect("field regex"));
static MUTATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bmutation\b").expect("mutation regex"));
static SCRIPT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?:js|mjs|cjs|ts)$").expect("script regex"));

fn words(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if FLAG_VALUES.contains(&arg.as_str()) {
            skip = true;
            continue;
        }
        if arg.starts_with('-') && out.len() < 2 {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn api_write(args: &[String]) -> bool {
    let at = args.iter().position(|a| a == "-X" || a == "--method" || a.starts_with("--method=") || METHOD_SHORT.is_match(a));
    let method = at.and_then(|i| {
        let arg = &args[i];
        if let Some(rest) = arg.strip_prefix("--method=") {
            Some(rest.to_string())
        } else if METHOD_SHORT.is_match(arg) {
            Some(arg[2..].to_string())
        } else {
            args.get(i + 1).cloned()
        }
    });
    if let Some(method) = method {
        return !READ_METHOD.is_match(&method);
    }
    if args.iter().any(|a| a == "graphql") {
        return args.iter().any(|a| MUTATION.is_match(a));
    }
    args.iter().any(|a| FIELD.is_match(a))
}

pub fn gh_write(args: &[String]) -> Option<String> {
    let found = words(args);
    let group = found.first()?;
    if ALWAYS_READ.contains(&group.as_str()) {
        return None;
    }
    if group == "api" {
        return api_write(args).then(|| "api write".into());
    }
    let action = found.get(1);
    let reads = READ_ONLY.iter().find(|(g, _)| g == group).map(|(_, r)| *r);
    match (reads, action) {
        (Some(reads), Some(action)) if reads.contains(&action.as_str()) => None,
        (Some(_), None) => None,
        (_, Some(action)) => Some(format!("{group} {action}")),
        (None, None) => Some(group.clone()),
    }
}

pub fn agent_chain(commands: &[String]) -> Option<String> {
    for command in commands {
        for part in command.split_whitespace().take(2) {
            let name = part.rsplit('/').next().unwrap_or(part);
            let name = SCRIPT_END.replace(name, "");
            if AGENTS.is_match(&name) {
                return Some(name.to_lowercase());
            }
        }
    }
    None
}

pub fn parents(start: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut pid = start;
    for _ in 0..PARENT_STEPS {
        if pid <= 1 {
            break;
        }
        let Ok(found) = std::process::Command::new("/bin/ps").args(["-o", "ppid=,args=", "-p", &pid.to_string()]).output() else { break };
        let line = String::from_utf8_lossy(&found.stdout).trim().to_string();
        let Some((ppid, args)) = line.split_once(char::is_whitespace) else { break };
        let Ok(next) = ppid.trim().parse::<u32>() else { break };
        out.push(args.trim().to_string());
        pid = next;
    }
    out
}

pub fn gate_main(args: &[String]) -> i32 {
    let (code, text) = super::gate::run_plan(&super::gate::env_now(args));
    if !text.is_empty() {
        eprintln!("{text}");
    }
    code
}
