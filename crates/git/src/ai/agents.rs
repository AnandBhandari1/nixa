use super::job::{self, Spec};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptVia {
    Stdin,
    Arg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Agent {
    pub id: &'static str,
    pub label: &'static str,
    pub binary: &'static str,
    pub fallbacks: &'static [&'static str],
    pub prompt_via: PromptVia,
    pub output_file: bool,
    pub needs_model: bool,
    pub model_hint: &'static str,
}

const fn agent(id: &'static str, label: &'static str, binary: &'static str, prompt_via: PromptVia, model_hint: &'static str) -> Agent {
    Agent {
        id,
        label,
        binary,
        fallbacks: &[],
        prompt_via,
        output_file: false,
        needs_model: false,
        model_hint,
    }
}

pub const AGENTS: &[Agent] = &[
    agent("claude", "Claude Code", "claude", PromptVia::Stdin, "haiku, sonnet or opus"),
    Agent {
        output_file: true,
        ..agent("codex", "Codex CLI", "codex", PromptVia::Arg, "")
    },
    agent("opencode", "OpenCode", "opencode", PromptVia::Arg, "provider/model"),
    Agent {
        fallbacks: &["agent"],
        ..agent("cursor", "Cursor CLI", "cursor-agent", PromptVia::Arg, "")
    },
    agent("antigravity", "Antigravity CLI", "agy", PromptVia::Arg, ""),
    agent("omp", "Oh My Pi", "omp", PromptVia::Arg, "opus or provider/model"),
    Agent {
        needs_model: true,
        ..agent("ollama", "Ollama", "ollama", PromptVia::Stdin, "")
    },
    agent("gemini", "Gemini CLI", "gemini", PromptVia::Stdin, ""),
    agent("copilot", "GitHub Copilot CLI", "copilot", PromptVia::Arg, ""),
];

pub const JOB_ENV: &[(&str, &str)] = &[("NO_COLOR", "1"), ("CLICOLOR", "0"), ("TERM", "dumb")];
pub const PATH_MARK: &str = "__NIXA_PATH__";
pub const CACHE: Duration = Duration::from_secs(60);
pub const SHELL_PATH_CACHE: Duration = Duration::from_secs(600);
pub const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(10);
pub const OLLAMA_TIMEOUT: Duration = Duration::from_secs(15);

pub fn agent_by_id(id: &str) -> Option<&'static Agent> {
    AGENTS.iter().find(|a| a.id == id)
}

pub fn binaries_of(agent: &Agent) -> Vec<&'static str> {
    std::iter::once(agent.binary).chain(agent.fallbacks.iter().copied()).collect()
}

fn opt(flag: &str, value: &str) -> Vec<String> {
    if value.is_empty() {
        vec![]
    } else {
        vec![flag.into(), value.into()]
    }
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

pub struct ChecksOff {
    pub id: &'static str,
    pub flags: &'static [&'static str],
    pub what: &'static str,
}

pub const CHECKS_OFF: &[ChecksOff] = &[
    ChecksOff {
        id: "cursor",
        flags: &["--trust"],
        what: "Trust the folder without asking",
    },
    ChecksOff {
        id: "copilot",
        flags: &["--no-ask-user"],
        what: "Work without asking questions",
    },
];

pub fn checks_off_of(agent: &Agent) -> Option<&'static ChecksOff> {
    CHECKS_OFF.iter().find(|c| c.id == agent.id)
}

pub fn args_of(agent: &Agent, model: &str, out_file: Option<&Path>, checks_off: bool) -> Vec<String> {
    let mut a = match agent.id {
        "claude" => words(&["-p", "--output-format", "text", "--no-session-persistence"]),
        "codex" => words(&["exec", "--skip-git-repo-check", "--sandbox", "read-only", "--ephemeral", "--color", "never"]),
        "opencode" => words(&["run"]),
        "cursor" => words(&["-p", "--output-format", "text"]),
        "antigravity" => words(&["--output-format", "text"]),
        "omp" => words(&["-p", "--no-session"]),
        "ollama" => return vec!["run".into(), model.into(), "--hidethinking".into()],
        "copilot" => words(&["--silent", "--stream", "off", "--no-color", "--output-format", "text"]),
        _ => vec![],
    };
    if checks_off && let Some(c) = checks_off_of(agent) {
        a.extend(words(c.flags));
    }
    let model_flag = match agent.id {
        "codex" | "opencode" | "gemini" => "-m",
        _ => "--model",
    };
    a.extend(opt(model_flag, model));
    match agent.id {
        "codex" => {
            if let Some(out) = out_file {
                a.extend(["-o".to_string(), out.display().to_string()]);
            }
        }
        "antigravity" => a.push("-p".into()),
        "copilot" => a.push("--prompt".into()),
        _ => {}
    }
    a
}

