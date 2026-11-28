use super::git::{self, Done, Facts, FileEntry, Identity, PullMode, Remote, base_name, folder_of};
use super::words::{Level, RemoteKind, RemoteOpts, note};
use super::{Action, ChangesEvent, ChangesView, GitWindow, Popup};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Corner, Entity, FontWeight, MouseButton, SharedString, div, px,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::modal::{dialog, overlay};
use nixa_ui::popover::{float_at, popover, tooltip};
use nixa_ui::text_input::TextInput;
use nixa_ui::{Icon, Theme, icon, theme};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use nixa_git::accounts::profiles::Profile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushItem {
    PushTo(String),
    PushTags,
    PushAllTags,
    ForcePush,
    Pull(PullMode),
    FetchAll,
    CheckPull,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item<T> {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub off: Option<String>,
    pub danger: bool,
    pub what: T,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line<T> {
    Section(&'static str),
    Item(Item<T>),
    Gap,
}

pub struct PushMenu {
    pub repo: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub has_remote: bool,
    pub mode: PullMode,
    pub remotes: Option<Vec<Remote>>,
    pub hi: Option<usize>,
    pub ask_force: bool,
    pub busy: Option<String>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub result: Option<Done>,
    pub error: Option<Done>,
    pub check: Option<Done>,
    pub note: Option<String>,
}

pub fn push_menu(
    repo: String,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    has_remote: bool,
    mode: PullMode,
) -> PushMenu {
    PushMenu {
        repo,
        branch,
        upstream,
        ahead,
        behind,
        has_remote,
        mode,
        remotes: None,
        hi: None,
        ask_force: false,
        busy: None,
        cancel: None,
        result: None,
        error: None,
        check: None,
        note: None,
    }
}

fn item<T>(id: &str, label: impl Into<String>, detail: impl Into<String>, off: Option<String>, what: T) -> Line<T> {
    Line::Item(Item {
        id: id.to_string(),
        label: label.into(),
        detail: detail.into(),
        off,
        danger: false,
        what,
    })
}

pub fn push_blocked(m: &PushMenu) -> Option<String> {
    (!m.has_remote).then(|| "This repo has no remote. Add one in the repo settings.".to_string())
}

pub fn main_remote(m: &PushMenu) -> Option<String> {
    let list = m.remotes.as_deref().unwrap_or(&[]);
    git::remote_of_upstream(m.upstream.as_deref(), list).or_else(|| {
        if list.iter().any(|r| r.name == "origin") {
            Some("origin".into())
        } else {
            list.first().map(|r| r.name.clone())
        }
    })
}

pub fn push_lines(m: &PushMenu) -> Vec<Line<PushItem>> {
    let blocked = push_blocked(m);
    let no_branch = m.branch.is_none().then(|| "Check out a branch first.".to_string());
    let no_up = m
        .upstream
        .is_none()
        .then(|| "This branch is not on the remote yet. Push it once first.".to_string());
    let off = |a: &Option<String>, b: &Option<String>, c: &Option<String>| a.clone().or(b.clone()).or(c.clone());
    let remotes = m.remotes.as_deref().unwrap_or(&[]);
    let upstream_remote = git::remote_of_upstream(m.upstream.as_deref(), remotes);
    let mut out = vec![Line::Section("Push")];
    if remotes.len() > 1 {
        for r in remotes {
            let detail = if Some(&r.name) == upstream_remote.as_ref() {
                "Where this branch goes now".to_string()
            } else {
                r.url.clone()
            };
            out.push(item(
                &format!("push-{}", r.name),
                format!("Push to {}", r.name),
                detail,
                off(&blocked, &no_branch, &None),
                PushItem::PushTo(r.name.clone()),
            ));
        }
    }
    out.push(item(
        "push-tags",
        "Push with tags",
        "Also sends the tags on the commits you push",
        off(&blocked, &no_branch, &None),
        PushItem::PushTags,
    ));
    let main = main_remote(m);
    out.push(item(
        "push-all-tags",
        match &main {
            Some(r) => format!("Push all tags to {r}"),
            None => "Push all tags".into(),
        },
        "Sends every tag in this repo",
        blocked.clone(),
        PushItem::PushAllTags,
    ));
    if let Some(Line::Item(i)) = Some(item(
        "push-force",
        "Force push (safe, with lease)…",
        "Replaces the remote branch with yours",
        off(&blocked, &no_branch, &no_up),
        PushItem::ForcePush,
    )) {
        out.push(Line::Item(Item { danger: true, ..i }));
    }
    out.push(Line::Section("Pull, this time only"));
    for mode in [PullMode::Rebase, PullMode::Merge, PullMode::FfOnly] {
        let detail = match mode {
            PullMode::Rebase => "Your new commits go on top",
            PullMode::Merge => "Makes a merge commit when needed",
            PullMode::FfOnly => "Only when you have no new commits",
        };
        let detail = if mode == m.mode {
            format!("{detail}. Your usual setting.")
        } else {
            detail.to_string()
        };
        out.push(item(
            &format!("pull-{}", mode.word().replace(' ', "-")),
            mode.tip(),
            detail,
            off(&blocked, &no_branch, &no_up),
            PushItem::Pull(mode),
        ));
    }
    out.push(Line::Section("Fetch"));
    out.push(item(
        "fetch-all",
        "Fetch all remotes",
        "Also removes branches deleted on the remote, and gets tags",
        blocked.clone(),
        PushItem::FetchAll,
    ));
    out.push(item(
        "check-pull",
        "Check for pull conflicts",
        "Fetches, then checks. Changes nothing here.",
        off(&blocked, &no_branch, &no_up),
        PushItem::CheckPull,
    ));
    out
}

pub fn load_remotes(_: &mut ChangesView, root: String, cx: &mut Context<ChangesView>) {
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::list_remotes(&root) }
    });
    cx.spawn(async move |this, cx| {
        let list = job.await;
        this.update(cx, |v, cx| {
            if let Some(Popup::Push { menu, .. }) = v.popup.as_mut()
                && menu.repo == root {
                    menu.remotes = Some(list);
                    cx.notify();
                }
        })
        .ok();
    })
    .detach();
}

