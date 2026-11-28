use super::git::{self, Done, Kind, PullMode, RepoState, plural};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub level: Level,
    pub spoken: String,
}

pub fn note(title: impl Into<String>, body: impl Into<String>, level: Level, spoken: impl Into<String>) -> Note {
    Note {
        title: title.into(),
        body: body.into(),
        level,
        spoken: spoken.into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteKind {
    Fetch,
    Pull,
    Push,
}

impl RemoteKind {
    pub fn word(self) -> &'static str {
        match self {
            RemoteKind::Fetch => "Fetch",
            RemoteKind::Pull => "Pull",
            RemoteKind::Push => "Push",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteOpts {
    pub mode: Option<PullMode>,
    pub then_push: bool,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub tags: bool,
    pub all: bool,
    pub prune: Option<bool>,
    pub force_lease: bool,
    pub quiet: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookFail {
    pub title: String,
    pub detail: String,
    pub all: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitOutcome {
    pub note: Option<Note>,
    pub clear: bool,
    pub reload: bool,
    pub hook_fail: Option<HookFail>,
    pub ask_identity: bool,
    pub amend_off: bool,
}

pub fn commit_outcome(r: &Done, amend: bool, summary: &str, all: bool) -> CommitOutcome {
    if r.ok {
        let done = if amend { "Commit updated" } else { "Commit done" };
        return CommitOutcome {
            note: Some(note(done, summary, Level::Success, done)),
            clear: true,
            reload: true,
            ..CommitOutcome::default()
        };
    }
    let detail = if r.detail.is_empty() { r.hint.clone() } else { r.detail.clone() };
    match r.kind {
        Kind::Hook => CommitOutcome {
            hook_fail: Some(HookFail {
                title: r.title.clone(),
                detail: detail.clone(),
                all,
            }),
            note: Some(note(r.title.clone(), detail, Level::Error, "Commit stopped by a hook")),
            ..CommitOutcome::default()
        },
        Kind::Identity => CommitOutcome {
            ask_identity: true,
            note: Some(note(r.title.clone(), r.hint.clone(), Level::Error, "Commit failed")),
            ..CommitOutcome::default()
        },
        Kind::HeadChanged => CommitOutcome {
            amend_off: true,
            note: Some(note(r.title.clone(), r.hint.clone(), Level::Warning, "The last commit changed")),
            ..CommitOutcome::default()
        },
        _ => CommitOutcome {
            note: Some(note(
                if r.title.is_empty() { "Commit failed".into() } else { r.title.clone() },
                r.body(),
                Level::Error,
                "Commit failed",
            )),
            ..CommitOutcome::default()
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub detail: String,
    pub stale: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteOutcome {
    pub note: Option<Note>,
    pub quiet_error: Option<Note>,
    pub next_push: bool,
    pub rejected: Option<Rejected>,
    pub open_accounts: Option<String>,
    pub reload: bool,
}

pub fn tag_note(r: &Done) -> String {
    if r.tags.is_empty() {
        return String::new();
    }
    let more = if r.tags.len() > 5 { ", …" } else { "" };
    format!(
        "{} sent: {}{more}",
        plural(r.tags.len(), "tag", "tags"),
        r.tags.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
    )
}

pub fn remote_outcome(
    kind: RemoteKind,
    r: &Done,
    quiet: bool,
    then_push: bool,
    same: bool,
    branch: Option<&str>,
) -> RemoteOutcome {
    let words = kind.word();
    if r.ok && r.stash_conflict {
        let mut body = r.hint.clone();
        if then_push {
            body.push_str("\nThe push was not started. Push again when the files are fixed.");
        }
        return RemoteOutcome {
            note: Some(note(r.title.clone(), body, Level::Warning, "Pulled, but your changes clashed")),
            ..RemoteOutcome::default()
        };
    }
    if r.ok {
        let tags = if kind == RemoteKind::Push { tag_note(r) } else { String::new() };
        let reload = kind == RemoteKind::Push;
        if then_push && same {
            let body = ["Now pushing…".to_string(), tags]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            return RemoteOutcome {
                next_push: true,
                reload,
                note: Some(note(
                    if r.title.is_empty() { "Pulled".into() } else { r.title.clone() },
                    body,
                    Level::Info,
                    "Pulled, now pushing",
                )),
                ..RemoteOutcome::default()
            };
        }
        if quiet {
            return RemoteOutcome {
                reload,
                ..RemoteOutcome::default()
            };
        }
        if kind == RemoteKind::Push && r.published {
            let b = r.branch.clone().unwrap_or_default();
            let where_to = r.remote.clone().unwrap_or_default();
            let body = [format!("{b} is now on {where_to}."), tags]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            return RemoteOutcome {
                reload,
                note: Some(note(format!("Published {b}"), body, Level::Success, format!("Published {b}"))),
                ..RemoteOutcome::default()
            };
        }
        let title = if r.title.is_empty() { format!("{words} done") } else { r.title.clone() };
        return RemoteOutcome {
            reload,
            note: Some(note(title.clone(), tags, Level::Success, title)),
            ..RemoteOutcome::default()
        };
    }
    if r.kind == Kind::Cancelled {
        if quiet {
            return RemoteOutcome::default();
        }
        return RemoteOutcome {
            note: Some(note(
                format!("{words} stopped"),
                "Nothing more was sent.",
                Level::Info,
                format!("{words} stopped"),
            )),
            ..RemoteOutcome::default()
        };
    }
    let title = if r.title.is_empty() { format!("{words} failed") } else { r.title.clone() };
    if quiet {
        return RemoteOutcome {
            quiet_error: Some(note(format!("Auto fetch: {title}"), r.body(), Level::Error, "Auto fetch failed")),
            ..RemoteOutcome::default()
        };
    }
    if r.kind == Kind::Rejected && kind == RemoteKind::Push {
        return RemoteOutcome {
            rejected: Some(Rejected {
                remote: r.remote.clone(),
                branch: r.branch.clone().or(branch.map(str::to_string)),
                detail: r.detail.clone(),
                stale: r.stale,
            }),
            note: Some(note("Push rejected", "", Level::Warning, "Push rejected")),
            ..RemoteOutcome::default()
        };
    }
    if matches!(r.kind, Kind::Auth | Kind::NotFound) {
        return RemoteOutcome {
            open_accounts: Some(format!("{words} did not work. {}. {}", r.title, r.hint).trim().to_string()),
            note: Some(note(title, r.body(), Level::Error, format!("{words} failed"))),
            ..RemoteOutcome::default()
        };
    }
    RemoteOutcome {
        note: Some(note(title, r.body(), Level::Error, format!("{words} failed"))),
        ..RemoteOutcome::default()
    }
}

pub fn state_text(state: &RepoState) -> (String, String, usize) {
    let n = state.conflicts.len();
    let what = match (state.op, state.step) {
        (Some(git::Op::Rebase), Some((s, total))) => format!("Rebasing: step {s} of {total}"),
        (Some(op), _) => format!("{} in progress", op.doing()),
        (None, _) => "Working in progress".into(),
    };
    let detail = if n > 0 {
        format!("{} with conflicts", plural(n, "file", "files"))
    } else {
        "No conflicts left".into()
    };
    (what, detail, n)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    Refuse(Note),
    Run(RemoteKind, RemoteOpts),
}

pub fn remote_plan(kind: RemoteKind, mut opts: RemoteOpts, has_remote: bool, branch: Option<&str>, detached: bool) -> Plan {
    let words = kind.word();
    if !has_remote {
        return Plan::Refuse(note(
            "This repo has no remote",
            "Add a remote in the repo settings first.",
            Level::Warning,
            "No remote",
        ));
    }
    opts.remote = opts.remote.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
    opts.branch = opts.branch.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
    if kind == RemoteKind::Fetch {
        return Plan::Run(kind, opts);
    }
    let Some(current) = branch.filter(|_| !detached) else {
        return Plan::Refuse(note(
            format!("{words} not started"),
            "You are not on a branch. Check out a branch first.",
            Level::Warning,
            format!("{words} not started"),
        ));
    };
    if kind == RemoteKind::Pull {
        return Plan::Run(kind, opts);
    }
    if opts.force_lease && opts.branch.is_none() {
        return Plan::Refuse(note(
            "Push not started",
            "The branch to force push is not known. Push again from the Changes panel.",
            Level::Warning,
            "Push not started",
        ));
    }
    if let Some(b) = &opts.branch
        && b != current {
            return Plan::Refuse(note(
                "Push not started",
                format!("You are on {current} now, not {b}. Nothing was pushed."),
                Level::Warning,
                "Push not started",
            ));
        }
    Plan::Run(kind, opts)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub summary: String,
    pub body: String,
    pub show_desc: bool,
}

pub fn draft_of(summary: &str, body: &str, show_desc: bool, amend: bool, before: Option<&Draft>) -> Option<Draft> {
    let (s, b, show) = if amend {
        let d = before?;
        (d.summary.clone(), d.body.clone(), d.show_desc)
    } else {
        (summary.to_string(), body.to_string(), show_desc)
    };
    if s.trim().is_empty() && b.trim().is_empty() {
        return None;
    }
    Some(Draft {
        show_desc: show || !b.trim().is_empty(),
        summary: s,
        body: b,
    })
}

pub fn leftover_note(paths: &[String], error: &str) -> Option<Note> {
    if paths.is_empty() {
        return None;
    }
    let names: Vec<&str> = paths.iter().take(4).map(|p| git::base_name(p)).collect();
    let more = if paths.len() > 4 { ", …" } else { "" };
    let mut body = format!(
        "{} still changed: {}{more}.\nA file may be locked or open in another app, or be a separate git repo inside this folder.",
        plural(paths.len(), "file is", "files are"),
        names.join(", ")
    );
    if !error.is_empty() {
        body.push('\n');
        body.push_str(error);
    }
    Some(note("Some changes are still there", body, Level::Warning, "Some changes are still there"))
}

pub fn identity_stop(kind: &str) -> Note {
    note(
        format!("{kind} not done"),
        "This repo needs a profile first. Your global git name and email are never used.",
        Level::Info,
        "Pick a profile first",
    )
}

pub fn pr_visible(id: Option<&git::Identity>, branch: Option<&str>, detached: bool, unborn: bool) -> bool {
    let (Some(id), Some(b)) = (id, branch) else { return false };
    if detached || unborn || !id.has_github() {
        return false;
    }
    match &id.default_branch {
        Some(d) => b != d,
        None => !matches!(b, "main" | "master"),
    }
}

pub fn fetch_due(minutes: u32, last: Option<std::time::Instant>) -> bool {
    minutes > 0
        && last.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(u64::from(minutes) * 60))
}

pub fn fetch_error_due(last: Option<std::time::Instant>) -> bool {
    last.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(3600))
}
