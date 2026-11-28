pub mod diff;
pub mod diff_view;
pub mod diff_tabs;
pub mod git;
pub mod host;
pub mod menus;
pub mod patch;
mod prefs;
mod row_actions;
mod view;
pub mod words;

#[cfg(test)]
mod tests;

use diff_view::{DiffEvent, DiffView};
use git::{Done, FileEntry, Header, Identity, Kind, LeftStash, PullMode, Repo, RepoState, Verb};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Pixels, Point, Subscription, Window,
};
use nixa_ui::text_input::{InputEvent, TextInput};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};
use words::{Draft, HookFail, Level, Note, Plan, RemoteKind, RemoteOpts, note};

pub const WATCH_EVERY: Duration = Duration::from_secs(1);
pub const REFRESH_TICKS: u32 = 10;
pub const FETCH_TICKS: u32 = 30;
pub const IDENTITY_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitWindow {
    Branches { repo: String },
    Stash { repo: String },
    PullRequest { repo: String, branch: String },
    Settings { repo: Option<String>, tab: &'static str, reason: Option<String> },
    Conflict { repo: String, file: String },
    Blame { repo: String, file: String, rev: Option<String> },
    FileHistory { repo: String, path: String },
    Review { repo: String },
    AgentMenu { repo: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangesEvent {
    Notify(Note),
    OpenFile { repo: String, path: String },
    Open(GitWindow),
    Suggest { repo: String, branch: Option<String> },
    StopSuggest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Refresh,
    StageAll,
    UnstageAll,
    Commit,
    CommitAll,
    Fetch,
    Pull,
    Push,
    PushMenu,
    DiscardAll,
    ShowChanges,
    ShowGraph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Changes,
    Graph,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Discard { commands: Vec<Vec<String>>, label: String },
    DiscardAll,
    Side { path: String, ours: bool },
    Abort,
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Commit { all: bool, no_verify: bool },
    Remote(RemoteKind, RemoteOpts),
    Op(Verb),
}

impl Action {
    pub fn word(&self) -> &'static str {
        match self {
            Action::Commit { .. } => "Commit",
            Action::Remote(RemoteKind::Push, _) => "Push",
            Action::Remote(RemoteKind::Pull, _) => "Pull",
            Action::Remote(RemoteKind::Fetch, _) => "Fetch",
            Action::Op(_) => "The next git step",
        }
    }
}

pub struct Job {
    pub kind: RemoteKind,
    pub cancel: Arc<AtomicBool>,
    pub quiet: bool,
    pub root: String,
    pub stopping: bool,
}

pub enum Popup {
    Repos { at: Point<Pixels>, hi: usize },
    Push { at: Point<Pixels>, menu: Box<menus::PushMenu> },
    File { at: Point<Pixels>, menu: Box<menus::FileMenu> },
    Rejected(menus::Rejected),
    Identity(Box<menus::IdentityBox>),
    Agents { at: Point<Pixels>, view: Entity<super::ai::AgentMenu>, _events: Subscription },
}

pub struct ChangesView {
    pub cwd: Option<String>,
    pub folder: Option<String>,
    pub home: String,
    pub context: Option<String>,
    pub repos: Vec<Repo>,
    pub project: Option<String>,
    pub picks: HashMap<String, String>,
    pub root: Option<String>,
    pub files: Vec<FileEntry>,
    pub limited: bool,
    pub line_totals: (u64, u64),
    pub has_head: bool,
    pub has_remote: bool,
    pub header: Header,
    pub head_sha: Option<String>,
    pub state: Option<RepoState>,
    pub left: Option<LeftStash>,
    pub left_skip: HashSet<String>,
    pub error: Option<String>,
    pub note: Option<String>,
    pub loading: bool,
    pub again: bool,
    pub busy: bool,
    pub loaded: bool,
    pub seq: u64,
    pub closed: HashSet<String>,
    pub open: Option<String>,
    pub ident: Option<Identity>,
    pub ident_root: Option<String>,
    pub ident_at: Option<Instant>,
    pub identity_busy: bool,
    pub job: Option<Job>,
    pub queued: Option<(RemoteKind, RemoteOpts, String)>,
    pub ai_busy: bool,
    pub ai_label: String,
    pub message: Entity<TextInput>,
    pub description: Entity<TextInput>,
    pub show_desc: bool,
    pub amend: bool,
    pub amend_head: Option<String>,
    pub before_amend: Option<Draft>,
    pub amend_token: u64,
    pub hook_fail: Option<HookFail>,
    pub drafts: HashMap<String, Draft>,
    pub draft_root: Option<String>,
    pub ask: Option<Ask>,
    pub popup: Option<Popup>,
    pub diff: Option<Entity<DiffView>>,
    pub tab: Tab,
    pub graph: Option<Entity<super::history::graph::GraphView>>,
    pub profiles: Vec<menus::Profile>,
    pub pull_mode: PullMode,
    pub auto_fetch_minutes: u32,
    pub fetched_at: HashMap<String, Instant>,
    pub fetch_error_at: HashMap<String, Instant>,
    pub stamps: Vec<Option<SystemTime>>,
    pub ticks: u32,
    pub show_diff: bool,
    pub focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ChangesEvent> for ChangesView {}

impl Focusable for ChangesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

pub fn build(cx: &mut Context<ChangesView>) -> ChangesView {
    let message = cx.new(|cx| TextInput::new(cx, "Commit message"));
    let description = cx.new(|cx| TextInput::new(cx, "Description (optional)"));
    let on_message = cx.subscribe(&message, |v: &mut ChangesView, _, e: &InputEvent, cx| match e {
        InputEvent::Submit(_) => commit(v, false, cx),
        InputEvent::Changed(_) => cx.notify(),
        InputEvent::Cancel => {}
    });
    let on_desc = cx.subscribe(&description, |_: &mut ChangesView, _, e: &InputEvent, cx| {
        if let InputEvent::Changed(_) = e {
            cx.notify();
        }
    });
    ChangesView {
        cwd: None,
        folder: None,
        home: home(),
        context: None,
        repos: Vec::new(),
        project: None,
        picks: HashMap::new(),
        root: None,
        files: Vec::new(),
        limited: false,
        line_totals: (0, 0),
        has_head: false,
        has_remote: false,
        header: Header::default(),
        head_sha: None,
        state: None,
        left: None,
        left_skip: HashSet::new(),
        error: None,
        note: None,
        loading: false,
        again: false,
        busy: false,
        loaded: false,
        seq: 0,
        closed: HashSet::new(),
        open: None,
        ident: None,
        ident_root: None,
        ident_at: None,
        identity_busy: false,
        job: None,
        queued: None,
        ai_busy: false,
        ai_label: "the AI agent".into(),
        message,
        description,
        show_desc: false,
        amend: false,
        amend_head: None,
        before_amend: None,
        amend_token: 0,
        hook_fail: None,
        drafts: HashMap::new(),
        draft_root: None,
        ask: None,
        popup: None,
        diff: None,
        tab: Tab::Changes,
        graph: None,
        profiles: Vec::new(),
        pull_mode: PullMode::Rebase,
        auto_fetch_minutes: 0,
        fetched_at: HashMap::new(),
        fetch_error_at: HashMap::new(),
        stamps: Vec::new(),
        ticks: 0,
        show_diff: false,
        focus: cx.focus_handle(),
        _subs: vec![on_message, on_desc],
    }
}

pub fn new(cx: &mut App) -> Entity<ChangesView> {
    let view = cx.new(build);
    if !cfg!(test) {
        start_watch(&view, cx);
    }
    view
}

pub fn render(view: &Entity<ChangesView>) -> AnyElement {
    view.clone().into_any_element()
}

pub fn start_watch(view: &Entity<ChangesView>, cx: &mut App) {
    let weak = view.downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(WATCH_EVERY).await;
            let Some(view) = weak.upgrade() else { break };
            if view.update(cx, tick).is_err() {
                break;
            }
        }
    })
    .detach();
}

