pub mod accounts_tab;
pub mod ai_tab;
pub mod clone_win;
pub mod kit;
pub mod identity_win;
pub mod pr_win;
pub mod profiles_tab;
pub mod repo_tab;
#[cfg(test)]
mod tests;

use gpui::App;
use std::path::PathBuf;

pub const SETTINGS_PART: &str = "Git accounts";

pub fn show(what: &str, folder: Option<&str>, cx: &mut App) -> bool {
    match what.split_once(':') {
        None if what == "git-pr" => pr_win::open(folder.map(PathBuf::from), cx),
        None if what == "git-clone" => clone_win::open(None, None, cx),
        Some(("git-clone", url)) => clone_win::open(Some(url.to_string()).filter(|u| !u.is_empty()), None, cx),
        Some(("git-pr", repo)) => pr_win::open(Some(PathBuf::from(repo)).filter(|r| !r.as_os_str().is_empty()), cx),
        _ => return false,
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitCommand {
    Clone,
    CreatePr,
    Accounts,
}

pub fn commands() -> Vec<(&'static str, &'static str, GitCommand)> {
    vec![
        ("Git: Clone a repo", "Clone a repo from GitHub or a link into a new folder", GitCommand::Clone),
        ("Git: Create pull request", "Push this branch and open a pull request with gh", GitCommand::CreatePr),
        ("Git: Accounts and profiles", "Open the git accounts, profiles and repo settings", GitCommand::Accounts),
    ]
}

pub fn run(command: GitCommand, folder: Option<String>, cx: &mut App) {
    match command {
        GitCommand::Clone => clone_win::open(None, None, cx),
        GitCommand::CreatePr => pr_win::open(folder.map(PathBuf::from), cx),
        GitCommand::Accounts => cx.defer(|cx| crate::screens::settings::open(SETTINGS_PART, cx)),
    }
}