fn push_menu_mut(v: &mut ChangesView) -> Option<&mut PushMenu> {
    match v.popup.as_mut() {
        Some(Popup::Push { menu, .. }) => Some(menu.as_mut()),
        _ => None,
    }
}

pub fn choose_push(v: &mut ChangesView, what: PushItem, cx: &mut Context<ChangesView>) {
    let Some(m) = push_menu_mut(v) else { return };
    if m.busy.is_some() {
        return;
    }
    m.ask_force = false;
    let branch = m.branch.clone();
    let send = |v: &mut ChangesView, kind: RemoteKind, opts: RemoteOpts, cx: &mut Context<ChangesView>| {
        v.popup = None;
        super::remote_op(v, kind, opts, cx);
    };
    match what {
        PushItem::PushTo(remote) => send(
            v,
            RemoteKind::Push,
            RemoteOpts {
                remote: Some(remote),
                branch,
                ..RemoteOpts::default()
            },
            cx,
        ),
        PushItem::PushTags => send(
            v,
            RemoteKind::Push,
            RemoteOpts {
                tags: true,
                branch,
                ..RemoteOpts::default()
            },
            cx,
        ),
        PushItem::ForcePush => {
            m.ask_force = true;
            m.result = None;
            m.error = None;
            cx.notify();
        }
        PushItem::Pull(mode) => send(
            v,
            RemoteKind::Pull,
            RemoteOpts {
                mode: Some(mode),
                ..RemoteOpts::default()
            },
            cx,
        ),
        PushItem::FetchAll => send(
            v,
            RemoteKind::Fetch,
            RemoteOpts {
                all: true,
                prune: Some(true),
                tags: true,
                ..RemoteOpts::default()
            },
            cx,
        ),
        PushItem::PushAllTags => run_tags(v, cx),
        PushItem::CheckPull => run_check(v, cx),
    }
}

pub fn force_yes(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(m) = push_menu_mut(v) else { return };
    let branch = m.branch.clone();
    v.popup = None;
    super::remote_op(
        v,
        RemoteKind::Push,
        RemoteOpts {
            force_lease: true,
            branch,
            ..RemoteOpts::default()
        },
        cx,
    );
}

fn start_menu_job(m: &mut PushMenu, busy: &str) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    m.busy = Some(busy.to_string());
    m.cancel = Some(cancel.clone());
    m.result = None;
    m.error = None;
    m.check = None;
    m.note = None;
    m.ask_force = false;
    cancel
}