pub fn tick(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    v.ticks = v.ticks.wrapping_add(1);
    if v.ticks.is_multiple_of(FETCH_TICKS) {
        auto_fetch(v, cx);
    }
    let Some(root) = v.root.clone() else { return };
    if v.loading || v.busy {
        return;
    }
    if v.ticks.is_multiple_of(REFRESH_TICKS) {
        refresh(v, cx);
        return;
    }
    let seq = v.seq;
    let job = cx.background_executor().spawn(async move { git::stamp(&root) });
    cx.spawn(async move |this, cx| {
        let now = job.await;
        this.update(cx, |v, cx| {
            if v.seq == seq && !v.loading && v.stamps != now {
                refresh(v, cx);
            }
        })
        .ok();
    })
    .detach();
}

pub fn notify(cx: &mut Context<ChangesView>, n: Note) {
    cx.emit(ChangesEvent::Notify(n));
}

pub fn report(cx: &mut Context<ChangesView>, title: impl Into<String>, body: impl Into<String>, level: Level) {
    let title = title.into();
    notify(cx, note(title.clone(), body, level, title));
}

pub fn wait_note(v: &ChangesView, cx: &mut Context<ChangesView>) {
    let what = match v.job.as_ref().filter(|j| !j.quiet) {
        Some(j) => format!("Wait for the {} to finish", j.kind.word().to_lowercase()),
        None => "Wait for the current git work to finish".into(),
    };
    notify(cx, note(what, "Then try again.", Level::Info, "Wait a moment"));
}

pub fn local_job(v: &ChangesView) -> bool {
    v.job.as_ref().is_some_and(|j| j.kind != RemoteKind::Fetch)
}

pub fn set_place(view: &Entity<ChangesView>, cwd: Option<String>, folder: Option<String>, cx: &mut App) {
    view.update(cx, |v, cx| change_place(v, cwd, folder, cx));
}

pub fn scan_key(v: &ChangesView) -> Option<String> {
    let loc = git::location(v.cwd.as_deref(), v.folder.as_deref(), &v.home)?;
    Some(git::scan_root(&loc, v.folder.as_deref(), &v.home))
}

pub fn change_place(v: &mut ChangesView, cwd: Option<String>, folder: Option<String>, cx: &mut Context<ChangesView>) {
    v.cwd = cwd.filter(|c| !c.is_empty());
    v.folder = folder.filter(|f| !f.is_empty());
    let key = scan_key(v);
    if key == v.context && v.loaded {
        return;
    }
    if key != v.context {
        save_draft(v, cx);
        reset(v, cx);
        v.context = key;
        v.repos.clear();
        v.project = None;
    }
    refresh(v, cx);
}

fn reset(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    v.root = None;
    v.files.clear();
    v.limited = false;
    v.line_totals = (0, 0);
    v.profiles.clear();
    v.pull_mode = PullMode::default();
    v.auto_fetch_minutes = 0;
    v.has_head = false;
    v.has_remote = false;
    v.header = Header::default();
    v.head_sha = None;
    v.state = None;
    v.left = None;
    v.error = None;
    v.note = None;
    v.open = None;
    v.closed.clear();
    v.ident = None;
    v.ident_root = None;
    v.ident_at = None;
    v.amend = false;
    v.amend_head = None;
    v.before_amend = None;
    v.hook_fail = None;
    v.show_desc = false;
    v.ask = None;
    v.popup = None;
    v.diff = None;
    if let Some(graph) = &v.graph {
        graph.update(cx, |graph, cx| graph.set_repo(None, cx));
    }
    v.stamps.clear();
    v.loaded = false;
    v.loading = false;
    v.seq += 1;
    v.message.update(cx, |i, cx| i.set_value("", cx));
    v.description.update(cx, |i, cx| i.set_value("", cx));
}