pub fn command_args(agent: &Agent, model: &str, out_file: Option<&Path>, checks_off: bool, prompt: &str) -> (Vec<String>, Option<String>) {
    let mut args = args_of(agent, model, out_file, checks_off);
    match agent.prompt_via {
        PromptVia::Arg => {
            args.push(prompt.into());
            (args, None)
        }
        PromptVia::Stdin => (args, Some(prompt.into())),
    }
}

pub fn model_placeholder(agent: Option<&Agent>, ollama_default: &str) -> String {
    let Some(agent) = agent else {
        return "Default model".into();
    };
    if agent.needs_model {
        return if ollama_default.is_empty() {
            "Default: the first model in ollama list".into()
        } else {
            format!("Default: {ollama_default}")
        };
    }
    let hint = if agent.model_hint.is_empty() {
        String::new()
    } else {
        format!(" (for example {})", agent.model_hint)
    };
    format!("Default: what {} uses{hint}", agent.label)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub label: String,
    pub installed: bool,
    pub path: String,
}

pub fn shell_path_script() -> String {
    format!("printf '\\n{PATH_MARK}%s\\n' \"$PATH\"")
}

pub fn parse_shell_path(text: &str) -> String {
    let line = text.split('\n').rev().find(|l| l.starts_with(PATH_MARK));
    let path = line.map(|l| l[PATH_MARK.len()..].trim()).unwrap_or("");
    path.split(':').filter(|p| p.starts_with('/')).collect::<Vec<_>>().join(":")
}

pub fn merge_paths(first: &str, second: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in first.split(':').chain(second.split(':')) {
        if part.starts_with('/') && !out.contains(&part) {
            out.push(part);
        }
    }
    out.join(":")
}

fn runnable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn find_binary(names: &[&str], path_list: &str) -> Option<PathBuf> {
    names.iter().find_map(|name| {
        path_list
            .split(':')
            .filter(|d| d.starts_with('/'))
            .map(|d| Path::new(d).join(name))
            .find(|p| runnable(p))
    })
}

pub fn detect_in(path_list: &str) -> Vec<Found> {
    AGENTS
        .iter()
        .map(|a| {
            let path = find_binary(&binaries_of(a), path_list);
            Found {
                id: a.id.into(),
                label: a.label.into(),
                installed: path.is_some(),
                path: path.map(|p| p.display().to_string()).unwrap_or_default(),
            }
        })
        .collect()
}

static SHELL_PATH: Mutex<Option<(Instant, String)>> = Mutex::new(None);
static FOUND: Mutex<Option<(Instant, Vec<Found>)>> = Mutex::new(None);

fn login_shell_path() -> String {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| runnable(Path::new(s)))
        .unwrap_or_else(|| "/bin/zsh".into());
    let spec = Spec {
        program: shell.into(),
        args: vec!["-ilc".into(), shell_path_script()],
        timeout: SHELL_PATH_TIMEOUT,
        ..Spec::default()
    };
    job::run(&spec, &AtomicBool::new(false))
        .map(|o| parse_shell_path(&o.stdout))
        .unwrap_or_default()
}

pub fn search_path() -> String {
    let own = std::env::var("PATH").unwrap_or_default();
    if let Ok(hit) = SHELL_PATH.lock()
        && let Some((at, path)) = hit.as_ref()
        && at.elapsed() < SHELL_PATH_CACHE
    {
        return merge_paths(&own, path);
    }
    let found = login_shell_path();
    if let Ok(mut slot) = SHELL_PATH.lock() {
        *slot = Some((Instant::now(), found.clone()));
    }
    merge_paths(&own, &found)
}

pub fn detect_agents(fresh: bool) -> Vec<Found> {
    if !fresh
        && let Ok(hit) = FOUND.lock()
        && let Some((at, list)) = hit.as_ref()
        && at.elapsed() < CACHE
    {
        return list.clone();
    }
    let list = detect_in(&search_path());
    if let Ok(mut slot) = FOUND.lock() {
        *slot = Some((Instant::now(), list.clone()));
    }
    list
}

pub fn forget_agents() {
    if let Ok(mut slot) = FOUND.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = SHELL_PATH.lock() {
        *slot = None;
    }
}

pub fn parse_ollama_list(text: &str) -> Vec<String> {
    text.split('\n')
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !name.to_lowercase().contains("embed"))
        .map(str::to_string)
        .collect()
}

