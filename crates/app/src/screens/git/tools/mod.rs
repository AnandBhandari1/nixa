pub mod blame;
pub mod conflict;
pub mod errors;
pub mod host;
pub mod kit;
pub mod rebase;
pub mod run;
pub mod state;
#[cfg(test)]
pub mod testrepo;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;
pub mod window;

pub use host::Host;

use gpui::App;
use std::path::PathBuf;

pub const SHOW_BLAME: &str = "git-blame:";
pub const SHOW_CONFLICT: &str = "git-conflict:";
pub const SHOW_REBASE: &str = "git-rebase:";

pub fn open_blame(open: blame::Open, host: Host, cx: &mut App) {
    blame::open(open, host, cx);
}

pub fn open_conflict(open: conflict::Open, host: Host, cx: &mut App) {
    conflict::open(open, host, cx);
}

pub fn open_rebase(open: rebase::Open, host: Host, cx: &mut App) {
    rebase::open(open, host, cx);
}

pub fn parts(rest: &str) -> Vec<Option<String>> {
    rest.split('|')
        .map(|p| Some(p.trim().to_string()).filter(|s| !s.is_empty()))
        .collect()
}

fn part(list: &[Option<String>], i: usize) -> Option<String> {
    list.get(i).cloned().flatten()
}

pub fn blame_open(rest: &str) -> blame::Open {
    let p = parts(rest);
    blame::Open {
        repo: part(&p, 0).map(PathBuf::from),
        file: part(&p, 1),
        rev: part(&p, 2).unwrap_or_default(),
        line: part(&p, 3).and_then(|l| l.parse().ok()),
    }
}

pub fn conflict_open(rest: &str) -> conflict::Open {
    let p = parts(rest);
    conflict::Open {
        repo: part(&p, 0).map(PathBuf::from),
        file: part(&p, 1),
    }
}

pub fn rebase_open(rest: &str) -> rebase::Open {
    let p = parts(rest);
    rebase::Open {
        repo: part(&p, 0).map(PathBuf::from),
        base: part(&p, 1),
    }
}

pub fn show(what: &str, host: Host, cx: &mut App) -> bool {
    if let Some(rest) = what.strip_prefix(SHOW_BLAME) {
        open_blame(blame_open(rest), host, cx);
    } else if let Some(rest) = what.strip_prefix(SHOW_CONFLICT) {
        open_conflict(conflict_open(rest), host, cx);
    } else if let Some(rest) = what.strip_prefix(SHOW_REBASE) {
        open_rebase(rebase_open(rest), host, cx);
    } else {
        return false;
    }
    true
}