fn value(input: &Entity<TextInput>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

pub fn message_text(v: &ChangesView, cx: &App) -> String {
    value(&v.message, cx)
}

pub fn description_text(v: &ChangesView, cx: &App) -> String {
    value(&v.description, cx)
}

fn set_text(input: &Entity<TextInput>, text: &str, cx: &mut Context<ChangesView>) {
    let text = text.to_string();
    input.update(cx, |i, cx| i.set_value(text, cx));
}

pub fn save_draft(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(root) = v.draft_root.take() else { return };
    let d = words::draft_of(
        &message_text(v, cx),
        &description_text(v, cx),
        v.show_desc,
        v.amend,
        v.before_amend.as_ref(),
    );
    match d {
        Some(d) => v.drafts.insert(root, d),
        None => v.drafts.remove(&root),
    };
}

fn restore_draft(v: &mut ChangesView, root: &str, cx: &mut Context<ChangesView>) {
    v.draft_root = Some(root.to_string());
    let Some(d) = v.drafts.get(root).cloned() else { return };
    if !message_text(v, cx).trim().is_empty() || !description_text(v, cx).trim().is_empty() || v.amend {
        return;
    }
    set_text(&v.message, &d.summary, cx);
    set_text(&v.description, &d.body, cx);
    v.show_desc = d.show_desc;
}

struct Loaded {
    repos: Vec<Repo>,
    project: Option<String>,
    snap: Result<git::Snapshot, String>,
    stamps: Vec<Option<SystemTime>>,
    prefs: prefs::PanelPrefs,
}

fn load_all(cwd: String, scan: String, pick: Option<String>) -> Loaded {
    let repos = git::discover(&scan);
    let chosen = git::choose_root(&cwd, &repos, pick.as_deref()).or_else(|| git::top_of(&cwd));
    let Some(root) = chosen else {
        return Loaded {
            repos,
            project: None,
            snap: Err("Not a git repository".into()),
            stamps: Vec::new(),
            prefs: prefs::PanelPrefs::default(),
        };
    };
    let snap = git::snapshot(&root);
    let stamps = git::stamp(&root);
    Loaded {
        repos,
        project: Some(root),
        snap,
        stamps,
        prefs: prefs::load(),
    }
}

pub fn refresh(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if v.loading || v.busy {
        v.again = true;
        return;
    }
    let Some(scan) = v.context.clone() else {
        v.show_diff = false;
        cx.notify();
        return;
    };
    let cwd = git::location(v.cwd.as_deref(), v.folder.as_deref(), &v.home).unwrap_or_else(|| scan.clone());
    let pick = v.picks.get(&scan).cloned();
    v.loading = true;
    v.again = false;
    v.seq += 1;
    let seq = v.seq;
    let job = cx.background_executor().spawn(async move { load_all(cwd, scan, pick) });
    cx.spawn(async move |this, cx| {
        let done = job.await;
        this.update(cx, |v, cx| loaded(v, seq, done, cx)).ok();
    })
    .detach();
    cx.notify();
}

fn loaded(v: &mut ChangesView, seq: u64, done: Loaded, cx: &mut Context<ChangesView>) {
    if v.seq != seq {
        return;
    }
    v.loading = false;
    v.loaded = true;
    v.repos = done.repos;
    v.project = done.project.clone();
    v.stamps = done.stamps;
    v.pull_mode = done.prefs.mode;
    v.auto_fetch_minutes = done.prefs.auto_fetch_minutes;
    v.profiles = done.prefs.profiles;
    let show_diff = std::mem::take(&mut v.show_diff);
    match done.snap {
        Err(e) => {
            v.root = None;
            v.files.clear();
            v.has_head = false;
            v.has_remote = false;
            v.header = Header::default();
            v.head_sha = None;
            v.line_totals = (0, 0);
            v.error = Some(e);
            if let Some(graph) = &v.graph {
                graph.update(cx, |graph, cx| graph.set_repo(None, cx));
            }
        }
        Ok(s) => {
            let root = s.root.clone();
            if v.draft_root.as_deref() != Some(root.as_str()) {
                restore_draft(v, &root, cx);
            }
            let before = (v.head_sha.clone(), v.header.branch.clone());
            v.root = Some(root.clone());
            v.files = s.files;
            v.limited = s.limited;
            v.line_totals = s.line_totals;
            v.has_head = s.has_head;
            v.has_remote = s.has_remote;
            v.header = s.header;
            v.head_sha = s.head_sha;
            v.error = None;
            if v.files.is_empty() && v.ask == Some(Ask::DiscardAll) {
                v.ask = None;
            }
            if s.state.as_ref().and_then(|st| st.op).is_none() && matches!(v.ask, Some(Ask::Abort | Ask::Skip)) {
                v.ask = None;
            }
            if let Some(Ask::Side { path, .. }) = &v.ask
                && !v.files.iter().any(|f| f.conflict && &f.path == path) {
                    v.ask = None;
                }
            v.state = s.state;
            v.left = s.left.filter(|l| !v.left_skip.contains(&format!("{root}|{}", l.sha)));
            let head_changed = before.0 != v.head_sha || before.1 != v.header.branch;
            if before.0.is_some() && head_changed {
                head_moved(v, cx);
            }
            if let Some(graph) = &v.graph {
                let repo = std::path::PathBuf::from(&root);
                graph.update(cx, |graph, cx| {
                    if graph.repo.as_ref() != Some(&repo) {
                        graph.set_repo(Some(repo), cx);
                    } else if head_changed {
                        graph.load(true, cx);
                    }
                });
            }
            let stale = v.ident_root.as_deref() != Some(root.as_str())
                || v.ident_at.is_none_or(|t| t.elapsed() > IDENTITY_EVERY);
            if stale {
                load_identity(v, cx);
            }
            if let Some(open) = &v.open
                && !v.files.iter().any(|f| &f.path == open) {
                    v.open = None;
                }
            if let Some(d) = v.diff.clone() {
                diff_view::refresh_entity(&d, cx);
            }
            if show_diff
                && let Some(path) = v.files.first().map(|f| f.path.clone())
            {
                set_tab(v, Tab::Changes, cx);
                open_diff(v, &path, cx);
            }
        }
    }
    if v.again && !v.busy {
        v.again = false;
        refresh(v, cx);
    }
    cx.notify();
}

fn head_moved(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    v.hook_fail = None;
    if !v.amend || v.amend_head.is_none() || v.amend_head == v.head_sha {
        return;
    }
    amend_off(v, cx);
    notify(
        cx,
        note("Amend turned off", "The last commit changed. Tick Amend again to load it.", Level::Info, "Amend turned off"),
    );
}

pub fn load_identity(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    v.ident_root = Some(root.clone());
    v.ident_at = Some(Instant::now());
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::read_identity(&root) }
    });
    cx.spawn(async move |this, cx| {
        let id = job.await;
        this.update(cx, |v, cx| {
            if v.root.as_deref() == Some(root.as_str()) {
                v.ident = Some(id);
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

pub fn toggle_folder(v: &mut ChangesView, folder: &str, cx: &mut Context<ChangesView>) {
    if !v.closed.remove(folder) {
        v.closed.insert(folder.to_string());
    }
    cx.notify();
}

pub fn act(v: &mut ChangesView, commands: Vec<Vec<String>>, ok: String, fail: &'static str, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if v.busy || local_job(v) || commands.is_empty() {
        return;
    }
    v.busy = true;
    v.note = None;
    cx.notify();
    let job = cx.background_executor().spawn(async move { git::run_all(&root, &commands) });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            match r {
                Ok(()) => report(cx, ok, "", Level::Success),
                Err(e) => report(cx, fail, e, Level::Error),
            }
            v.busy = false;
            refresh(v, cx);
        })
        .ok();
    })
    .detach();
}

