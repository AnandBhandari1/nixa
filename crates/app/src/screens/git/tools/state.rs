use super::run;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Op {
    pub fn word(self) -> &'static str {
        match self {
            Op::Merge => "merge",
            Op::Rebase => "rebase",
            Op::CherryPick => "cherry-pick",
            Op::Revert => "revert",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoState {
    pub op: Option<Op>,
    pub step: Option<(u32, u32)>,
    pub conflicts: Vec<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
}

fn full(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

fn cat(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name))
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn first_command(todo: &str) -> String {
    todo.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or_default()
        .to_string()
}

pub struct Marks {
    pub merge: bool,
    pub rebase_merge: bool,
    pub rebase_apply: bool,
    pub cherry_pick: bool,
    pub revert: bool,
    pub sequencer: bool,
    pub applying: bool,
    pub seq_first: String,
}

pub fn op_of(m: &Marks) -> Option<Op> {
    let rebasing = m.rebase_merge || m.rebase_apply;
    if rebasing {
        return (!m.applying).then_some(Op::Rebase);
    }
    if m.merge {
        return Some(Op::Merge);
    }
    if m.cherry_pick {
        return Some(Op::CherryPick);
    }
    if m.revert {
        return Some(Op::Revert);
    }
    if m.sequencer {
        return match m.seq_first.as_str() {
            "revert" => Some(Op::Revert),
            "pick" | "p" => Some(Op::CherryPick),
            _ => None,
        };
    }
    None
}

pub fn read_state(root: &Path) -> Result<RepoState, String> {
    let paths = run::git(
        root,
        &[
            "rev-parse",
            "--git-path",
            "MERGE_HEAD",
            "--git-path",
            "rebase-merge",
            "--git-path",
            "rebase-apply",
            "--git-path",
            "CHERRY_PICK_HEAD",
            "--git-path",
            "REVERT_HEAD",
            "--git-path",
            "sequencer/todo",
        ],
    );
    if !paths.ok() {
        let said = paths.said();
        return Err(if said.is_empty() {
            "cannot read the repo state".into()
        } else {
            said
        });
    }
    let list: Vec<PathBuf> = paths.stdout.lines().map(|l| full(root, l)).collect();
    if list.len() < 6 {
        return Err("git did not name the state files".into());
    }
    let merge_dir = &list[1];
    let apply_dir = &list[2];
    let dir = if merge_dir.is_dir() {
        Some(merge_dir.clone())
    } else if apply_dir.is_dir() {
        Some(apply_dir.clone())
    } else {
        None
    };
    let marks = Marks {
        merge: list[0].exists(),
        rebase_merge: merge_dir.exists(),
        rebase_apply: apply_dir.exists(),
        cherry_pick: list[3].exists(),
        revert: list[4].exists(),
        sequencer: list[5].exists(),
        applying: dir.as_ref().is_some_and(|d| d.join("applying").exists()),
        seq_first: first_command(&fs::read_to_string(&list[5]).unwrap_or_default()),
    };
    let op = op_of(&marks);
    let step = match (&dir, op) {
        (Some(d), Some(Op::Rebase)) => {
            let (n, total) = if d == merge_dir {
                (cat(d, "msgnum"), cat(d, "end"))
            } else {
                (cat(d, "next"), cat(d, "last"))
            };
            match (n.parse::<u32>(), total.parse::<u32>()) {
                (Ok(n), Ok(total)) if total > 0 => Some((n, total)),
                _ => None,
            }
        }
        _ => None,
    };
    let head_name = dir
        .as_ref()
        .map(|d| cat(d, "head-name"))
        .unwrap_or_default();
    let from_rebase = (op == Some(Op::Rebase))
        .then(|| head_name.strip_prefix("refs/heads/").map(str::to_string))
        .flatten();
    let branch = from_rebase.or_else(|| {
        let r = run::git(root, &["symbolic-ref", "-q", "--short", "HEAD"]);
        let name = r.first_line();
        (r.ok() && !name.is_empty()).then_some(name)
    });
    let head = {
        let r = run::git(root, &["rev-parse", "-q", "--verify", "--short", "HEAD"]);
        let sha = r.first_line();
        (r.ok() && !sha.is_empty()).then_some(sha)
    };
    let unmerged = run::read(
        root,
        &[
            "-c",
            "core.quotepath=off",
            "diff",
            "--name-only",
            "--diff-filter=U",
            "-z",
        ],
    );
    let conflicts = unmerged
        .stdout
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    Ok(RepoState {
        op,
        step,
        conflicts,
        branch,
        head,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks() -> Marks {
        Marks {
            merge: false,
            rebase_merge: false,
            rebase_apply: false,
            cherry_pick: false,
            revert: false,
            sequencer: false,
            applying: false,
            seq_first: String::new(),
        }
    }

    #[test]
    fn op_follows_the_marks() {
        assert_eq!(op_of(&marks()), None);
        assert_eq!(
            op_of(&Marks {
                merge: true,
                ..marks()
            }),
            Some(Op::Merge)
        );
        assert_eq!(
            op_of(&Marks {
                rebase_merge: true,
                merge: true,
                ..marks()
            }),
            Some(Op::Rebase)
        );
        assert_eq!(
            op_of(&Marks {
                rebase_apply: true,
                applying: true,
                ..marks()
            }),
            None
        );
        assert_eq!(
            op_of(&Marks {
                cherry_pick: true,
                ..marks()
            }),
            Some(Op::CherryPick)
        );
        assert_eq!(
            op_of(&Marks {
                revert: true,
                ..marks()
            }),
            Some(Op::Revert)
        );
        assert_eq!(
            op_of(&Marks {
                sequencer: true,
                seq_first: "revert".into(),
                ..marks()
            }),
            Some(Op::Revert)
        );
        assert_eq!(
            op_of(&Marks {
                sequencer: true,
                seq_first: "p".into(),
                ..marks()
            }),
            Some(Op::CherryPick)
        );
    }

    #[test]
    fn first_command_skips_comments() {
        assert_eq!(first_command("# hi\n\npick abc one\n"), "pick");
        assert_eq!(first_command(""), "");
    }
}
