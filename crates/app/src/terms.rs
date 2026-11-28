use crate::tabbar::QUICK_AGENTS;
use gpui::{AnyElement, ElementId};
use nixa_protocol::{Status, TermInfo};
use nixa_ui::Theme;
use nixa_ui::chip::{program_badge as badge, term_mark};
use std::path::Path;

const SHELLS: [&str; 5] = ["zsh", "bash", "sh", "fish", "nu"];

pub fn program(cmd: &[String]) -> String {
    cmd.first()
        .map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.clone())
        })
        .unwrap_or_default()
}

pub fn is_shell(cmd: &[String]) -> bool {
    cmd.is_empty() || SHELLS.contains(&program(cmd).trim_start_matches('-'))
}

pub fn monogram(cmd: &[String]) -> String {
    if let Some(agent) = QUICK_AGENTS.iter().find(|a| {
        let run = crate::screens::tabbar::quick::run_of(a, false);
        !run.is_empty() && cmd.starts_with(&run)
    }) {
        return agent.mono.to_string();
    }
    program(cmd)
        .chars()
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

pub fn program_badge(t: &Theme, cmd: &[String], size: f32) -> AnyElement {
    // Program marks are shared by the list, its compact tiles, children and pane headers.
    let word = crate::programs::program_word(cmd).unwrap_or_default();
    let word = Path::new(word).file_name().and_then(|s| s.to_str()).unwrap_or(word);
    let name = match word {
        "cursor-agent" => "cursor",
        "agy" | "agyd" => "antigravity",
        "omp" => "pi",
        "dima" | "dimcode" => "dim",
        other => other,
    };
    if let Some(mark) = nixa_ui::agent_icons::glyph(name, t.foreground, size) {
        return mark;
    }
    let mono = (!is_shell(cmd)).then(|| monogram(cmd));
    badge(t, mono.as_deref(), size)
}

pub fn term_name(term: &TermInfo) -> String {
    if !term.name.trim().is_empty() {
        return term.name.clone();
    }
    let p = program(&term.cmd);
    if p.is_empty() { "shell".to_string() } else { p }
}

pub fn status_mark(t: &Theme, id: impl Into<ElementId>, term: Option<&TermInfo>) -> AnyElement {
    term_mark(t, id, term)
}

pub fn status_rank(term: &TermInfo) -> u8 {
    match term.status {
        Some(Status::Blocked) => 5,
        Some(Status::Error) => 4,
        Some(Status::Working) => 3,
        Some(Status::Done) => 2,
        Some(Status::Idle) => 1,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn shells_and_agents() {
        assert!(is_shell(&[]));
        assert!(is_shell(&cmd(&["/bin/zsh", "-l"])));
        assert!(is_shell(&cmd(&["-zsh"])));
        assert!(!is_shell(&cmd(&["claude"])));
        assert_eq!(
            monogram(&cmd(&["claude", "--dangerously-skip-permissions"])),
            "CC"
        );
        assert_eq!(monogram(&cmd(&["/usr/bin/htop"])), "HT");
    }
}