pub fn stage(v: &mut ChangesView, paths: Vec<String>, label: &str, cx: &mut Context<ChangesView>) {
    act(v, git::stage_commands(&paths), format!("Staged {label}"), "Stage failed", cx);
}

pub fn unstage(v: &mut ChangesView, paths: Vec<String>, label: &str, cx: &mut Context<ChangesView>) {
    let has_head = v.has_head;
    act(v, git::unstage_commands(&paths, has_head), format!("Unstaged {label}"), "Unstage failed", cx);
}

pub fn ask_discard(v: &mut ChangesView, paths: Vec<String>, label: &str, cx: &mut Context<ChangesView>) {
    let commands = git::discard_commands(&v.files, &paths, v.has_head);
    if commands.is_empty() || v.root.is_none() {
        return;
    }
    v.ask = Some(Ask::Discard {
        commands,
        label: label.to_string(),
    });
    cx.notify();
}

pub fn answer(v: &mut ChangesView, yes: bool, cx: &mut Context<ChangesView>) {
    if yes && (v.busy || v.identity_busy || local_job(v)) {
        return;
    }
    let Some(ask) = v.ask.take() else { return };
    cx.notify();
    if !yes {
        return;
    }
    match ask {
        Ask::Discard { commands, label } => act(v, commands, format!("Discarded {label}"), "Discard failed", cx),
        Ask::DiscardAll => discard_all(v, cx),
        Ask::Side { path, ours } => resolve_side(v, path, ours, cx),
        Ask::Abort => with_identity(v, Action::Op(Verb::Abort), cx),
        Ask::Skip => with_identity(v, Action::Op(Verb::Skip), cx),
    }
}

pub fn ask_discard_all(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if v.root.is_none() || v.files.is_empty() || v.busy {
        return;
    }
    v.ask = Some(Ask::DiscardAll);
    cx.notify();
}

pub fn discard_all(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if v.busy {
        return;
    }
    if local_job(v) {
        wait_note(v, cx);
        return;
    }
    let commands = git::discard_all_commands(&v.files, 100, v.has_head);
    if commands.is_empty() {
        return;
    }
    let count = v.files.len();
    v.busy = true;
    cx.notify();
    let job = cx.background_executor().spawn(async move {
        let mut error = String::new();
        for c in &commands {
            if let Err(e) = git::run_all(&root, std::slice::from_ref(c))
                && error.is_empty() {
                    error = e;
                }
        }
        (git::changed_paths(&root), error)
    });
    cx.spawn(async move |this, cx| {
        let (left, error) = job.await;
        this.update(cx, |v, cx| {
            if let Some(n) = words::leftover_note(&left, &error) {
                notify(cx, n);
            } else if !error.is_empty() {
                report(cx, "Discard failed", error, Level::Error);
            } else {
                notify(
                    cx,
                    note("Discarded all changes", git::plural(count, "file", "files"), Level::Success, "Discarded all changes"),
                );
            }
            v.busy = false;
            refresh(v, cx);
        })
        .ok();
    })
    .detach();
}

pub fn conflict_count(v: &ChangesView) -> usize {
    let from_state = v.state.as_ref().map(|s| s.conflicts.len()).unwrap_or(0);
    from_state.max(v.files.iter().filter(|f| f.conflict).count())
}

pub fn staged_count(v: &ChangesView) -> usize {
    v.files.iter().filter(|f| f.staged).count()
}

