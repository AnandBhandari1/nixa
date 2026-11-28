use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Codex,
    Opencode,
    Agy,
    Cursor,
    Kimi,
    Jcode,
    Dim,
    Omp,
    Omo,
    Claude,
    Crush,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Self::Codex,
        Self::Opencode,
        Self::Agy,
        Self::Cursor,
        Self::Kimi,
        Self::Jcode,
        Self::Dim,
        Self::Omp,
        Self::Omo,
        Self::Claude,
        Self::Crush,
    ];

    pub const SHOWN: [Kind; 5] = [Self::Codex, Self::Crush, Self::Agy, Self::Opencode, Self::Claude];

    pub fn parse(word: &str) -> Option<Kind> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.word() == word.trim())
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Agy => "agy",
            Self::Cursor => "cursor",
            Self::Kimi => "kimi",
            Self::Jcode => "jcode",
            Self::Dim => "dim",
            Self::Omp => "omp",
            Self::Omo => "omo",
            Self::Claude => "claude",
            Self::Crush => "crush",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Opencode => "OpenCode",
            Self::Agy => "Antigravity",
            Self::Cursor => "Cursor",
            Self::Kimi => "Kimi",
            Self::Jcode => "Jcode",
            Self::Dim => "DimCode",
            Self::Omp => "omp",
            Self::Omo => "OmO",
            Self::Claude => "Claude Code",
            Self::Crush => "Crush",
        }
    }

    pub fn bin_name(self) -> &'static str {
        match self {
            Self::Cursor => "cursor-agent",
            Self::Dim => "dimcode",
            other => other.word(),
        }
    }

    pub fn bin_env(self) -> String {
        format!("NIXA_{}_BIN", self.word().to_uppercase())
    }
}
