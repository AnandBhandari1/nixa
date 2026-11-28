use super::{accounts, ai, changes, history, tools};
use crate::links::brain::Link;
use crate::screens::events;
use crate::workspace::Workspace;
use gpui::{App, Entity, Global};
use nixa_git::ai::Opts;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Suggesting(Option<Arc<AtomicBool>>);

impl Global for Suggesting {}

pub fn init(cx: &mut App) {
    history::init(cx);
    let base = history::links::links(cx);
    history::links::set_links(
        history::links::Links {
            changed: refresh_changes,
            open_ai: Some(open_ai),
            open_rebase: Some(open_rebase),
            open_identity: Some(open_identity),
            open_folder: Some(open_folder),
            ..base
        },
        cx,
    );
}

pub fn toast(title: &str, body: &str, level: events::Level, cx: &mut App) {
    let notice = events::Notice::new(level, title.to_string()).body(body.to_string());
    let Some(ws) = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()) else { return };
    cx.defer(move |cx| {
        ws.update(cx, |ws, _, cx| events::notify(&ws.hub.events, notice, cx)).ok();
    });
}

pub fn refresh_changes(repo: &Path, cx: &mut App) {
    let repo = repo.to_path_buf();
    let Some(ws) = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()) else { return };
    cx.defer(move |cx| {
        let view = ws.read_with(cx, |ws, _| ws.changes.clone()).ok();
        if let Some(view) = view
            && view.read(cx).root.as_deref().map(PathBuf::from).is_some_and(|r| same_repo(&r, &repo))
        {
            changes::refresh_view(&view, cx);
        }
    });
}

pub fn same_repo(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}

fn open_settings(cx: &mut App) {
    cx.defer(|cx| crate::screens::settings::open(accounts::SETTINGS_PART, cx));
}

fn open_ai(kind: &str, repo: &Path, sha: &str, cx: &mut App) {
    let request = match kind {
        "review" => ai::Request::review_commit(repo, sha),
        _ => ai::Request::explain(repo, sha),
    };
    ai::open(request, cx);
}

fn open_rebase(repo: &Path, base: &str, _branch: Option<&str>, cx: &mut App) {
    let open = tools::rebase::Open {
        repo: Some(repo.to_path_buf()),
        base: Some(base.to_string()).filter(|b| !b.is_empty()),
    };
    tools::open_rebase(open, tools_host(), cx);
}

fn open_identity(_: &Path, cx: &mut App) {
    open_settings(cx);
}

fn open_folder(folder: &str, cx: &mut App) {
    accounts::clone_win::open_space(folder.to_string(), cx);
}

fn show_commit(repo: &Path, sha: &str, cx: &mut App) -> bool {
    history::open_graph(Some(repo.to_path_buf()), history::Then::Commit(sha.to_string()), cx);
    true
}

fn explain_commit(repo: &Path, sha: &str, cx: &mut App) -> bool {
    ai::open(ai::Request::explain(repo, sha), cx);
    true
}

fn open_file(repo: &Path, file: &str, cx: &mut App) -> bool {
    crate::screens::editor::open_at(repo, file, cx)
}

fn pick_identity(_: &Path, cx: &mut App) -> bool {
    open_settings(cx);
    true
}

fn tools_changed(repo: &Path, _: tools::host::Changed, cx: &mut App) {
    history::refresh_all(repo, cx);
    refresh_changes(repo, cx);
}

fn tools_notify(n: &tools::host::Notice, cx: &mut App) -> bool {
    let level = if n.warning { events::Level::Warning } else { events::Level::Success };
    super::announce::post(&n.title, &n.body, level, cx);
    true
}

pub fn explain_conflict(ask: &tools::host::ConflictAsk, cancel: &tools::run::Cancel) -> Result<tools::host::Answer, String> {
    let settings = ai::model::load_settings();
    let opts = Opts { cancel: cancel.clone(), ..Opts::default() };
    let sides = (!ask.ours.is_empty() || !ask.theirs.is_empty()).then_some((ask.ours.as_str(), ask.theirs.as_str()));
    nixa_git::ai::explain_conflict(&settings, &ask.root, &ask.file, sides, &opts)
        .map(|text| tools::host::Answer { text, agent: Some(settings.agent.clone()).filter(|a| !a.is_empty()) })
        .map_err(|e| ai::model::error_text(&e))
}

pub fn tools_host() -> tools::Host {
    tools::Host {
        show_commit,
        explain_commit,
        open_file,
        pick_identity,
        changed: tools_changed,
        notify: tools_notify,
        explain_conflict,
    }
}

pub fn open_window(window: &changes::GitWindow, cx: &mut App) {
    use changes::GitWindow;
    match window.clone() {
        GitWindow::Branches { repo } => history::branches::open(Some(PathBuf::from(repo)), cx),
        GitWindow::Stash { repo } => history::stash::open(Some(PathBuf::from(repo)), cx),
        GitWindow::PullRequest { repo, .. } => accounts::pr_win::open(Some(PathBuf::from(repo)), cx),
        GitWindow::Settings { .. } | GitWindow::AgentMenu { .. } => open_settings(cx),
        GitWindow::Conflict { repo, file } => {
            let open = tools::conflict::Open { repo: Some(PathBuf::from(repo)), file: Some(file) };
            tools::open_conflict(open, tools_host(), cx);
        }
        GitWindow::Blame { repo, file, rev } => {
            let open = tools::blame::Open { repo: Some(PathBuf::from(repo)), file: Some(file), rev: rev.unwrap_or_default(), line: None };
            tools::open_blame(open, tools_host(), cx);
        }
        GitWindow::FileHistory { repo, path } => history::open_graph(Some(PathBuf::from(repo)), history::Then::File(path), cx),
        GitWindow::Review { repo } => ai::open(ai::Request::review_staged(PathBuf::from(repo)), cx),
    }
}

pub fn stop_suggest(view: &Entity<changes::ChangesView>, cx: &mut App) {
    if let Some(flag) = cx.try_global::<Suggesting>().and_then(|s| s.0.clone()) {
        flag.store(true, Ordering::SeqCst);
    }
    changes::set_ai(view, false, None, cx);
}

pub fn suggest(view: Entity<changes::ChangesView>, repo: String, branch: Option<String>, link: Option<Link>, cx: &mut App) {
    let cancel = Arc::new(AtomicBool::new(false));
    cx.set_global(Suggesting(Some(cancel.clone())));
    let settings = ai::model::load_settings();
    let (label, _) = ai::model::agent_label(&settings);
    changes::set_ai(&view, true, Some(label), cx);
    let job = cx.background_executor().spawn(async move {
        let opts = Opts { cancel, ..Opts::default() };
        let result = nixa_git::ai::commit_message(&settings, Path::new(&repo), branch.as_deref(), &opts);
        let report = ai::voice::commit_result(&result);
        if let Some(link) = link {
            let _ = ai::voice::say(&link, &nixa_protocol::paths::data_dir(), &report);
        }
        (result, report)
    });
    cx.spawn(async move |cx| {
        let (result, report) = job.await;
        cx.update(|cx| {
            changes::set_ai(&view, false, None, cx);
            if let Ok(text) = &result {
                changes::fill_message(&view, &text.summary, &text.body, cx);
            }
            let level = match report.level {
                ai::voice::Level::Info => events::Level::Info,
                ai::voice::Level::Success => events::Level::Success,
                ai::voice::Level::Warning | ai::voice::Level::Error => events::Level::Warning,
            };
            toast(&report.title, &report.body, level, cx);
        })
        .ok();
    })
    .detach();
}