pub fn commit(v: &mut ChangesView, all: bool, cx: &mut Context<ChangesView>) {
    let message = message_text(v, cx);
    if (message.trim().is_empty() && !v.amend) || v.root.is_none() || v.loading || v.identity_busy {
        return;
    }
    if v.busy || local_job(v) {
        wait_note(v, cx);
        return;
    }
    if !all && staged_count(v) == 0 && !v.amend {
        return;
    }
    if all && v.files.is_empty() && !v.amend {
        return;
    }
    if all && conflict_count(v) > 0 {
        notify(
            cx,
            note(
                "Fix the conflicted files first",
                "Commit all would save the conflict markers. Pick a side or mark each file resolved, then commit.",
                Level::Warning,
                "Fix the conflicts first",
            ),
        );
        return;
    }
    with_identity(v, Action::Commit { all, no_verify: false }, cx);
}

pub fn commit_without_hooks(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(fail) = v.hook_fail.clone() else { return };
    with_identity(v, Action::Commit { all: fail.all, no_verify: true }, cx);
}

pub fn with_identity(v: &mut ChangesView, action: Action, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if v.identity_busy {
        return;
    }
    v.identity_busy = true;
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::read_identity(&root) }
    });
    cx.spawn(async move |this, cx| {
        let id = job.await;
        this.update(cx, |v, cx| {
            v.identity_busy = false;
            if v.root.as_deref() != Some(root.as_str()) {
                cx.notify();
                return;
            }
            let ready = id.ready();
            v.ident = Some(id.clone());
            v.ident_at = Some(Instant::now());
            if ready {
                run_action(v, action, cx);
            } else {
                open_identity(v, Some(action), cx);
                notify(cx, note("Pick a profile for this repo", "", Level::Info, "Pick a profile for this repo"));
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn open_identity(v: &mut ChangesView, pending: Option<Action>, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    let profile_root = root.clone();
    super::accounts::kit::job(cx, prefs::profiles, move |v, profiles, _| {
        if v.root.as_deref() != Some(profile_root.as_str()) {
            return;
        }
        if let Some(Popup::Identity(popup)) = v.popup.as_mut()
            && !popup.busy
        {
            let picked = popup.picked.and_then(|index| popup.profiles.get(index)).map(|p| &p.id);
            popup.picked = picked.and_then(|id| profiles.iter().position(|p| &p.id == id));
            popup.profiles = profiles.clone();
        }
        v.profiles = profiles;
    });
    let ident = v.ident.clone().unwrap_or_default();
    let boxed = menus::identity_box(cx, root, ident, &v.profiles, pending);
    v.popup = Some(Popup::Identity(Box::new(boxed)));
    cx.notify();
}

pub fn identity_done(v: &mut ChangesView, pending: Option<Action>, ok: bool, cx: &mut Context<ChangesView>) {
    v.popup = None;
    load_identity(v, cx);
    match (pending, ok) {
        (Some(a), true) => with_identity(v, a, cx),
        (Some(a), false) => notify(cx, words::identity_stop(a.word())),
        _ => {}
    }
    cx.notify();
}

pub fn run_action(v: &mut ChangesView, action: Action, cx: &mut Context<ChangesView>) {
    match action {
        Action::Commit { all, no_verify } => do_commit(v, all, no_verify, cx),
        Action::Op(verb) => run_op(v, verb, cx),
        Action::Remote(kind, opts) => run_remote(v, kind, opts, cx),
    }
}

pub fn do_commit(v: &mut ChangesView, all: bool, no_verify: bool, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if v.busy || local_job(v) {
        wait_note(v, cx);
        return;
    }
    let summary = message_text(v, cx).trim().to_string();
    let body = description_text(v, cx).trim_end().to_string();
    let amend = v.amend;
    if summary.is_empty() && !amend {
        return;
    }
    let opts = git::CommitOpts {
        summary: summary.clone(),
        body,
        amend,
        expect_head: if amend { v.amend_head.clone() } else { None },
        no_verify,
        all,
    };
    v.busy = true;
    v.note = None;
    v.hook_fail = None;
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::commit(&root, &opts) }
    });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            let same = v.root.as_deref() == Some(root.as_str());
            let out = words::commit_outcome(&r, amend, &summary, all);
            if let Some(n) = out.note {
                notify(cx, n);
            }
            if out.clear && same {
                set_text(&v.message, "", cx);
                set_text(&v.description, "", cx);
                v.show_desc = false;
                v.amend = false;
                v.before_amend = None;
                v.amend_head = None;
            }
            if same {
                v.hook_fail = out.hook_fail;
            }
            if out.amend_off && same && v.amend {
                amend_off(v, cx);
            }
            v.busy = false;
            if out.ask_identity && same {
                open_identity(v, Some(Action::Commit { all, no_verify }), cx);
            }
            if out.reload {
                load_identity(v, cx);
            }
            refresh(v, cx);
        })
        .ok();
    })
    .detach();
}

pub fn amend_off(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    v.amend = false;
    v.amend_token += 1;
    v.amend_head = None;
    if let Some(back) = v.before_amend.take() {
        set_text(&v.message, &back.summary, cx);
        set_text(&v.description, &back.body, cx);
        v.show_desc = back.show_desc;
    }
    cx.notify();
}