pub fn ollama_models(path: &str) -> Vec<String> {
    let spec = Spec {
        program: if path.is_empty() { "ollama".into() } else { path.into() },
        args: vec!["list".into()],
        env: vec![("PATH".into(), search_path())],
        timeout: OLLAMA_TIMEOUT,
        ..Spec::default()
    };
    match job::run(&spec, &AtomicBool::new(false)) {
        Ok(out) if out.code == Some(0) => parse_ollama_list(&out.stdout),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn every_agent_builds_its_command() {
        let claude = agent_by_id("claude").unwrap();
        let (args, stdin) = command_args(claude, "haiku", None, false, "P");
        assert_eq!(args, words(&["-p", "--output-format", "text", "--no-session-persistence", "--model", "haiku"]));
        assert_eq!(stdin.as_deref(), Some("P"));
        let codex = agent_by_id("codex").unwrap();
        let (args, stdin) = command_args(codex, "", Some(Path::new("/t/o")), false, "P");
        assert_eq!(args.last().map(String::as_str), Some("P"));
        assert!(args.windows(2).any(|w| w == ["-o", "/t/o"]));
        assert!(!args.contains(&"-m".to_string()));
        assert_eq!(stdin, None);
        let agy = agent_by_id("antigravity").unwrap();
        assert_eq!(command_args(agy, "m", None, false, "P").0, words(&["--output-format", "text", "--model", "m", "-p", "P"]));
        let ollama = agent_by_id("ollama").unwrap();
        assert_eq!(args_of(ollama, "llama3", None, true), words(&["run", "llama3", "--hidethinking"]));
        let gemini = agent_by_id("gemini").unwrap();
        assert_eq!(args_of(gemini, "g", None, true), words(&["-m", "g"]));
        let copilot = agent_by_id("copilot").unwrap();
        assert_eq!(command_args(copilot, "", None, false, "P").0.last().map(String::as_str), Some("P"));
        assert_eq!(args_of(claude, "", None, true), args_of(claude, "", None, false));
        assert_eq!(binaries_of(agent_by_id("cursor").unwrap()), vec!["cursor-agent", "agent"]);
        assert!(agent_by_id("nope").is_none());
    }

    #[test]
    fn checks_stay_on_unless_the_user_turns_them_off() {
        let cursor = agent_by_id("cursor").unwrap();
        assert_eq!(args_of(cursor, "", None, false), words(&["-p", "--output-format", "text"]));
        assert_eq!(args_of(cursor, "", None, true), words(&["-p", "--output-format", "text", "--trust"]));
        let copilot = agent_by_id("copilot").unwrap();
        assert!(!args_of(copilot, "", None, false).contains(&"--no-ask-user".to_string()));
        assert!(args_of(copilot, "", None, true).contains(&"--no-ask-user".to_string()));
        for agent in AGENTS {
            let flags = checks_off_of(agent).map(|c| c.flags).unwrap_or(&[]);
            let args = args_of(agent, "m", Some(Path::new("/t/o")), false);
            assert!(flags.iter().all(|f| !args.contains(&f.to_string())), "{}", agent.id);
            for bad in ["--trust", "--no-ask-user", "--force", "--yolo", "--dangerously-skip-permissions", "--allow-all-tools"] {
                assert!(!args.contains(&bad.to_string()), "{} {bad}", agent.id);
            }
        }
    }

    #[test]
    fn placeholders() {
        assert_eq!(model_placeholder(None, ""), "Default model");
        assert_eq!(model_placeholder(agent_by_id("claude"), ""), "Default: what Claude Code uses (for example haiku, sonnet or opus)");
        assert_eq!(model_placeholder(agent_by_id("codex"), ""), "Default: what Codex CLI uses");
        assert_eq!(model_placeholder(agent_by_id("ollama"), "llama3"), "Default: llama3");
    }

    #[test]
    fn shell_path_parsing() {
        let text = format!("motd\n{PATH_MARK}/a:/b:rel:/c\n");
        assert_eq!(parse_shell_path(&text), "/a:/b:/c");
        assert_eq!(parse_shell_path("none"), "");
        assert_eq!(merge_paths("/a:/b", "/b:/c:x"), "/a:/b:/c");
    }

    #[test]
    fn finds_runnable_binaries_only() {
        let dir = std::env::temp_dir().join(format!("nixa-git-ai-find-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("agent");
        fs::write(&tool, "#!/bin/sh\n").unwrap();
        fs::write(dir.join("cursor-agent"), "no exec").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let list = detect_in(&dir.display().to_string());
        let cursor = list.iter().find(|f| f.id == "cursor").unwrap();
        assert!(cursor.installed);
        assert_eq!(cursor.path, tool.display().to_string());
        assert!(!list.iter().find(|f| f.id == "claude").unwrap().installed);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ollama_list_skips_embeddings() {
        let text = "NAME ID SIZE\nllama3.2:latest a 2GB\nnomic-embed-text b 1GB\n\nqwen c 1GB\n";
        assert_eq!(parse_ollama_list(text), vec!["llama3.2:latest", "qwen"]);
    }
}
