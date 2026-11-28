use crate::programs;
use nixa_store::agents::{checks_off_from, start_command_with, start_flags};
use std::path::{Path, PathBuf};

pub struct QuickAgent {
    pub id: &'static str,
    pub name: &'static str,
    pub mono: &'static str,
    pub icon: &'static str,
    pub provider: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button {
    pub index: usize,
    pub off: bool,
}

pub const QUICK_AGENTS: &[QuickAgent] = &[
    QuickAgent {
        id: "claude",
        mono: "CC",
        name: "Claude Code",
        icon: "claude",
        provider: "claude",
    },
    QuickAgent {
        id: "codex",
        mono: "CX",
        name: "Codex",
        icon: "codex",
        provider: "codex",
    },
    QuickAgent {
        id: "opencode",
        mono: "OC",
        name: "OpenCode",
        icon: "opencode",
        provider: "opencode",
    },
    QuickAgent {
        id: "crush",
        mono: "CR",
        name: "Crush",
        icon: "crush",
        provider: "crush",
    },
    QuickAgent {
        id: "jcode",
        mono: "JC",
        name: "Jcode",
        icon: "jcode",
        provider: "jcode",
    },
    QuickAgent {
        id: "hermes",
        mono: "HE",
        name: "Hermes",
        icon: "hermes",
        provider: "hermes",
    },
    QuickAgent {
        id: "dim",
        mono: "DI",
        name: "DimCode",
        icon: "dim",
        provider: "dim",
    },
    QuickAgent {
        id: "cursor",
        mono: "CU",
        name: "Cursor",
        icon: "cursor",
        provider: "cursor",
    },
    QuickAgent {
        id: "antigravity",
        mono: "AG",
        name: "Antigravity",
        icon: "antigravity",
        provider: "agy",
    },
    QuickAgent {
        id: "omp",
        mono: "PI",
        name: "Oh My Pi",
        icon: "pi",
        provider: "omp",
    },
    QuickAgent {
        id: "gemini",
        mono: "GE",
        name: "Gemini",
        icon: "gemini",
        provider: "gemini",
    },
    QuickAgent {
        id: "kimi",
        mono: "KI",
        name: "Kimi",
        icon: "agent",
        provider: "kimi",
    },
    QuickAgent {
        id: "omo",
        mono: "OM",
        name: "OmO",
        icon: "omo",
        provider: "omo",
    },
];

fn words(line: Option<&str>) -> Vec<String> {
    line.and_then(|l| programs::split_command(l).ok())
        .unwrap_or_default()
}

pub fn run_of(agent: &QuickAgent, off: bool) -> Vec<String> {
    words(start_command_with(agent.provider, off))
}

pub fn off_flags(agent: &QuickAgent) -> String {
    start_flags(agent.provider).to_string()
}

pub fn can_turn_off(agent: &QuickAgent) -> bool {
    start_command_with(agent.provider, true) != start_command_with(agent.provider, false)
}

pub fn tip(agent: &QuickAgent, off: bool) -> String {
    if off && can_turn_off(agent) {
        format!("Open {} · checks off", agent.name)
    } else {
        format!("Open {}", agent.name)
    }
}

fn dim_binary(dirs: &[PathBuf], home: Option<&Path>) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join("dima"))
        .find(|path| programs::runnable(path))
        .or_else(|| home.and_then(nixa_protocol::paths::dima_node_bin).map(|dir| dir.join("dima")))
}

pub fn is_installed(agent: &QuickAgent, dirs: &[PathBuf]) -> bool {
    if agent.provider == "dim" {
        let home = if cfg!(test) { None } else { std::env::var_os("HOME").map(PathBuf::from) };
        return dim_binary(dirs, home.as_deref()).is_some();
    }
    programs::installed(&run_of(agent, false), dirs)
}

pub fn off_of(text: Option<&str>, agent: &QuickAgent) -> bool {
    can_turn_off(agent) && checks_off_from(text, agent.provider)
}

pub fn buttons(text: Option<&str>, dirs: &[PathBuf]) -> Vec<Button> {
    programs::BUILT_IN
        .iter()
        .filter_map(|(key, ..)| QUICK_AGENTS.iter().enumerate().find(|(_, agent)| agent.id == *key))
        .filter(|(_, agent)| is_installed(agent, dirs))
        .map(|(index, agent)| Button {
            index,
            off: off_of(text, agent),
        })
        .collect()
}

pub fn buttons_in(file: &Path, dirs: &[PathBuf]) -> Vec<Button> {
    let text = programs::read(file).ok().flatten();
    buttons(text.as_deref(), dirs)
}

pub fn button_in(file: &Path, index: usize) -> Option<Button> {
    let agent = QUICK_AGENTS.get(index)?;
    let text = programs::read(file).ok().flatten();
    Some(Button { index, off: off_of(text.as_deref(), agent) })
}

pub fn start(button: Button, model: Option<&str>) -> Option<(String, Vec<String>)> {
    let agent = QUICK_AGENTS.get(button.index)?;
    let mut run = run_of(agent, button.off);
    if run.is_empty() {
        return None;
    }
    if agent.provider == "dim" {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let bin = dim_binary(&programs::default_dirs(), home.as_deref())?;
        let executable = run.iter_mut().find(|word| word.as_str() == "dima")?;
        *executable = bin.to_str()?.to_owned();
    }
    if let Some(model) = model {
        if !matches!(agent.provider, "agy" | "claude" | "codex" | "cursor" | "omp" | "opencode" | "gemini" | "kimi")
            || model.trim().is_empty() || model.starts_with('-')
        {
            return None;
        }
        run.extend(["--model".into(), model.into()]);
    }
    Some((agent.name.into(), run))
}