pub fn toggle_amend(v: &mut ChangesView, on: bool, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if !on {
        amend_off(v, cx);
        return;
    }
    if !v.has_head {
        return;
    }
    v.before_amend = Some(Draft {
        summary: message_text(v, cx),
        body: description_text(v, cx),
        show_desc: v.show_desc,
    });
    v.amend = true;
    v.amend_head = None;
    v.amend_token += 1;
    let token = v.amend_token;
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::head_message(&root) }
    });
    cx.spawn(async move |this, cx| {
        let head = job.await;
        this.update(cx, |v, cx| {
            let Some((sha, summary, body)) = head else { return };
            if !v.amend || token != v.amend_token || v.root.as_deref() != Some(root.as_str()) {
                return;
            }
            v.amend_head = Some(sha);
            set_text(&v.message, &summary, cx);
            set_text(&v.description, &body, cx);
            if !body.trim().is_empty() {
                v.show_desc = true;
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn suggest(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if v.ai_busy {
        cx.emit(ChangesEvent::StopSuggest);
        return;
    }
    if v.busy || staged_count(v) == 0 {
        return;
    }
    cx.emit(ChangesEvent::Suggest {
        repo: root,
        branch: v.header.branch.clone(),
    });
}

pub fn set_ai(view: &Entity<ChangesView>, busy: bool, label: Option<String>, cx: &mut App) {
    view.update(cx, |v, cx| {
        v.ai_busy = busy;
        if let Some(l) = label {
            v.ai_label = l;
        }
        cx.notify();
    });
}

pub fn fill_message(view: &Entity<ChangesView>, summary: &str, body: &str, cx: &mut App) {
    let body = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    view.update(cx, |v, cx| {
        set_text(&v.message, summary, cx);
        set_text(&v.description, &body, cx);
        if !body.is_empty() {
            v.show_desc = true;
        }
        save_draft(v, cx);
        cx.notify();
    });
}

pub fn refresh_view(view: &Entity<ChangesView>, cx: &mut App) {
    view.update(cx, refresh);
}

pub fn show_first_diff(view: &Entity<ChangesView>, cx: &mut App) {
    view.update(cx, |v, cx| {
        if v.loaded && !v.loading {
            if let Some(path) = v.files.first().map(|f| f.path.clone()) {
                set_tab(v, Tab::Changes, cx);
                open_diff(v, &path, cx);
            }
        } else if v.context.is_some() {
            v.show_diff = true;
            if !v.loading {
                refresh(v, cx);
            }
        }
    });
}

pub fn set_tab(v: &mut ChangesView, tab: Tab, cx: &mut Context<ChangesView>) {
    if v.tab != tab {
        v.tab = tab;
        if tab == Tab::Graph && v.graph.is_none() {
            v.graph = Some(super::history::new_graph(v.root.as_ref().map(std::path::PathBuf::from), cx));
        }
        cx.notify();
    }
}

pub fn remote_op(v: &mut ChangesView, kind: RemoteKind, opts: RemoteOpts, cx: &mut Context<ChangesView>) {
    if v.root.is_none() {
        return;
    }
    match words::remote_plan(kind, opts, v.has_remote, v.header.branch.as_deref(), v.header.detached) {
        Plan::Refuse(n) => notify(cx, n),
        Plan::Run(RemoteKind::Fetch, o) => run_remote(v, RemoteKind::Fetch, o, cx),
        Plan::Run(k, o) => with_identity(v, Action::Remote(k, o), cx),
    }
}

pub fn fetch(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if v.has_remote {
        run_remote(v, RemoteKind::Fetch, RemoteOpts::default(), cx);
    }
}

pub fn can_sync(v: &ChangesView) -> bool {
    v.root.is_some()
        && v.has_remote
        && v.has_head
        && !v.header.detached
        && !v.header.unborn
        && v.header.branch.is_some()
}

pub fn pull(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if can_sync(v) {
        with_identity(v, Action::Remote(RemoteKind::Pull, RemoteOpts::default()), cx);
    }
}

pub fn push(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if can_sync(v) {
        with_identity(v, Action::Remote(RemoteKind::Push, RemoteOpts::default()), cx);
    }
}

fn run_remote_work(root: &str, kind: RemoteKind, o: &RemoteOpts, mode: PullMode, cancel: &AtomicBool) -> Done {
    match kind {
        RemoteKind::Fetch => git::fetch(
            root,
            &git::FetchOpts {
                all: o.all,
                prune: o.prune.unwrap_or(true),
                tags: o.tags,
                remote: o.remote.clone(),
            },
            cancel,
        ),
        RemoteKind::Pull => git::pull(root, o.mode.unwrap_or(mode), cancel),
        RemoteKind::Push => git::push(
            root,
            &git::PushOpts {
                force_lease: o.force_lease,
                branch: o.branch.clone(),
                remote: o.remote.clone(),
                tags: o.tags,
            },
            cancel,
        ),
    }
}

pub fn run_remote(v: &mut ChangesView, kind: RemoteKind, opts: RemoteOpts, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if let Some(job) = &v.job {
        if opts.quiet {
            return;
        }
        if job.quiet {
            v.queued = Some((kind, opts, root));
            stop_remote(v, cx);
            return;
        }
        wait_note(v, cx);
        return;
    }
    let (kind, opts) = if kind == RemoteKind::Push && opts.then_push {
        (
            RemoteKind::Pull,
            RemoteOpts {
                then_push: true,
                ..opts
            },
        )
    } else {
        (kind, opts)
    };
    let cancel = Arc::new(AtomicBool::new(false));
    v.job = Some(Job {
        kind,
        cancel: cancel.clone(),
        quiet: opts.quiet,
        root: root.clone(),
        stopping: false,
    });
    if kind == RemoteKind::Fetch {
        v.fetched_at.insert(root.clone(), Instant::now());
    }
    cx.notify();
    let mode = v.pull_mode;
    let work = cx.background_executor().spawn({
        let root = root.clone();
        let opts = opts.clone();
        async move { run_remote_work(&root, kind, &opts, mode, &cancel) }
    });
    cx.spawn(async move |this, cx| {
        let r = work.await;
        this.update(cx, |v, cx| after_remote(v, kind, &opts, &root, r, cx)).ok();
    })
    .detach();
}

fn after_remote(v: &mut ChangesView, kind: RemoteKind, opts: &RemoteOpts, root: &str, r: Done, cx: &mut Context<ChangesView>) {
    if v.job.as_ref().is_some_and(|j| j.root == root && j.kind == kind) {
        v.job = None;
    }
    let same = v.root.as_deref() == Some(root);
    let out = words::remote_outcome(kind, &r, opts.quiet, opts.then_push, same, v.header.branch.as_deref());
    if out.reload {
        load_identity(v, cx);
    }
    if let Some(n) = out.quiet_error {
        let due = words::fetch_error_due(v.fetch_error_at.get(root).copied());
        if due {
            v.fetch_error_at.insert(root.to_string(), Instant::now());
            notify(cx, n);
        }
    }
    if let Some(n) = out.note {
        notify(cx, n);
    }
    if let Some(rej) = out.rejected.filter(|_| same) {
        v.popup = Some(Popup::Rejected(menus::Rejected {
            remote: rej.remote,
            branch: rej.branch,
            detail: rej.detail,
            stale: rej.stale,
            confirm: false,
        }));
    }
    if let Some(reason) = out.open_accounts {
        prefs::open_settings(Some(root.to_string()), "accounts", Some(reason), cx);
    }
    if out.next_push {
        run_remote(
            v,
            RemoteKind::Push,
            RemoteOpts {
                branch: opts.branch.clone(),
                ..RemoteOpts::default()
            },
            cx,
        );
    }
    if let Some((k, o, qroot)) = v.queued.take()
        && v.job.is_none() && v.root.as_deref() == Some(qroot.as_str()) {
            run_remote(v, k, o, cx);
        }
    if v.loading || v.busy {
        v.again = true;
    } else {
        refresh(v, cx);
    }
    cx.notify();
}

pub fn stop_remote(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(job) = v.job.as_mut() else { return };
    if job.stopping {
        return;
    }
    job.cancel.store(true, Ordering::Relaxed);
    job.stopping = true;
    cx.notify();
}

pub fn auto_fetch(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if !v.has_remote || v.job.is_some() || v.busy || v.loading || v.identity_busy {
        return;
    }
    if !words::fetch_due(v.auto_fetch_minutes, v.fetched_at.get(&root).copied()) {
        return;
    }
    run_remote(
        v,
        RemoteKind::Fetch,
        RemoteOpts {
            quiet: true,
            ..RemoteOpts::default()
        },
        cx,
    );
}

pub fn run_op(v: &mut ChangesView, verb: Verb, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    let Some(op) = v.state.as_ref().and_then(|s| s.op) else { return };
    if v.busy || local_job(v) {
        wait_note(v, cx);
        return;
    }
    v.busy = true;
    v.ask = None;
    cx.notify();
    let job = cx.background_executor().spawn({
        let root = root.clone();
        async move { git::sequence(&root, op, verb) }
    });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            if r.ok {
                report(cx, r.title.clone(), r.hint.clone(), Level::Success);
            } else {
                report(cx, r.title.clone(), r.body(), Level::Error);
            }
            if v.root.as_deref() == Some(root.as_str())
                && let Some(st) = r.state.clone() {
                    v.state = Some(st);
                }
            v.busy = false;
            refresh(v, cx);
        })
        .ok();
    })
    .detach();
}

pub fn ask_side(v: &mut ChangesView, path: &str, ours: bool, cx: &mut Context<ChangesView>) {
    if v.root.is_none() || v.busy {
        return;
    }
    v.ask = Some(Ask::Side {
        path: path.to_string(),
        ours,
    });
    cx.notify();
}

fn simple(v: &mut ChangesView, work: impl FnOnce(&str) -> Done + Send + 'static, spoken: &'static str, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if v.busy {
        return;
    }
    v.busy = true;
    v.note = None;
    cx.notify();
    let job = cx.background_executor().spawn(async move { work(&root) });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            if r.ok {
                notify(cx, note(r.title.clone(), r.hint.clone(), Level::Success, spoken));
            } else {
                let level = if matches!(r.kind, Kind::Markers | Kind::Conflict) {
                    Level::Warning
                } else {
                    Level::Error
                };
                notify(cx, note(r.title.clone(), r.body(), level, r.title.clone()));
            }
            v.busy = false;
            refresh(v, cx);
        })
        .ok();
    })
    .detach();
}