fn run_tags(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(m) = push_menu_mut(v) else { return };
    let remote = main_remote(m);
    let root = m.repo.clone();
    let cancel = start_menu_job(m, "Pushing all tags…");
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        let cancel = cancel.clone();
        async move {
            if !git::read_identity(&root).ready() {
                return git::refusal(git::Kind::Identity, "Pick a profile for this repo first", "Then push the tags again.");
            }
            git::push_tags(&root, remote.as_deref(), &cancel)
        }
    });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            let Some(m) = push_menu_mut(v) else { return };
            if m.repo != root {
                return;
            }
            m.busy = None;
            m.cancel = None;
            if r.ok {
                m.result = Some(r);
                super::refresh(v, cx);
            } else if r.kind == git::Kind::Cancelled {
                m.note = Some("Stopped. Some tags may have been sent already.".into());
            } else {
                m.error = Some(r);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn run_check(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(m) = push_menu_mut(v) else { return };
    let root = m.repo.clone();
    let mode = m.mode;
    let cancel = start_menu_job(m, "Fetching and checking…");
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        let cancel = cancel.clone();
        async move {
            let fetched = git::fetch(&root, &git::FetchOpts { prune: true, ..git::FetchOpts::default() }, &cancel);
            if fetched.kind == git::Kind::Cancelled || cancel.load(Ordering::Relaxed) {
                return (fetched, None);
            }
            (fetched, Some(git::preview_pull(&root, mode)))
        }
    });
    cx.spawn(async move |this, cx| {
        let (fetched, check) = job.await;
        this.update(cx, |v, cx| {
            let Some(m) = push_menu_mut(v) else { return };
            if m.repo != root {
                return;
            }
            m.busy = None;
            m.cancel = None;
            match check {
                None => m.note = Some("Stopped. Nothing was checked.".into()),
                Some(c) => {
                    if !fetched.ok {
                        m.note = Some(format!(
                            "Could not fetch, so this uses what was fetched before. {}.",
                            fetched.title
                        ));
                    }
                    if c.ok {
                        m.check = Some(c);
                    } else {
                        m.error = Some(c);
                    }
                }
            }
            if fetched.ok {
                super::refresh(v, cx);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn stop_menu_job(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(m) = push_menu_mut(v) else { return };
    if let Some(c) = &m.cancel {
        c.store(true, Ordering::Relaxed);
        m.busy = Some("Stopping…".into());
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileItem {
    Conflict,
    Open,
    History,
    Blame,
    CopyPath,
    CopyFull,
    Finder,
    Ignore,
}

pub struct FileMenu {
    pub repo: String,
    pub file: String,
    pub status: char,
    pub staged: bool,
    pub conflict: bool,
    pub facts: Option<Facts>,
    pub hi: Option<usize>,
    pub ask: bool,
    pub busy: bool,
    pub error: Option<String>,
}

pub fn file_menu(repo: String, f: &FileEntry) -> FileMenu {
    FileMenu {
        repo,
        file: f.path.clone(),
        status: f.status,
        staged: f.staged,
        conflict: f.conflict,
        facts: None,
        hi: None,
        ask: false,
        busy: false,
        error: None,
    }
}

pub fn full_path(repo: &str, file: &str) -> String {
    format!("{}/{file}", repo.trim_end_matches('/'))
}

pub fn file_lines(m: &FileMenu) -> Vec<Line<FileItem>> {
    let deleted = m.status == 'D';
    let committed = m.facts.is_none_or(|f| f.in_head);
    let blameable = m.facts.is_none_or(|f| f.in_head || f.tracked);
    let mut out = Vec::new();
    if m.conflict {
        out.push(item("conflict", "Fix the conflict…", "Open the window that helps you pick each side", None, FileItem::Conflict));
    }
    out.push(item(
        "open",
        "Open in editor",
        "",
        deleted.then(|| "The file is not in the folder".to_string()),
        FileItem::Open,
    ));
    out.push(item(
        "history",
        "History of this file",
        "",
        (!committed).then(|| "This file has no commits yet".to_string()),
        FileItem::History,
    ));
    out.push(item(
        "blame",
        "Who changed each line",
        "",
        (!blameable).then(|| "This file has no commits yet".to_string()),
        FileItem::Blame,
    ));
    out.push(Line::Gap);
    out.push(item("copy", "Copy path", "", None, FileItem::CopyPath));
    out.push(item("copy-full", "Copy full path", "", None, FileItem::CopyFull));
    out.push(item(
        "finder",
        "Show in Finder",
        "",
        deleted.then(|| "The file is not in the folder".to_string()),
        FileItem::Finder,
    ));
    out.push(Line::Gap);
    let ignore_off = match m.facts {
        None => Some("Checking the file…".to_string()),
        Some(f) if f.tracked => Some(
            "Git already tracks this file, so ignoring it does nothing. This works only for new files.".to_string(),
        ),
        Some(_) => None,
    };
    out.push(item("ignore", "Ignore on this computer…", "", ignore_off, FileItem::Ignore));
    out
}

pub fn load_facts(_: &mut ChangesView, root: String, file: String, cx: &mut Context<ChangesView>) {
    let job = cx.background_executor().spawn({
        let root = root.clone();
        let file = file.clone();
        async move { git::file_facts(&root, &file) }
    });
    cx.spawn(async move |this, cx| {
        let facts = job.await;
        this.update(cx, |v, cx| {
            if let Some(Popup::File { menu, .. }) = v.popup.as_mut()
                && menu.repo == root && menu.file == file {
                    menu.facts = Some(facts);
                    cx.notify();
                }
        })
        .ok();
    })
    .detach();
}

fn file_menu_mut(v: &mut ChangesView) -> Option<&mut FileMenu> {
    match v.popup.as_mut() {
        Some(Popup::File { menu, .. }) => Some(menu.as_mut()),
        _ => None,
    }
}

pub fn choose_file(v: &mut ChangesView, what: FileItem, cx: &mut Context<ChangesView>) {
    let Some(m) = file_menu_mut(v) else { return };
    if m.busy {
        return;
    }
    let repo = m.repo.clone();
    let file = m.file.clone();
    let deleted = m.status == 'D';
    match what {
        FileItem::Ignore => {
            m.ask = true;
            m.error = None;
            cx.notify();
            return;
        }
        FileItem::Finder => {
            m.busy = true;
            let full = full_path(&repo, &file);
            let job = cx.background_executor().spawn(async move {
                std::process::Command::new("/usr/bin/open")
                    .args(["-R", &full])
                    .output()
                    .map_err(|e| e.to_string())
                    .and_then(|o| {
                        if o.status.success() {
                            Ok(())
                        } else {
                            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
                        }
                    })
            });
            cx.spawn(async move |this, cx| {
                let r = job.await;
                this.update(cx, |v, cx| {
                    match r {
                        Ok(()) => v.popup = None,
                        Err(e) => {
                            if let Some(m) = file_menu_mut(v) {
                                m.busy = false;
                                m.error = Some(format!("Could not show it in Finder: {e}"));
                            }
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
            cx.notify();
            return;
        }
        FileItem::CopyPath => cx.write_to_clipboard(ClipboardItem::new_string(file.clone())),
        FileItem::CopyFull => cx.write_to_clipboard(ClipboardItem::new_string(full_path(&repo, &file))),
        FileItem::Conflict => cx.emit(ChangesEvent::Open(GitWindow::Conflict { repo, file })),
        FileItem::Open => cx.emit(ChangesEvent::OpenFile { repo, path: file }),
        FileItem::History => cx.emit(ChangesEvent::Open(GitWindow::FileHistory { repo, path: file })),
        FileItem::Blame => cx.emit(ChangesEvent::Open(GitWindow::Blame {
            repo,
            file,
            rev: deleted.then(|| "HEAD".to_string()),
        })),
    }
    v.popup = None;
    cx.notify();
}

pub fn ignore_yes(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(m) = file_menu_mut(v) else { return };
    if m.busy {
        return;
    }
    m.busy = true;
    let repo = m.repo.clone();
    let file = m.file.clone();
    cx.notify();
    let job = cx.background_executor().spawn({
        let repo = repo.clone();
        let file = file.clone();
        async move { git::exclude_locally(&repo, &file) }
    });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            if r.ok {
                v.popup = None;
                super::notify(
                    cx,
                    note(
                        r.title.clone(),
                        format!("{file}\nAdded to .git/info/exclude. No shared file changed."),
                        Level::Success,
                        r.title.clone(),
                    ),
                );
                super::refresh(v, cx);
            } else if let Some(m) = file_menu_mut(v) {
                m.busy = false;
                m.ask = false;
                m.error = Some(format!("Could not ignore it: {}", [r.title.as_str(), r.body().as_str()].join(". ")));
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub struct Rejected {
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub detail: String,
    pub stale: bool,
    pub confirm: bool,
}

pub fn target_name(r: &Rejected) -> String {
    match (&r.remote, &r.branch) {
        (Some(remote), Some(b)) => format!("{remote}/{b}"),
        (None, Some(b)) => format!("The remote copy of {b}"),
        _ => "The remote branch".into(),
    }
}

pub fn rejected_why(r: &Rejected) -> String {
    let t = target_name(r);
    if r.stale {
        format!("{t} changed after your last fetch, so git stopped the push to keep those commits safe.")
    } else {
        format!("{t} has commits that your branch does not have, so git refused the push.")
    }
}

pub fn pull_then_push(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(Popup::Rejected(r)) = v.popup.take() else { return };
    super::remote_op(
        v,
        RemoteKind::Push,
        RemoteOpts {
            then_push: true,
            branch: r.branch,
            ..RemoteOpts::default()
        },
        cx,
    );
}

pub fn force_after_reject(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(Popup::Rejected(r)) = v.popup.take() else { return };
    if r.branch.is_none() {
        return;
    }
    super::remote_op(
        v,
        RemoteKind::Push,
        RemoteOpts {
            force_lease: true,
            branch: r.branch,
            ..RemoteOpts::default()
        },
        cx,
    );
}

pub struct IdentityBox {
    pub root: String,
    pub identity: Identity,
    pub profiles: Vec<Profile>,
    pub picked: Option<usize>,
    pub name: Entity<TextInput>,
    pub email: Entity<TextInput>,
    pub error: Option<String>,
    pub busy: bool,
    pub pending: Option<Action>,
}

pub fn suggested(profiles: &[Profile], id: &Identity) -> Option<usize> {
    let email = id.local_email.as_deref()?.to_lowercase();
    profiles.iter().position(|p| p.email.to_lowercase() == email)
}

pub fn identity_box(
    cx: &mut Context<ChangesView>,
    root: String,
    identity: Identity,
    profiles: &[Profile],
    pending: Option<Action>,
) -> IdentityBox {
    let name_now = identity.local_name.clone().unwrap_or_default();
    let email_now = identity.local_email.clone().unwrap_or_default();
    let name = cx.new(|cx| {
        let mut i = TextInput::new(cx, "Your Name");
        i.set_value(name_now, cx);
        i
    });
    let email = cx.new(|cx| {
        let mut i = TextInput::new(cx, "you@company.com");
        i.set_value(email_now, cx);
        i
    });
    IdentityBox {
        picked: suggested(profiles, &identity),
        root,
        identity,
        profiles: profiles.to_vec(),
        name,
        email,
        error: None,
        busy: false,
        pending,
    }
}

pub fn identity_use(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(Popup::Identity(b)) = v.popup.as_mut() else { return };
    if b.busy {
        return;
    }
    let profile = b.picked.and_then(|i| b.profiles.get(i)).cloned();
    let name = b.name.read(cx).value().to_string();
    let email = b.email.read(cx).value().to_string();
    b.busy = true;
    b.error = None;
    let root = b.root.clone();
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move {
            if let Some(profile) = profile {
                let env = super::super::accounts::kit::real().env;
                nixa_git::accounts::profiles::stamp_repo(&env, std::path::Path::new(&root), &profile, false)
                    .map(|_| ())
            } else {
                git::set_identity(&root, &name, &email)
            }
        }
    });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            let Some(Popup::Identity(b)) = v.popup.as_mut() else { return };
            if b.root != root {
                return;
            }
            match r {
                Ok(()) => {
                    let pending = b.pending.take();
                    super::identity_done(v, pending, true, cx);
                }
                Err(e) => {
                    b.busy = false;
                    b.error = Some(e);
                    cx.notify();
                }
            }
        })
        .ok();
    })
    .detach();
}

pub fn identity_cancel(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(Popup::Identity(b)) = v.popup.as_mut() else { return };
    let pending = b.pending.take();
    super::identity_done(v, pending, false, cx);
}

fn hint(t: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div().text_size(px(11.)).text_color(t.dim).child(text.into())
}

fn menu_row<T: Clone + 'static>(
    t: &Theme,
    prefix: &str,
    it: &Item<T>,
    hot: bool,
    busy: bool,
    on: impl Fn(&mut ChangesView, T, &mut Context<ChangesView>) + 'static,
    cx: &mut Context<ChangesView>,
) -> AnyElement {
    let id = format!("{prefix}-{}", it.id);
    let off = it.off.is_some() || busy;
    let ink = if off {
        t.dim
    } else if it.danger {
        t.red
    } else {
        t.foreground
    };
    let what = it.what.clone();
    let hover = t.hover;
    let sel = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || sel)
        .min_h(px(30.))
        .flex_none()
        .px(px(8.))
        .py(px(3.))
        .rounded(px(7.))
        .flex()
        .flex_col()
        .justify_center()
        .when(hot && !off, |el| el.bg(t.selected))
        .when(!off, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .when_some(it.off.clone(), |el, why| el.tooltip(tooltip(why)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(!off, |el| el.on_click(cx.listener(move |v, _, _, cx| on(v, what.clone(), cx))))
        .child(div().truncate().text_size(px(12.5)).text_color(ink).child(it.label.clone()))
        .when(!it.detail.is_empty(), |el| {
            el.child(div().truncate().text_size(px(11.)).text_color(t.dim).child(it.detail.clone()))
        })
        .into_any_element()
}

fn lines_view<T: Clone + 'static>(
    t: &Theme,
    prefix: &str,
    lines: &[Line<T>],
    hi: Option<usize>,
    busy: bool,
    on: impl Fn(&mut ChangesView, T, &mut Context<ChangesView>) + Clone + 'static,
    cx: &mut Context<ChangesView>,
) -> Vec<AnyElement> {
    let mut index = 0;
    lines
        .iter()
        .map(|l| match l {
            Line::Section(text) => div()
                .px(px(8.))
                .pt(px(8.))
                .pb(px(3.))
                .text_size(px(11.))
                .text_color(t.dim)
                .child(*text)
                .into_any_element(),
            Line::Gap => div().mx(px(7.)).my(px(4.)).h(px(1.)).bg(t.border).into_any_element(),
            Line::Item(it) => {
                let row = menu_row(t, prefix, it, hi == Some(index), busy, on.clone(), cx);
                index += 1;
                row
            }
        })
        .collect()
}

fn small(t: &Theme, id: &'static str, label: impl Into<SharedString>, kind: ButtonKind) -> gpui::Stateful<gpui::Div> {
    button(t, id, label, kind)
        .h(px(24.))
        .px(px(9.))
        .debug_selector(move || id.to_string())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

pub fn push_menu_view(m: &PushMenu, at: gpui::Point<gpui::Pixels>, cx: &mut Context<ChangesView>) -> AnyElement {
    let t = theme(cx).clone();
    let lines = push_lines(m);
    let counts = if m.upstream.is_some() {
        format!(" · ↑{} ↓{}", m.ahead, m.behind)
    } else {
        String::new()
    };
    let line = match &m.branch {
        Some(b) => format!("{b} → {}{counts}", m.upstream.as_deref().unwrap_or("not on the remote yet")),
        None => "Not on a branch".into(),
    };
    let rows = lines_view(&t, "push-menu", &lines, m.hi, m.busy.is_some(), choose_push, cx);
    let mut panel = popover(&t)
        .id("push-menu")
        .debug_selector(|| "push-menu".into())
        .w(px(320.))
        .max_h(px(560.))
        .flex()
        .flex_col()
        .on_mouse_down_out(cx.listener(|v, _, _, cx| {
            let busy = matches!(&v.popup, Some(Popup::Push { menu, .. }) if menu.busy.is_some());
            if !busy {
                v.popup = None;
                cx.notify();
            }
        }))
        .child(
            div()
                .px(px(8.))
                .pt(px(4.))
                .pb(px(4.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child("Push and pull"),
                )
                .child(div().truncate().text_size(px(11.5)).text_color(t.dim).child(line)),
        );
    if let Some(b) = push_blocked(m) {
        panel = panel.child(div().px(px(8.)).pb(px(4.)).child(hint(&t, b)));
    }
    if let Some(busy) = &m.busy {
        panel = panel.child(
            div()
                .mx(px(4.))
                .my(px(4.))
                .px(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(Icon::Spinner).size(px(13.)).text_color(t.dim))
                .child(div().flex_1().truncate().text_size(px(12.)).text_color(t.dim).child(busy.clone()))
                .when(m.cancel.as_ref().is_some_and(|c| !c.load(Ordering::Relaxed)), |el| {
                    el.child(
                        small(&t, "push-menu-stop", "Stop", ButtonKind::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| stop_menu_job(v, cx))),
                    )
                }),
        );
    }
    if m.ask_force {
        panel = panel.child(
            div()
                .mx(px(4.))
                .my(px(4.))
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.red)
                .bg(t.raised)
                .child(div().text_size(px(12.)).text_color(t.foreground).child(format!(
                    "This replaces {} with your {}. It stops if someone else pushed since your last fetch. Commits only on the remote are lost.",
                    m.upstream.as_deref().unwrap_or("the remote branch"),
                    m.branch.as_deref().unwrap_or("branch")
                )))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .justify_end()
                        .child(small(&t, "push-menu-force-no", "No", ButtonKind::Secondary).on_click(cx.listener(|v, _, _, cx| {
                            if let Some(m) = push_menu_mut(v) {
                                m.ask_force = false;
                            }
                            cx.notify();
                        })))
                        .child(
                            small(&t, "push-menu-force-yes", "Yes, force push", ButtonKind::Danger)
                                .on_click(cx.listener(|v, _, _, cx| force_yes(v, cx))),
                        ),
                ),
        );
    }
    if let (Some(c), None) = (&m.check, &m.busy)
        && let Some(pc) = &c.check {
            let color = if pc.behind == 0 {
                t.foreground
            } else if pc.clean {
                t.green
            } else {
                t.yellow
            };
            let can_pull = pc.behind > 0 && !(pc.mode == PullMode::FfOnly && pc.diverged);
            let mode = pc.mode;
            let mut card = div()
                .id("push-menu-check")
                .debug_selector(|| "push-menu-check".into())
                .mx(px(4.))
                .my(px(4.))
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.border)
                .bg(t.raised)
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .child(c.title.clone()),
                )
                .when(!c.hint.is_empty(), |el| el.child(hint(&t, c.hint.clone())));
            if pc.behind > 0 {
                card = card.child(hint(
                    &t,
                    format!(
                        "{} on {} · {} of yours · pull with {}",
                        git::plural(pc.behind as usize, "new commit", "new commits"),
                        pc.upstream,
                        git::plural(pc.ahead as usize, "commit", "commits"),
                        pc.mode.word()
                    ),
                ));
            }
            for (title, names, col) in [
                ("Files with conflicts", &pc.conflicts, t.red),
                ("Your changed files the pull also changes", &pc.local, t.yellow),
            ] {
                if names.is_empty() {
                    continue;
                }
                card = card.child(div().pt(px(4.)).text_size(px(11.)).text_color(t.dim).child(title));
                for n in names.iter().take(8) {
                    card = card.child(div().truncate().text_size(px(11.5)).text_color(col).child(n.clone()));
                }
                if names.len() > 8 {
                    card = card.child(hint(&t, format!("and {} more", names.len() - 8)));
                }
            }
            if let Some(n) = &m.note {
                card = card.child(hint(&t, n.clone()));
            }
            card = card.child(
                div()
                    .pt(px(6.))
                    .flex()
                    .gap(px(8.))
                    .justify_end()
                    .child(small(&t, "push-menu-check-close", "Close", ButtonKind::Secondary).on_click(cx.listener(|v, _, _, cx| {
                        v.popup = None;
                        cx.notify();
                    })))
                    .when(can_pull, |el| {
                        el.child(
                            small(&t, "push-menu-check-pull", format!("Pull with {}", mode.word()), ButtonKind::Primary).on_click(
                                cx.listener(move |v, _, _, cx| choose_push(v, PushItem::Pull(mode), cx)),
                            ),
                        )
                    }),
            );
            panel = panel.child(card);
        }
    if let (Some(e), None) = (&m.error, &m.busy) {
        panel = panel.child(
            div()
                .id("push-menu-error")
                .debug_selector(|| "push-menu-error".into())
                .mx(px(4.))
                .my(px(4.))
                .px(px(4.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(12.)).text_color(t.red).child(e.title.clone()))
                .when(!e.hint.is_empty(), |el| el.child(hint(&t, e.hint.clone())))
                .when(!e.detail.is_empty(), |el| el.child(hint(&t, git::masked_url(&e.detail))))
                .when(e.kind == git::Kind::Identity, |el| {
                    el.child(small(&t, "push-menu-identity", "Pick a profile", ButtonKind::Secondary).on_click(cx.listener(|v, _, _, cx| {
                        v.popup = None;
                        super::open_identity(v, None, cx);
                    })))
                })
                .when(matches!(e.kind, git::Kind::Auth | git::Kind::NotFound), |el| {
                    el.child(small(&t, "push-menu-accounts", "Open accounts", ButtonKind::Secondary).on_click(cx.listener(|v, _, _, cx| {
                        v.popup = None;
                        super::emit_window(v, "settings", cx);
                        cx.notify();
                    })))
                }),
        );
    }
    if let (Some(r), None) = (&m.result, &m.busy) {
        panel = panel.child(
            div()
                .mx(px(4.))
                .my(px(4.))
                .px(px(4.))
                .child(div().text_size(px(12.)).text_color(t.green).child(r.title.clone()))
                .when(!r.tags.is_empty(), |el| {
                    let more = if r.tags.len() > 8 {
                        format!(" and {} more", r.tags.len() - 8)
                    } else {
                        String::new()
                    };
                    el.child(hint(&t, format!("{}{more}", r.tags.iter().take(8).cloned().collect::<Vec<_>>().join(", "))))
                }),
        );
    }
    if let (Some(n), None, None) = (&m.note, &m.busy, &m.check) {
        panel = panel.child(div().px(px(8.)).pb(px(2.)).child(hint(&t, n.clone())));
    }
    panel = panel.child(
        div()
            .id("push-menu-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(rows),
    );
    float_at(at, Corner::TopRight, panel).into_any_element()
}

pub fn file_menu_view(m: &FileMenu, at: gpui::Point<gpui::Pixels>, cx: &mut Context<ChangesView>) -> AnyElement {
    let t = theme(cx).clone();
    let lines = file_lines(m);
    let rows = lines_view(&t, "file-menu", &lines, m.hi, m.busy, choose_file, cx);
    let word = if m.conflict {
        Some("conflict")
    } else {
        match m.status {
            'M' => Some("changed"),
            'A' => Some("new"),
            'D' => Some("deleted"),
            _ => None,
        }
    };
    let words: Vec<&str> = [word, m.staged.then_some("staged")].into_iter().flatten().collect();
    let folder = folder_of(&m.file).to_string();
    let bad = m.conflict || m.status == 'D';
    let mut panel = popover(&t)
        .id("file-menu")
        .debug_selector(|| "file-menu".into())
        .w(px(280.))
        .flex()
        .flex_col()
        .on_mouse_down_out(cx.listener(|v, _, _, cx| {
            if !matches!(&v.popup, Some(Popup::File { menu, .. }) if menu.busy) {
                v.popup = None;
                cx.notify();
            }
        }))
        .child(
            div()
                .px(px(8.))
                .pt(px(4.))
                .pb(px(6.))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.bright)
                                .child(base_name(&m.file).to_string()),
                        )
                        .when(!words.is_empty(), |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .text_color(if bad { t.red } else { t.dim })
                                    .child(words.join(", ")),
                            )
                        }),
                )
                .when(!folder.is_empty(), |el| el.child(hint(&t, folder.clone()))),
        )
        .child(div().mx(px(7.)).mb(px(4.)).h(px(1.)).bg(t.border))
        .children(rows);
    if m.ask {
        panel = panel.child(
            div()
                .id("file-menu-ask")
                .debug_selector(|| "file-menu-ask".into())
                .mx(px(4.))
                .mt(px(4.))
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.yellow)
                .bg(t.raised)
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.yellow)
                        .child(format!("Ignore {} on this computer?", base_name(&m.file))),
                )
                .child(hint(&t, "Git stops showing this file here. This adds one line to .git/info/exclude in this repo. No shared file changes, and .gitignore is not touched. Other people still see the file."))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .justify_end()
                        .child(disabled(small(&t, "file-menu-ask-no", "No", ButtonKind::Secondary), m.busy).on_click(cx.listener(|v, _, _, cx| {
                            if let Some(m) = file_menu_mut(v) {
                                m.ask = false;
                            }
                            cx.notify();
                        })))
                        .child(
                            disabled(
                                small(&t, "file-menu-ask-yes", if m.busy { "Ignoring…" } else { "Yes, do it" }, ButtonKind::Primary),
                                m.busy,
                            )
                            .on_click(cx.listener(|v, _, _, cx| ignore_yes(v, cx))),
                        ),
                ),
        );
    }
    if let Some(e) = &m.error {
        panel = panel.child(div().px(px(8.)).pb(px(4.)).text_size(px(11.5)).text_color(t.red).child(e.clone()));
    }
    float_at(at, Corner::TopLeft, panel).into_any_element()
}

