use super::run::Cancel;
use gpui::App;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Changed {
    Conflicts,
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub warning: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictAsk {
    pub root: PathBuf,
    pub file: String,
    pub ours: String,
    pub theirs: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub agent: Option<String>,
}

#[derive(Clone, Copy)]
pub struct Host {
    pub show_commit: fn(&Path, &str, &mut App) -> bool,
    pub explain_commit: fn(&Path, &str, &mut App) -> bool,
    pub open_file: fn(&Path, &str, &mut App) -> bool,
    pub pick_identity: fn(&Path, &mut App) -> bool,
    pub changed: fn(&Path, Changed, &mut App),
    pub notify: fn(&Notice, &mut App) -> bool,
    pub explain_conflict: fn(&ConflictAsk, &Cancel) -> Result<Answer, String>,
}

pub const NO_HISTORY: &str = "The History view is not in this build yet.";
pub const NO_AI: &str = "Git AI is not in this build yet.";
pub const NO_EDITOR: &str = "The editor is not in this build yet.";
pub const NO_PROFILES: &str =
    "Git profiles are not in this build yet. Set user.name and user.email for this repo.";

#[cfg(test)]
fn not_yet(_: &Path, _: &str, _: &mut App) -> bool {
    false
}

#[cfg(test)]
fn no_profiles(_: &Path, _: &mut App) -> bool {
    false
}

#[cfg(test)]
fn quiet(_: &Path, _: Changed, _: &mut App) {}

#[cfg(test)]
fn no_notice(_: &Notice, _: &mut App) -> bool {
    false
}

#[cfg(test)]
fn no_ai(_: &ConflictAsk, _: &Cancel) -> Result<Answer, String> {
    Err(NO_AI.into())
}

#[cfg(test)]
pub fn host() -> Host {
    Host {
        show_commit: not_yet,
        explain_commit: not_yet,
        open_file: not_yet,
        pick_identity: no_profiles,
        changed: quiet,
        notify: no_notice,
        explain_conflict: no_ai,
    }
}