pub fn resolve_side(v: &mut ChangesView, path: String, ours: bool, cx: &mut Context<ChangesView>) {
    let spoken = if ours { "Kept current" } else { "Took incoming" };
    simple(v, move |root| git::resolve_file(root, &path, ours), spoken, cx);
}

pub fn mark_resolved(v: &mut ChangesView, path: String, cx: &mut Context<ChangesView>) {
    simple(v, move |root| git::mark_resolved(root, &path, true), "Marked resolved", cx);
}

pub fn bring_back(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(left) = v.left.clone() else { return };
    if v.busy || local_job(v) {
        wait_note(v, cx);
        return;
    }
    v.left = None;
    simple(v, move |root| git::bring_back(root, &left), "Changes brought back", cx);
}

pub fn skip_left(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if let (Some(root), Some(left)) = (v.root.clone(), v.left.take()) {
        v.left_skip.insert(format!("{root}|{}", left.sha));
    }
    cx.notify();
}

pub fn select_project(v: &mut ChangesView, path: &str, cx: &mut Context<ChangesView>) {
    if v.project.as_deref() == Some(path) {
        return;
    }
    if v.busy || local_job(v) {
        wait_note(v, cx);
        return;
    }
    if !v.repos.iter().any(|r| r.path == path) {
        return;
    }
    save_draft(v, cx);
    if let Some(key) = v.context.clone() {
        v.picks.insert(key, path.to_string());
    }
    let repos = std::mem::take(&mut v.repos);
    reset(v, cx);
    v.repos = repos;
    v.project = Some(path.to_string());
    refresh(v, cx);
}