pub fn repo_menu_view(v: &ChangesView, at: gpui::Point<gpui::Pixels>, hi: usize, cx: &mut Context<ChangesView>) -> AnyElement {
    let t = theme(cx).clone();
    let hover = t.hover;
    let rows: Vec<AnyElement> = v
        .repos
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let current = Some(&r.path) == v.project.as_ref();
            let path = r.path.clone();
            let sel = format!("repo-menu-{i}");
            div()
                .id(SharedString::from(sel.clone()))
                .debug_selector(move || sel)
                .h(px(30.))
                .flex_none()
                .px(px(8.))
                .gap(px(8.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .cursor_pointer()
                .when(i == hi, |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .tooltip(tooltip(r.path.clone()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.popup = None;
                    super::select_project(v, &path, cx);
                    cx.notify();
                }))
                .child(div().w(px(12.)).flex_none().text_size(px(11.)).text_color(t.green).child(if current { "✓" } else { "" }))
                .child(icon(Icon::Folder).size(px(13.)).flex_none().text_color(if current { t.cyan } else { t.blue }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.5))
                        .text_color(if current { t.cyan } else { t.green })
                        .child(r.name.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.5))
                        .text_color(if r.count > 0 { t.yellow } else { t.dim })
                        .child(if r.count > 0 { format!("{} changed", r.count) } else { "clean".into() }),
                )
                .into_any_element()
        })
        .collect();
    let panel = popover(&t)
        .id("repo-menu")
        .debug_selector(|| "repo-menu".into())
        .w(px(280.))
        .max_h(px(440.))
        .flex()
        .flex_col()
        .on_mouse_down_out(cx.listener(|v, _, _, cx| {
            v.popup = None;
            cx.notify();
        }))
        .child(div().px(px(8.)).pt(px(4.)).pb(px(6.)).text_size(px(11.5)).text_color(t.dim).child("Git repos in this folder"))
        .child(div().id("repo-menu-list").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().children(rows));
    float_at(at, Corner::TopLeft, panel).into_any_element()
}

