use super::model::{
    self, Commit, LOG_FORMAT, MAX_COMMITS, MAX_DEEPER, MAX_TODO, REBASE_FLAGS, Step, Summary,
};
use crate::screens::git::tools::errors;
use crate::screens::git::tools::run::{self, Cancel, Out};
use crate::screens::git::tools::state::{self, Op, RepoState};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Busy,
    NoCommits,
    Invalid,
    UnknownBase,
    NotOnBranch,
    NoUpstream,
    NoBase,
    TooMany,
    Merges,
    Unmerged,
    MixedChanges,
    Ignored,
    HeadChanged,
    Nothing,
    Identity,
    TooBig,
    Conflict,
    EmptyJoin,
    Stopped,
    Cancelled,
    Timeout,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub ok: bool,
    pub kind: Option<Kind>,
    pub title: String,
    pub hint: String,
    pub detail: String,
    pub files: Vec<String>,
}

impl Note {
    pub fn text(&self) -> String {
        [self.title.as_str(), self.hint.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(". ")
            .replace("..", ".")
    }
}

pub fn success(title: &str) -> Note {
    Note {
        ok: true,
        title: title.into(),
        ..Note::default()
    }
}

pub fn refusal(kind: Kind, title: &str, hint: &str) -> Note {
    Note {
        ok: false,
        kind: Some(kind),
        title: title.into(),
        hint: hint.into(),
        ..Note::default()
    }
}

pub fn failure(text: &str) -> Note {
    let said = errors::describe(text);
    Note {
        ok: false,
        kind: Some(if said.kind == errors::Kind::Timeout {
            Kind::Timeout
        } else {
            Kind::Other
        }),
        title: said.title,
        hint: said.hint,
        detail: said.detail,
        files: Vec::new(),
    }
}

fn with_files(mut note: Note, files: Vec<String>) -> Note {
    note.detail = model::list_text(&files);
    note.files = files;
    note
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub note: Note,
    pub base: String,
    pub base_short: String,
    pub base_subject: String,
    pub base_tree: String,
    pub upstream: Option<String>,
    pub head: String,
    pub head_ref: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub count: usize,
    pub commits: Vec<Commit>,
    pub pushed: usize,
    pub dirty: bool,
    pub staged: bool,
}

impl Plan {
    pub fn ok(&self) -> bool {
        self.note.ok
    }
}

fn read(root: &Path, args: &[&str]) -> Out {
    run::read(root, args)
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn nul_list(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn resolve_base(
    root: &Path,
    base: Option<&str>,
    head: &str,
) -> Result<(String, Option<String>), Note> {
    if let Some(name) = base {
        if name.starts_with('-') || name.chars().any(char::is_whitespace) || name.is_empty() {
            return Err(refusal(Kind::Invalid, "That is not a commit id", ""));
        }
        let spec = format!("{name}^{{commit}}");
        let r = read(
            root,
            &["rev-parse", "-q", "--verify", "--end-of-options", &spec],
        );
        if !r.ok() {
            return Err(refusal(
                Kind::UnknownBase,
                "That commit is not in this repo",
                "Load the history again and pick a commit from it.",
            ));
        }
        let base = r.first_line();
        let in_line = read(root, &["merge-base", "--is-ancestor", &base, head]);
        if in_line.code == Some(1) {
            return Err(refusal(
                Kind::NotOnBranch,
                "That commit is not on this branch",
                "Pick a commit from the current branch.",
            ));
        }
        if !in_line.ok() {
            return Err(failure(&in_line.said()));
        }
        return Ok((base, None));
    }
    let up = read(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    let upstream = up.first_line();
    if !up.ok() || upstream.is_empty() {
        return Err(refusal(
            Kind::NoUpstream,
            "This branch has no remote branch to compare with",
            "Pick a commit in History and use Change commits from here.",
        ));
    }
    let mb = read(root, &["merge-base", "HEAD", "@{upstream}"]);
    let base = mb.first_line();
    if !mb.ok() || base.is_empty() {
        return Err(refusal(
            Kind::NoBase,
            &format!("This branch and {upstream} share no commits"),
            "Pick a commit in History and use Change commits from here.",
        ));
    }
    Ok((base, Some(upstream)))
}

const IGNORED_LIST: [&str; 6] = [
    "ls-files",
    "-z",
    "-o",
    "-i",
    "--exclude-standard",
    "--full-name",
];

fn ignored_in_danger(root: &Path, base: &str, head: &str) -> Result<Vec<String>, Note> {
    let mut args = IGNORED_LIST.to_vec();
    args.extend(["--directory", "--no-empty-directory", "--", ":(top)"]);
    let listed = read(root, &args);
    if !listed.ok() {
        return Err(failure(&listed.said()));
    }
    let entries = nul_list(&listed.stdout);
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let range = format!("{base}..{head}");
    let log = read(
        root,
        &[
            "log",
            "-z",
            "--format=",
            "--name-only",
            "--no-renames",
            "--no-relative",
            &range,
        ],
    );
    if !log.ok() {
        return Err(failure(&log.said()));
    }
    let tree = read(
        root,
        &["ls-tree", "-r", "-z", "--name-only", "--full-tree", base],
    );
    if !tree.ok() {
        return Err(failure(&tree.said()));
    }
    let mut touched = nul_list(&log.stdout);
    touched.extend(nul_list(&tree.stdout));
    let (risky, deeper) = model::ignored_at_risk(&entries, &touched);
    let mut files = risky;
    files.extend(deeper.iter().skip(MAX_DEEPER).cloned());
    let near: Vec<String> = deeper.into_iter().take(MAX_DEEPER).collect();
    if !near.is_empty() {
        let specs: Vec<String> = near.iter().map(|d| format!(":(top,literal){d}")).collect();
        let mut args = IGNORED_LIST.to_vec();
        args.push("--");
        args.extend(specs.iter().map(String::as_str));
        let inner = read(root, &args);
        if !inner.ok() {
            return Err(failure(&inner.said()));
        }
        let (r2, d2) = model::ignored_at_risk(&nul_list(&inner.stdout), &touched);
        files.extend(r2);
        files.extend(d2);
    }
    files.sort();
    files.dedup();
    Ok(files)
}

pub fn rebase_plan(root: &Path, base: Option<&str>) -> Plan {
    let state = match state::read_state(root) {
        Ok(s) => s,
        Err(e) => {
            return Plan {
                note: failure(&e),
                ..Plan::default()
            };
        }
    };
    if let Some(op) = state.op {
        return Plan {
            note: refusal(
                Kind::Busy,
                &format!("A {} is in progress", op.word()),
                "Finish it or cancel it in the Changes panel first.",
            ),
            ..Plan::default()
        };
    }
    let head_r = read(root, &["rev-parse", "-q", "--verify", "HEAD^{commit}"]);
    let head = head_r.first_line();
    if !head_r.ok() || head.is_empty() {
        return Plan {
            note: refusal(Kind::NoCommits, "This repo has no commits yet", ""),
            ..Plan::default()
        };
    }
    let (base, upstream) = match resolve_base(root, base, &head) {
        Ok(found) => found,
        Err(note) => {
            return Plan {
                note,
                ..Plan::default()
            };
        }
    };
    let base_r = read(root, &["log", "-1", "--format=%h%x00%T%x00%s", &base]);
    if !base_r.ok() {
        return Plan {
            note: failure(&base_r.said()),
            ..Plan::default()
        };
    }
    let base_line = base_r.stdout.trim_end_matches('\n').to_string();
    let mut base_parts = base_line.split('\0');
    let base_short = base_parts.next().unwrap_or_default().to_string();
    let base_tree = base_parts.next().unwrap_or_default().to_string();
    let base_subject = base_parts.next().unwrap_or_default().to_string();
    let range = format!("{base}..{head}");
    let count_r = read(root, &["rev-list", "--count", &range]);
    if !count_r.ok() {
        return Plan {
            note: failure(&count_r.said()),
            ..Plan::default()
        };
    }
    let count: usize = count_r.first_line().parse().unwrap_or(0);
    let ref_r = read(root, &["symbolic-ref", "-q", "HEAD"]);
    let head_ref = Some(ref_r.first_line()).filter(|r| ref_r.ok() && r.starts_with("refs/heads/"));
    let mut plan = Plan {
        note: Note::default(),
        base: base.clone(),
        base_short,
        base_subject,
        base_tree: base_tree.clone(),
        upstream,
        head: head.clone(),
        detached: head_ref.is_none(),
        head_ref,
        branch: state.branch.clone(),
        count,
        ..Plan::default()
    };
    if count > MAX_COMMITS {
        plan.note = refusal(
            Kind::TooMany,
            &format!("There are {count} commits to change"),
            &format!("This window can change up to {MAX_COMMITS}. Pick a newer commit in History."),
        );
        return plan;
    }
    let format = format!("--format={LOG_FORMAT}");
    let log_r = read(
        root,
        &[
            "-c",
            "core.quotepath=off",
            "log",
            "--topo-order",
            "--reverse",
            "--no-color",
            &format,
            &range,
        ],
    );
    if !log_r.ok() {
        plan.note = failure(&log_r.said());
        return plan;
    }
    let raw = model::parse_log(&log_r.stdout);
    let mut trees: HashMap<String, String> = raw
        .iter()
        .map(|c| (c.sha.clone(), c.tree.clone()))
        .collect();
    trees.insert(base.clone(), base_tree);
    let unpushed_r = read(root, &["rev-list", &range, "--not", "--remotes"]);
    let unpushed: Vec<String> = if unpushed_r.ok() {
        lines(&unpushed_r.stdout)
    } else {
        raw.iter().map(|c| c.sha.clone()).collect()
    };
    plan.commits = raw
        .into_iter()
        .map(|mut c| {
            c.pushed = !unpushed.contains(&c.sha);
            c.empty = !c.is_merge && c.parents.first().and_then(|p| trees.get(p)) == Some(&c.tree);
            c
        })
        .collect();
    let status_r = read(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=no",
            "--ignore-submodules=all",
        ],
    );
    if !status_r.ok() {
        plan.note = failure(&status_r.said());
        return plan;
    }
    let local = model::local_changes(&model::parse_status_z(&status_r.stdout));
    plan.pushed = plan.commits.iter().filter(|c| c.pushed).count();
    plan.dirty = local.dirty;
    plan.staged = local.staged;
    if plan.commits.iter().any(|c| c.is_merge) {
        plan.note = refusal(
            Kind::Merges,
            "There is a merge in these commits",
            "This window cannot change commits across a merge. Pick a commit after the merge.",
        );
        return plan;
    }
    if plan.commits.is_empty() {
        plan.note = success("No commits after this one");
        return plan;
    }
    if !local.unmerged.is_empty() {
        plan.note = with_files(
            refusal(
                Kind::Unmerged,
                "Some files still have a conflict",
                "Fix them in the Changes panel first.",
            ),
            local.unmerged,
        );
        return plan;
    }
    if !local.mixed.is_empty() {
        plan.note = with_files(
            refusal(
                Kind::MixedChanges,
                "Some files have staged and unstaged parts",
                "Commit or stash them first.",
            ),
            local.mixed,
        );
        return plan;
    }
    match ignored_in_danger(root, &base, &head) {
        Err(note) => plan.note = note,
        Ok(files) if !files.is_empty() => {
            plan.note = with_files(
                refusal(
                    Kind::Ignored,
                    "These files are ignored by git and would be lost",
                    "Git would write over them while it changes the commits. Move them out of the repo folder first, then press Load again.",
                ),
                files,
            )
        }
        Ok(_) => {
            plan.note = success(&format!(
                "{} to change",
                model::plural(plan.commits.len(), "commit")
            ))
        }
    }
    plan
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub note: Note,
    pub clean: bool,
    pub empty: Vec<String>,
    pub problems: Vec<model::StepProblem>,
}

fn checked_plan(
    root: &Path,
    base: Option<&str>,
    steps: &[Step],
    expect_head: &str,
) -> Result<Plan, Box<(Note, Vec<model::StepProblem>)>> {
    let plan = rebase_plan(root, base);
    if !plan.ok() {
        return Err(Box::new((plan.note, Vec::new())));
    }
    if !expect_head.is_empty() && plan.head != expect_head {
        return Err(Box::new((
            refusal(
                Kind::HeadChanged,
                "The branch changed since this window loaded",
                "Press Load again, then make your changes again.",
            ),
            Vec::new(),
        )));
    }
    let check = model::validate(steps, Some(&plan.commits));
    if !check.ok() {
        return Err(Box::new((
            refusal(Kind::Invalid, &check.problem().unwrap_or_default(), ""),
            check.problems,
        )));
    }
    Ok(plan)
}

pub fn preview(root: &Path, base: Option<&str>, steps: &[Step], expect_head: &str) -> Preview {
    let plan = match checked_plan(root, base, steps, expect_head) {
        Ok(plan) => plan,
        Err(refused) => {
            let (note, problems) = *refused;
            return Preview {
                note,
                problems,
                ..Preview::default()
            };
        }
    };
    let map = model::by_sha(&plan.commits);
    let mut tree = plan.base_tree.clone();
    let mut before = plan.base_tree.clone();
    let mut empty: Vec<String> = Vec::new();
    for step in steps {
        if step.action == model::Action::Drop {
            continue;
        }
        let Some(commit) = map.get(step.sha.as_str()) else {
            continue;
        };
        let parent = commit.parents.first().cloned().unwrap_or_default();
        let merge_base = format!("--merge-base={parent}");
        let r = run::git(
            root,
            &[
                "-c",
                "core.quotepath=off",
                "merge-tree",
                "--write-tree",
                "--name-only",
                "--no-messages",
                &merge_base,
                &tree,
                &commit.sha,
            ],
        );
        if r.code == Some(1) {
            let files = model::conflict_files(&r.stdout);
            let detail = if files.is_empty() {
                String::new()
            } else {
                let shown = files.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
                let more = if files.len() > 8 {
                    format!(" and {} more", files.len() - 8)
                } else {
                    String::new()
                };
                format!("Files: {shown}{more}")
            };
            return Preview {
                note: Note {
                    ok: true,
                    kind: Some(Kind::Conflict),
                    title: format!("{} will have a conflict", commit.short),
                    hint: "Move it to another place, drop it, or fix the conflict when git stops there.".into(),
                    detail,
                    files,
                },
                clean: false,
                empty,
                problems: Vec::new(),
            };
        }
        if !r.ok() {
            return Preview {
                note: failure(&r.said()),
                ..Preview::default()
            };
        }
        let next = r.first_line();
        if !model::is_sha(&next) {
            let said = r.said();
            return Preview {
                note: failure(if said.is_empty() {
                    "git merge-tree gave no result"
                } else {
                    &said
                }),
                ..Preview::default()
            };
        }
        let joins = step.action.joins();
        if joins && next == before {
            return Preview {
                note: Note {
                    ok: true,
                    kind: Some(Kind::EmptyJoin),
                    title: format!("{} will stop the rebase", commit.short),
                    hint: "Joining it into the commit above leaves no changes at all, so git stops there. Drop both commits instead.".into(),
                    ..Note::default()
                },
                clean: false,
                empty,
                problems: Vec::new(),
            };
        }
        if !joins {
            before = tree.clone();
        }
        if next == tree && !commit.empty && !joins {
            empty.push(commit.short.clone());
        }
        tree = next;
    }
    let head_tree = read(root, &["rev-parse", &format!("{}^{{tree}}", plan.head)]).first_line();
    let mut notes = vec![if tree == head_tree {
        "The files at the end stay the same as now.".to_string()
    } else {
        "The files at the end will be different from now.".to_string()
    }];
    if !empty.is_empty() {
        let one = empty.len() == 1;
        notes.push(format!(
            "{} will have no changes left, because {} already there. {}: {}. Drop {} if you do not want {}.",
            model::plural(empty.len(), "commit"),
            if one { "its changes are" } else { "their changes are" },
            if one { "It stays as an empty commit" } else { "They stay as empty commits" },
            empty.join(", "),
            if one { "it" } else { "them" },
            if one { "it" } else { "them" },
        ));
    }
    Preview {
        note: Note {
            ok: true,
            title: "No conflicts expected".into(),
            hint: notes.join(" "),
            ..Note::default()
        },
        clean: true,
        empty,
        problems: Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub note: Note,
    pub state: Option<RepoState>,
    pub summary: Option<Summary>,
    pub needs_force: bool,
    pub warning: bool,
    pub problems: Vec<model::StepProblem>,
}

impl Outcome {
    pub fn locked(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.op.is_some())
            || matches!(self.note.kind, Some(Kind::Conflict) | Some(Kind::Stopped))
    }
}

fn from_note(note: Note) -> Outcome {
    Outcome {
        note,
        ..Outcome::default()
    }
}

pub fn has_identity(root: &Path) -> bool {
    let email = run::git(root, &["config", "user.email"]);
    let name = run::git(root, &["config", "user.name"]);
    email.ok() && !email.first_line().is_empty() && name.ok() && !name.first_line().is_empty()
}

pub struct TempDir(pub PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        if self.0.to_string_lossy().contains("nixa-git-rebase.") {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

pub fn make_temp(todo: &str, check: &str) -> Result<TempDir, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("nixa-git-rebase.{}-{stamp}", std::process::id()));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&dir)
        .map_err(|e| format!("cannot make a folder for the step list: {e}"))?;
    let temp = TempDir(dir);
    fs::write(temp.0.join("todo"), todo).map_err(|e| e.to_string())?;
    fs::write(temp.0.join("check"), check).map_err(|e| e.to_string())?;
    Ok(temp)
}

fn head_now(root: &Path) -> Option<String> {
    let r = run::git(root, &["rev-parse", "-q", "--verify", "HEAD"]);
    let sha = r.first_line();
    (r.ok() && !sha.is_empty()).then_some(sha)
}

const AUTOSTASH_CLASH: &str = "applying autostash resulted in conflicts";

fn todo_broken(text: &str) -> bool {
    let low = text.to_lowercase();
    low.contains("could not parse")
        || low.contains("unknown command")
        || low.contains("cannot squash without a previous commit")
        || low.contains("please fix this using 'git rebase --edit-todo'")
        || (low.contains("invalid line ") && low.chars().any(|c| c.is_ascii_digit()))
}

pub fn run_rebase(
    root: &Path,
    base: Option<&str>,
    steps: &[Step],
    expect_head: &str,
    cancel: Option<&Cancel>,
) -> Outcome {
    let plan = match checked_plan(root, base, steps, expect_head) {
        Ok(plan) => plan,
        Err(refused) => {
            let (note, problems) = *refused;
            return Outcome {
                note,
                problems,
                ..Outcome::default()
            };
        }
    };
    if model::is_unchanged(steps, &plan.commits) {
        return from_note(refusal(
            Kind::Nothing,
            "Nothing to change",
            "Pick another action or move a commit first.",
        ));
    }
    if !has_identity(root) {
        return from_note(refusal(
            Kind::Identity,
            "Pick a profile for this repo first",
            "Then press Apply again.",
        ));
    }
    let todo = match model::build_todo(steps, &plan.commits) {
        Ok(todo) => todo,
        Err(e) => return from_note(refusal(Kind::Invalid, &e, "")),
    };
    if todo.len() > MAX_TODO {
        return from_note(refusal(
            Kind::TooBig,
            "The new messages are too long",
            "Make the messages shorter, or change fewer commits at once.",
        ));
    }
    let want: Vec<String> = plan.commits.iter().map(|c| c.sha.clone()).collect();
    let check = model::check_script(&want, &plan.head, plan.head_ref.as_deref());
    let temp = match make_temp(&todo, &check) {
        Ok(t) => t,
        Err(e) => return from_note(failure(&e)),
    };
    let dir = temp.0.to_string_lossy().to_string();
    let editor = format!(
        "/bin/sh {} {}",
        model::shell_quote(&format!("{dir}/check")),
        model::shell_quote(&dir)
    );
    let mut args = vec!["rebase", "-i"];
    args.extend(REBASE_FLAGS);
    args.push(&plan.base);
    let r = run::git_with(
        root,
        &args,
        &[("GIT_SEQUENCE_EDITOR".to_string(), editor)],
        run::JOB_WAIT,
        cancel,
    );
    drop(temp);
    let summary = model::summarize(steps, &plan.commits);
    let after = state::read_state(root).ok();
    let head = head_now(root);
    let moved = head.as_deref() != Some(plan.head.as_str());
    let rebasing = after.as_ref().is_some_and(|s| s.op == Some(Op::Rebase));
    if r.cancelled || r.timed_out {
        let mut note = if r.cancelled {
            refusal(Kind::Cancelled, "Stopped", "")
        } else {
            failure("timed out waiting")
        };
        note.hint = if rebasing {
            "The rebase stopped part way. Continue or cancel it in the Changes panel.".into()
        } else if moved {
            "The commits may have changed. Check the history.".into()
        } else {
            "Nothing was changed.".into()
        };
        return Outcome {
            note,
            state: after,
            ..Outcome::default()
        };
    }
    let text = r.said();
    if rebasing {
        let has_conflicts = after.as_ref().is_some_and(|s| !s.conflicts.is_empty());
        if has_conflicts {
            return Outcome {
                note: Note {
                    ok: false,
                    kind: Some(Kind::Conflict),
                    title: "Stopped on a conflict".into(),
                    hint: "Fix it in the Changes panel, then press Continue.".into(),
                    detail: errors::describe(&text).detail,
                    files: Vec::new(),
                },
                state: after,
                summary: Some(summary),
                ..Outcome::default()
            };
        }
        if !r.ok() && todo_broken(&text) {
            let undo = run::git(root, &["rebase", "--abort"]);
            if undo.ok() {
                let mut note = refusal(
                    Kind::Invalid,
                    "Git did not take the step list",
                    "Nothing was changed.",
                );
                note.detail = errors::describe(&text).detail;
                return from_note(note);
            }
        }
        return Outcome {
            note: Note {
                ok: false,
                kind: Some(Kind::Stopped),
                title: "The rebase stopped part way".into(),
                hint: "Continue or cancel it in the Changes panel.".into(),
                detail: errors::describe(&text).detail,
                files: Vec::new(),
            },
            state: after,
            summary: Some(summary),
            ..Outcome::default()
        };
    }
    let clash = text.to_lowercase().contains(AUTOSTASH_CLASH);
    if !r.ok() && text.contains(model::CHANGED_MARK) {
        let kept = if clash {
            " Your own changes are kept in the stash."
        } else {
            ""
        };
        return Outcome {
            note: refusal(
                Kind::HeadChanged,
                "The branch changed while starting",
                &format!("Nothing was changed.{kept} Load again."),
            ),
            state: after,
            ..Outcome::default()
        };
    }
    if !r.ok() {
        return Outcome {
            note: failure(&text),
            state: after,
            ..Outcome::default()
        };
    }
    let force = summary.pushed_changed;
    if clash {
        let hint = format!(
            "Your own changes did not fit with the new commits. They are kept in the stash. Open the stash window to get them back. {}",
            model::force_hint(force)
        );
        return Outcome {
            note: Note {
                ok: true,
                title: "Commits changed, but your changes clashed".into(),
                hint: hint.trim().to_string(),
                ..Note::default()
            },
            summary: Some(summary),
            needs_force: force > 0,
            warning: true,
            ..Outcome::default()
        };
    }
    Outcome {
        note: Note {
            ok: true,
            title: "Commits changed".into(),
            hint: model::force_hint(force),
            ..Note::default()
        },
        summary: Some(summary),
        needs_force: force > 0,
        ..Outcome::default()
    }
}