pub fn open_diff(v: &mut ChangesView, path: &str, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    if path.contains('\0') {
        return;
    }
    v.open = Some(path.to_string());
    let same = v
        .diff
        .as_ref()
        .is_some_and(|d| d.read(cx).repo == root && d.read(cx).path == path);
    if !same {
        let view = diff_view::new(cx, root, path.to_string());
        let sub = cx.subscribe(&view, |v: &mut ChangesView, _, e: &DiffEvent, cx| on_diff(v, e, cx));
        v._subs.push(sub);
        v.diff = Some(view);
    }
    cx.notify();
}

fn on_diff(v: &mut ChangesView, e: &DiffEvent, cx: &mut Context<ChangesView>) {
    match e {
        DiffEvent::Changed => refresh(v, cx),
        DiffEvent::Notify(n) => notify(cx, n.clone()),
        DiffEvent::OpenFile { repo, path } => cx.emit(ChangesEvent::OpenFile {
            repo: repo.clone(),
            path: path.clone(),
        }),
        DiffEvent::Close => {
            v.diff = None;
            cx.notify();
        }
    }
}

pub fn emit_window(v: &ChangesView, which: &str, cx: &mut Context<ChangesView>) {
    let root = v.root.clone();
    let window = match (which, root) {
        ("branches", Some(repo)) => GitWindow::Branches { repo },
        ("stash", Some(repo)) => GitWindow::Stash { repo },
        ("pr", Some(repo)) => match v.header.branch.clone() {
            Some(branch) => GitWindow::PullRequest { repo, branch },
            None => return,
        },
        ("review", Some(repo)) if staged_count(v) > 0 => GitWindow::Review { repo },
        ("agents", repo) => GitWindow::AgentMenu { repo },
        ("settings", repo) => GitWindow::Settings {
            repo,
            tab: "accounts",
            reason: None,
        },
        _ => return,
    };
    match window {
        GitWindow::Settings { repo, tab, reason } => prefs::open_settings(repo, tab, reason, cx),
        other => cx.emit(ChangesEvent::Open(other)),
    }
}

pub fn close_popup(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    if let Some(Popup::Identity(b)) = v.popup.take()
        && let Some(a) = b.pending {
            notify(cx, words::identity_stop(a.word()));
        }
    cx.notify();
}

pub fn commands() -> Vec<(&'static str, Command)> {
    vec![
        ("Git: refresh the changes", Command::Refresh),
        ("Git: stage all changes", Command::StageAll),
        ("Git: unstage all changes", Command::UnstageAll),
        ("Git: commit the staged changes", Command::Commit),
        ("Git: stage everything and commit", Command::CommitAll),
        ("Git: fetch", Command::Fetch),
        ("Git: pull", Command::Pull),
        ("Git: push", Command::Push),
        ("Git: push and pull menu", Command::PushMenu),
        ("Git: discard all changes", Command::DiscardAll),
        ("Git: show the changes", Command::ShowChanges),
        ("Git: show the graph", Command::ShowGraph),
    ]
}

pub fn run(view: &Entity<ChangesView>, command: Command, window: &mut Window, cx: &mut App) {
    view.update(cx, |v, cx| match command {
        Command::Refresh => {
            load_identity(v, cx);
            refresh(v, cx);
        }
        Command::StageAll => stage_all(v, cx),
        Command::UnstageAll => unstage_all(v, cx),
        Command::Commit => commit(v, false, cx),
        Command::CommitAll => commit(v, true, cx),
        Command::Fetch => fetch(v, cx),
        Command::Pull => pull(v, cx),
        Command::Push => push(v, cx),
        Command::PushMenu => open_push_menu(v, Point::default(), cx),
        Command::DiscardAll => ask_discard_all(v, cx),
        Command::ShowChanges => set_tab(v, Tab::Changes, cx),
        Command::ShowGraph => set_tab(v, Tab::Graph, cx),
    });
    let focus = view.read(cx).focus.clone();
    window.focus(&focus);
}

pub fn stage_all(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let paths = stageable_all(v);
    if !paths.is_empty() {
        stage(v, paths, "all changes", cx);
    }
}

pub fn unstage_all(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let paths: Vec<String> = v.files.iter().filter(|f| f.staged).map(|f| f.path.clone()).collect();
    if !paths.is_empty() {
        unstage(v, paths, "all changes", cx);
    }
}

pub fn stageable_all(v: &ChangesView) -> Vec<String> {
    v.files
        .iter()
        .filter(|f| !f.conflict && f.y != ' ')
        .map(|f| f.path.clone())
        .collect()
}

pub fn open_push_menu(v: &mut ChangesView, at: Point<Pixels>, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    let menu = menus::push_menu(
        root.clone(),
        v.header.branch.clone(),
        v.header.upstream.clone(),
        v.header.ahead,
        v.header.behind,
        v.has_remote,
        v.pull_mode,
    );
    v.popup = Some(Popup::Push { at, menu: Box::new(menu) });
    menus::load_remotes(v, root, cx);
    cx.notify();
}

pub fn open_file_menu(v: &mut ChangesView, path: &str, at: Point<Pixels>, cx: &mut Context<ChangesView>) {
    let Some(root) = v.root.clone() else { return };
    let Some(f) = v.files.iter().find(|f| f.path == path).cloned() else { return };
    v.popup = Some(Popup::File {
        at,
        menu: Box::new(menus::file_menu(root.clone(), &f)),
    });
    menus::load_facts(v, root, f.path, cx);
    cx.notify();
}

pub fn open_repo_menu(v: &mut ChangesView, at: Point<Pixels>, cx: &mut Context<ChangesView>) {
    if v.repos.len() < 2 {
        return;
    }
    let hi = v
        .repos
        .iter()
        .position(|r| Some(&r.path) == v.project.as_ref())
        .unwrap_or(0);
    v.popup = Some(Popup::Repos { at, hi });
    cx.notify();
}