fn option_card(
    t: &Theme,
    id: &'static str,
    mark: &'static str,
    heading: String,
    detail: String,
    danger: bool,
    off: bool,
) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let tone = if danger { t.red } else { t.accent };
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .w_full()
        .flex_none()
        .px(px(12.))
        .py(px(8.))
        .gap(px(10.))
        .flex()
        .items_start()
        .rounded(px(8.))
        .border_1()
        .border_color(if danger && !off { t.red } else { t.border })
        .bg(t.raised)
        .when(off, |el| el.opacity(0.55))
        .when(!off, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .child(div().w(px(14.)).flex_none().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(tone).child(mark))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if danger { t.red } else { t.foreground })
                        .child(heading),
                )
                .child(div().text_size(px(11.5)).text_color(t.dim).child(detail)),
        )
}

pub fn rejected_view(r: &Rejected, cx: &mut Context<ChangesView>) -> AnyElement {
    let t = theme(cx).clone();
    let branch = r.branch.clone();
    let choices: AnyElement = if r.confirm {
        div()
            .id("push-rejected-confirm")
            .debug_selector(|| "push-rejected-confirm".into())
            .px(px(12.))
            .py(px(10.))
            .gap(px(8.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(t.red)
            .bg(t.raised)
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.red)
                    .child(format!("Force push {} now?", branch.as_deref().unwrap_or("this branch"))),
            )
            .child(hint(&t, "Your branch will replace the remote one. Commits that only the remote has will be lost."))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        small(&t, "push-rejected-force-yes", "Yes, force push", ButtonKind::Danger)
                            .on_click(cx.listener(|v, _, _, cx| force_after_reject(v, cx))),
                    )
                    .child(small(&t, "push-rejected-force-no", "No, go back", ButtonKind::Secondary).on_click(cx.listener(|v, _, _, cx| {
                        if let Some(Popup::Rejected(r)) = v.popup.as_mut() {
                            r.confirm = false;
                        }
                        cx.notify();
                    }))),
            )
            .into_any_element()
    } else {
        let detail = match (&branch, r.stale) {
            (None, _) => "Push from the Changes panel to force push. This window does not know which branch to push.".to_string(),
            (Some(_), true) => "Replaces the remote branch with yours. Git stops it again if the remote changed since your last fetch.".to_string(),
            (Some(_), false) => "Replaces the remote branch with yours. Use it after you changed commits you already pushed. Commits only on the remote are lost.".to_string(),
        };
        let heading = match &branch {
            Some(b) => format!("Force push {b} (safe, with lease)"),
            None => "Force push (safe, with lease)".into(),
        };
        let off = branch.is_none();
        option_card(&t, "push-rejected-force", "↑", heading, detail, true, off)
            .when(!off, |el| {
                el.on_click(cx.listener(|v, _, _, cx| {
                    if let Some(Popup::Rejected(r)) = v.popup.as_mut() {
                        r.confirm = true;
                    }
                    cx.notify();
                }))
            })
            .into_any_element()
    };
    let body = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().text_size(px(12.5)).text_color(t.foreground).child(rejected_why(r)))
        .when(!r.detail.is_empty(), |el| el.child(hint(&t, r.detail.clone())))
        .child(
            option_card(
                &t,
                "push-rejected-pull",
                "↓",
                "Pull, then push".into(),
                "Brings in the remote commits, puts yours on top, then pushes. Nothing is lost.".into(),
                false,
                false,
            )
            .on_click(cx.listener(|v, _, _, cx| pull_then_push(v, cx))),
        )
        .child(choices);
    let footer = small(&t, "push-rejected-cancel", "Cancel", ButtonKind::Secondary)
        .on_click(cx.listener(|v, _, _, cx| {
            v.popup = None;
            cx.notify();
        }))
        .into_any_element();
    let this = cx.entity().downgrade();
    let this2 = this.clone();
    let card = dialog(&t, "push-rejected-view", "Push was rejected", 460., body, Some(footer), move |_, app| {
        this.update(app, |v, cx| {
            v.popup = None;
            cx.notify();
        })
        .ok();
    })
    .debug_selector(|| "push-rejected-view".into());
    overlay(&t, "push-rejected-scrim", card, move |_, app| {
        this2
            .update(app, |v, cx| {
                v.popup = None;
                cx.notify();
            })
            .ok();
    })
    .into_any_element()
}

fn field(t: &Theme, id: &'static str, label: &'static str, input: &Entity<TextInput>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().text_size(px(11.5)).text_color(t.dim).child(label))
        .child(
            div().id(id).debug_selector(move || id.to_string()).child(input.clone()),
        )
}

pub fn identity_view(b: &IdentityBox, cx: &mut Context<ChangesView>) -> AnyElement {
    let t = theme(cx).clone();
    let id = &b.identity;
    let now = match (&id.local_name, &id.local_email) {
        (Some(n), Some(e)) => div().text_size(px(12.5)).text_color(t.foreground).child(format!("Commits as {n} <{e}>")),
        _ => div()
            .text_size(px(12.5))
            .text_color(t.yellow)
            .child("None: this repo has no name and email of its own"),
    };
    let then = b.pending.as_ref().map(|a| match a {
        Action::Commit { .. } => "After you pick, the commit goes on. Cancel stops the commit.",
        Action::Remote(RemoteKind::Push, _) => "After you pick, the push goes on. Cancel stops the push.",
        Action::Remote(RemoteKind::Pull, _) => "After you pick, the pull goes on. Cancel stops the pull.",
        _ => "After you pick, git goes on with the next step. Cancel stops it.",
    });
    let hover = t.hover;
    let profiles: Vec<AnyElement> = b
        .profiles
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let on = b.picked == Some(i);
            let sel = format!("identity-pick-{i}");
            div()
                .id(SharedString::from(sel.clone()))
                .debug_selector(move || sel)
                .min_h(px(38.))
                .px(px(10.))
                .py(px(6.))
                .gap(px(10.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .border_1()
                .border_color(if on { t.accent } else { t.border })
                .when(on, |el| el.bg(t.selected))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |v, _, _, cx| {
                    if let Some(Popup::Identity(b)) = v.popup.as_mut() {
                        b.picked = if b.picked == Some(i) { None } else { Some(i) };
                        b.error = None;
                    }
                    cx.notify();
                }))
                .child(
                    icon(if on { Icon::CircleCheck } else { Icon::Circle })
                        .size(px(14.))
                        .text_color(if on { t.accent } else { t.dim }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(t.foreground).child(p.label.clone()))
                        .child(hint(&t, format!("{} <{}>", p.name, p.email))),
                )
                .into_any_element()
        })
        .collect();
    let mut body = div()
        .id("identity-body")
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(hint(&t, b.root.clone()))
        .child(hint(&t, "Every repo needs its own name and email. Your global git name and email are never used."))
        .when_some(then, |el, w| el.child(hint(&t, w)))
        .child(div().pt(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Now"))
        .child(now)
        .child(
            small(&t, "identity-manage-profiles", "Manage profiles…", ButtonKind::Secondary)
                .on_click(cx.listener(|v, _, _, cx| {
                    super::prefs::open_settings(v.root.clone(), "profiles", None, cx);
                })),
        );
    if let Some(r) = id.remotes.first() {
        if let Some(h) = &r.host {
            body = body.child(hint(&t, format!("Remote {}: {h}", r.name)));
        }
        if r.token_in_url {
            body = body.child(
                div()
                    .text_size(px(11.5))
                    .text_color(t.yellow)
                    .child(format!("The {} URL has a password or token in it. Remove it from the URL.", r.name)),
            );
        }
    }
    if !profiles.is_empty() {
        body = body
            .child(div().pt(px(6.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Pick a profile"))
            .children(profiles);
    }
    if b.picked.is_none() {
        body = body
            .child(div().pt(px(6.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Name and email for this repo"))
            .child(field(&t, "identity-name", "Name for commits", &b.name))
            .child(field(&t, "identity-email", "Email for commits", &b.email));
    }
    if let Some(e) = &b.error {
        body = body.child(div().text_size(px(11.5)).text_color(t.red).child(e.clone()));
    }
    let footer = div()
        .flex()
        .gap(px(8.))
        .child(
            disabled(small(&t, "identity-cancel", "Cancel", ButtonKind::Secondary), b.busy)
                .on_click(cx.listener(|v, _, _, cx| identity_cancel(v, cx))),
        )
        .child(
            disabled(
                small(&t, "identity-use", if b.busy { "Working…" } else { "Use for this repo" }, ButtonKind::Primary),
                b.busy,
            )
            .on_click(cx.listener(|v, _, _, cx| identity_use(v, cx))),
        )
        .into_any_element();
    let title = format!("Git identity for {}", base_name(&b.root));
    let this = cx.entity().downgrade();
    let this2 = this.clone();
    let card = dialog(&t, "identity-view", title, 460., body, Some(footer), move |_, app| {
        this.update(app, identity_cancel).ok();
    })
    .debug_selector(|| "identity-view".into());
    overlay(&t, "identity-scrim", card, move |_, app| {
        this2.update(app, identity_cancel).ok();
    })
    .into_any_element()
}
