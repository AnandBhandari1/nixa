use super::kit::{self, Ctx};
use crate::screens::settings::parts::memory::text_area::TextArea;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use nixa_git::accounts::profiles::{ensure_identity, load_profiles};
use nixa_git::accounts::Env;
use nixa_git::forge::{
    BranchState, ForgeError, Found, GhOpts, NewPr, Opened, PrAccount, Remote, account_reason, base_branches,
    branch_state, create_pr, default_branch, default_target, find_pr, forge_targets, pick_base, pick_remote,
    pr_account_for, pr_head, pull_branch, push_branch, repo_remotes, same_repo, title_from_branch,
};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const WIDTH: f32 = 620.0;
pub const HEIGHT: f32 = 640.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Failed,
    Form,
    Exists,
    Done,
}

pub type PrAi = fn(
    &nixa_git::ai::AiSettings,
    &std::path::Path,
    Option<&str>,
    Option<&str>,
    &nixa_git::ai::Opts,
) -> Result<nixa_git::ai::PrText, nixa_git::ai::AiError>;

#[derive(Clone, Copy)]
pub struct PrDeps {
    pub ai: PrAi,
}

pub fn real_deps() -> PrDeps {
    PrDeps { ai: nixa_git::ai::pr_description }
}

pub struct PrView {
    pub ctx: Ctx,
    pub deps: PrDeps,
    pub focus: FocusHandle,
    pub root: Option<PathBuf>,
    pub phase: Phase,
    pub load_id: u32,
    pub info: BranchState,
    pub remotes: Vec<Remote>,
    pub account: Option<PrAccount>,
    pub targets: Vec<Remote>,
    pub target_name: Option<String>,
    pub bases: Vec<String>,
    pub base: String,
    pub base_touched: bool,
    pub default_base: Option<String>,
    pub show_bases: bool,
    pub checking: bool,
    pub existing: Option<Found>,
    pub old_pr: Option<Found>,
    pub created: Option<Opened>,
    pub copied: bool,
    pub draft: bool,
    pub auto_title: String,
    pub busy: bool,
    pub step: String,
    pub cancel: Option<Arc<AtomicBool>>,
    pub error: Option<ForgeError>,
    retry_pull: bool,
    pub note: Option<ForgeError>,
    pub warnings: Vec<String>,
    pub ai_cancel: Option<nixa_git::ai::Cancel>,
    pub ai_error: Option<String>,
    pub title: Entity<TextInput>,
    pub body: Entity<TextArea>,
}

pub fn fail(kind: &str, title: &str, hint: &str) -> ForgeError {
    ForgeError { kind: kind.into(), title: title.into(), hint: hint.into(), detail: String::new(), url: None }
}

pub fn stopped() -> ForgeError {
    fail("cancelled", "Stopped", "If it stopped while creating, check GitHub before you try again.")
}

pub fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn new(ctx: Ctx, deps: PrDeps, root: Option<PathBuf>, cx: &mut App) -> Entity<PrView> {
    cx.new(|cx| build(ctx, deps, root, cx))
}

pub fn build(ctx: Ctx, deps: PrDeps, root: Option<PathBuf>, cx: &mut Context<PrView>) -> PrView {
    let title = kit::input("Title", "", false, cx);
    let body = cx.new(|cx| TextArea::new(cx, "Description (optional)"));
    cx.subscribe(&title, |v, _, e: &InputEvent, cx| match e {
        InputEvent::Submit(_) => submit(v, cx),
        _ => cx.notify(),
    })
    .detach();
    let mut view = PrView {
        ctx,
        deps,
        focus: cx.focus_handle(),
        root,
        phase: Phase::Loading,
        load_id: 0,
        info: BranchState::default(),
        remotes: Vec::new(),
        account: None,
        targets: Vec::new(),
        target_name: None,
        bases: Vec::new(),
        base: String::new(),
        base_touched: false,
        default_base: None,
        show_bases: false,
        checking: false,
        existing: None,
        old_pr: None,
        created: None,
        copied: false,
        draft: false,
        auto_title: String::new(),
        busy: false,
        step: String::new(),
        cancel: None,
        error: None,
        retry_pull: false,
        note: None,
        warnings: Vec::new(),
        ai_cancel: None,
        ai_error: None,
        title,
        body,
    };
    load(&mut view, cx);
    view
}

pub fn target(v: &PrView) -> Option<Remote> {
    v.targets
        .iter()
        .find(|t| Some(&t.name) == v.target_name.as_ref())
        .cloned()
        .or_else(|| v.account.as_ref().map(|a| a.remote.clone()))
}

pub fn acting(v: &PrView) -> Option<PrAccount> {
    let a = v.account.clone()?;
    let remote = target(v).unwrap_or_else(|| a.remote.clone());
    Some(PrAccount { remote, ..a })
}

pub fn cross_repo(v: &PrView) -> bool {
    match (v.account.as_ref(), target(v)) {
        (Some(a), Some(t)) => !same_repo(&a.remote, &t),
        _ => false,
    }
}

pub fn head(v: &PrView) -> String {
    pr_head(v.account.as_ref().map(|a| &a.remote), target(v).as_ref(), v.info.branch.as_deref().unwrap_or(""))
}

pub fn same_base(v: &PrView) -> bool {
    !v.base.is_empty() && !cross_repo(v) && Some(v.base.as_str()) == v.info.branch.as_deref()
}

pub fn ready(v: &PrView, cx: &App) -> bool {
    v.phase == Phase::Form && !v.busy && !kit::value(&v.title, cx).trim().is_empty() && !v.base.is_empty() && !same_base(v)
}

pub struct Start {
    pub info: BranchState,
    pub remotes: Vec<Remote>,
    pub account: PrAccount,
}

pub fn read_start(env: &Env, root: Option<&std::path::Path>) -> Result<Start, ForgeError> {
    let no_repo = || fail("noRepo", "Open this from a repo.", "Open the Git panel in a repository to create a pull request.");
    let root = root.ok_or_else(no_repo)?;
    let work_tree = nixa_git::accounts::helper::git_ok(env, root, &["rev-parse", "--is-inside-work-tree"]);
    if !work_tree.is_ok_and(|reply| reply.trim() == "true") {
        return Err(no_repo());
    }
    if env.gh.is_none() {
        return Err(fail("noGh", "GitHub CLI (gh) is not installed", "Install it with: brew install gh. Then press Try again."));
    }
    let info = branch_state(env, root);
    let remotes = repo_remotes(env, root);
    if info.branch.is_none() {
        return Err(fail("other", "You are not on a branch", "Check out a branch first, then open this again."));
    }
    let remote = pick_remote(&remotes, info.push_remote.as_deref())
        .cloned()
        .ok_or_else(|| fail("other", "This repo has no GitHub remote", "Add a remote in Settings, Git accounts, This repo."))?;
    let account = pr_account_for(env, root, &remotes, Some(&remote.name)).ok_or_else(|| {
        fail("auth", &format!("No account for {}", remote.host), "Add or import an account in Settings, Git accounts. Then press Try again.")
    })?;
    Ok(Start { info, remotes, account })
}

pub fn load(v: &mut PrView, cx: &mut Context<PrView>) {
    v.load_id += 1;
    let id = v.load_id;
    v.phase = Phase::Loading;
    v.error = None;
    v.note = None;
    v.existing = None;
    v.old_pr = None;
    v.created = None;
    v.copied = false;
    let env = v.ctx.env.clone();
    let root = v.root.clone();
    kit::job(cx, move || read_start(&env, root.as_deref()), move |v, got, cx| {
        if id != v.load_id {
            return;
        }
        match got {
            Ok(s) => {
                v.targets = forge_targets(&s.remotes, &s.account.remote);
                v.target_name = default_target(&v.targets, &s.account.remote.name);
                v.auto_title = title_from_branch(s.info.branch.as_deref().unwrap_or(""));
                if kit::value(&v.title, cx).trim().is_empty() {
                    let t = v.auto_title.clone();
                    v.title.update(cx, |i, cx| i.set_value(t, cx));
                }
                v.info = s.info;
                v.remotes = s.remotes;
                v.account = Some(s.account);
                v.phase = Phase::Form;
                load_target(v, cx);
            }
            Err(e) => {
                v.phase = Phase::Failed;
                v.error = Some(e);
            }
        }
    });
    cx.notify();
}

pub struct TargetInfo {
    pub bases: Vec<String>,
    pub main: Result<Option<String>, ForgeError>,
    pub pr: Result<Option<Found>, ForgeError>,
}

pub fn load_target(v: &mut PrView, cx: &mut Context<PrView>) {
    let (Some(root), Some(acct), Some(t)) = (v.root.clone(), acting(v), target(v)) else { return };
    let id = v.load_id;
    v.checking = true;
    v.note = None;
    v.existing = None;
    v.old_pr = None;
    v.default_base = None;
    let exclude: Vec<String> = if cross_repo(v) { Vec::new() } else { v.info.branch.clone().into_iter().collect() };
    let head = head(v);
    let env = v.ctx.env.clone();
    kit::job(
        cx,
        move || TargetInfo {
            bases: base_branches(&env, &root, &t.name, &exclude),
            main: default_branch(&env, &root, &acct),
            pr: find_pr(&env, &root, &acct, &head),
        },
        move |v, got, _| {
            if id != v.load_id {
                return;
            }
            v.bases = got.bases;
            let main = match got.main {
                Ok(m) => m,
                Err(e) => {
                    v.note.get_or_insert(e);
                    None
                }
            };
            let pr = match got.pr {
                Ok(p) => p,
                Err(e) => {
                    v.note.get_or_insert(e);
                    None
                }
            };
            v.default_base = main.clone();
            if !v.base_touched {
                v.base = pick_base(&v.bases, main.as_deref());
            }
            v.checking = false;
            match pr {
                Some(p) if p.state == "OPEN" => {
                    v.existing = Some(p);
                    v.phase = Phase::Exists;
                }
                other => v.old_pr = other,
            }
        },
    );
    cx.notify();
}

pub fn pick_target(v: &mut PrView, name: &str, cx: &mut Context<PrView>) {
    if v.busy || v.target_name.as_deref() == Some(name) {
        return;
    }
    v.target_name = Some(name.to_string());
    v.base_touched = false;
    v.show_bases = false;
    v.load_id += 1;
    load_target(v, cx);
}

pub enum Sent {
    Created(Opened),
    Exists(Found),
    Failed(ForgeError),
    Stopped,
}

#[derive(Clone)]
pub struct Job {
    pub root: PathBuf,
    pub account: PrAccount,
    pub target: Option<Remote>,
    pub branch: String,
    pub pr: NewPr,
    pub cancel: Arc<AtomicBool>,
}

pub enum Checked {
    Ready { account: PrAccount, push: Option<String>, warnings: Vec<String> },
    Stop(Sent, Vec<String>),
}

pub fn check_first(env: &Env, job: &Job) -> Checked {
    let profiles = load_profiles(env);
    let warnings = match ensure_identity(env, &job.root, &profiles) {
        Ok(e) if e.status == "ask" => {
            let why = fail("identity", "This repo has no profile yet", "Choose the name and email for this repo to continue.");
            return Checked::Stop(Sent::Failed(why), e.warnings);
        }
        Ok(e) => e.warnings,
        Err(e) => return Checked::Stop(Sent::Failed(ForgeError { detail: e, ..fail("other", "Could not check the profile for this repo", "") }), Vec::new()),
    };
    if job.cancel.load(Ordering::SeqCst) {
        return Checked::Stop(Sent::Stopped, warnings);
    }
    let remotes = repo_remotes(env, &job.root);
    let account = pr_account_for(env, &job.root, &remotes, Some(&job.account.remote.name)).unwrap_or_else(|| job.account.clone());
    let info = branch_state(env, &job.root);
    if info.branch.as_deref() != Some(job.branch.as_str()) {
        return Checked::Stop(Sent::Failed(fail("other", "The branch changed", "Close this and open it again.")), warnings);
    }
    let push = match (&info.upstream, info.ahead) {
        (None, _) => Some("Publishing this branch…".to_string()),
        (Some(_), n) if n > 0 => Some(format!("Pushing {}…", plural(n, "commit", "commits"))),
        _ => None,
    };
    Checked::Ready { account, push, warnings }
}

pub fn push_first(env: &Env, job: &Job, account: &PrAccount) -> Option<Sent> {
    match push_branch(env, &job.root, account, &job.branch, Some(job.cancel.clone())) {
        Ok(()) if job.cancel.load(Ordering::SeqCst) => Some(Sent::Stopped),
        Ok(()) => None,
        Err(_) if job.cancel.load(Ordering::SeqCst) => Some(Sent::Stopped),
        Err(e) if e.kind == "rejected" => {
            Some(Sent::Failed(ForgeError { hint: "Pull the remote changes, then try pushing again.".into(), ..e }))
        }
        Err(e) => Some(Sent::Failed(e)),
    }
}

pub fn acting_for(job: &Job, account: &PrAccount) -> PrAccount {
    PrAccount { remote: job.target.clone().unwrap_or_else(|| account.remote.clone()), ..account.clone() }
}

pub fn look_first(env: &Env, job: &Job, acting: &PrAccount) -> Option<Sent> {
    if job.cancel.load(Ordering::SeqCst) {
        return Some(Sent::Stopped);
    }
    match find_pr(env, &job.root, acting, &job.pr.head) {
        Ok(Some(found)) if found.state == "OPEN" => Some(Sent::Exists(found)),
        _ => None,
    }
}

pub fn make(env: &Env, job: &Job, acting: &PrAccount) -> Sent {
    if job.cancel.load(Ordering::SeqCst) {
        return Sent::Stopped;
    }
    let opts = GhOpts { cancel: Some(job.cancel.clone()), ..GhOpts::default() };
    match create_pr(env, &job.root, acting, &job.pr, &opts) {
        Ok(o) => Sent::Created(o),
        Err(e) if e.kind == "exists" && e.url.is_some() => {
            let url = e.url.clone().unwrap_or_default();
            let number = url.rsplit('/').next().and_then(|n| n.parse().ok()).unwrap_or(0);
            Sent::Exists(Found { number, url, state: "OPEN".into(), title: String::new() })
        }
        Err(_) if job.cancel.load(Ordering::SeqCst) => Sent::Stopped,
        Err(e) => Sent::Failed(e),
    }
}

fn say(v: &mut PrView, text: &str) {
    if v.cancel.as_ref().is_some_and(|c| !c.load(Ordering::SeqCst)) {
        v.step = text.to_string();
    }
}

pub fn submit(v: &mut PrView, cx: &mut Context<PrView>) {
    submit_with_pull(v, false, cx);
}

fn submit_with_pull(v: &mut PrView, pull: bool, cx: &mut Context<PrView>) {
    if !ready(v, cx) {
        return;
    }
    let (Some(root), Some(account), Some(branch)) = (v.root.clone(), v.account.clone(), v.info.branch.clone()) else { return };
    let cancel = Arc::new(AtomicBool::new(false));
    let job = Job {
        root,
        account,
        target: target(v),
        branch,
        pr: NewPr {
            title: kit::value(&v.title, cx).trim().to_string(),
            body: v.body.read(cx).value().to_string(),
            base: v.base.clone(),
            head: head(v),
            draft: v.draft,
        },
        cancel: cancel.clone(),
    };
    v.busy = true;
    v.cancel = Some(cancel);
    v.retry_pull = pull;
    v.error = None;
    v.show_bases = false;
    v.warnings.clear();
    v.step = "Checking the profile for this repo…".into();
    let env = v.ctx.env.clone();
    let j = job.clone();
    kit::job(cx, move || check_first(&env, &j), move |v, checked, cx| match checked {
        Checked::Stop(s, w) => finish(v, (s, w), cx),
        Checked::Ready { account, push, warnings } => {
            v.warnings = warnings;
            match push.or_else(|| pull.then(|| "Pushing this branch…".into())) {
                Some(words) => {
                    say(v, if pull { "Pulling, then pushing this branch…" } else { &words });
                    let settings = v.ctx.db.clone();
                    let env = v.ctx.env.clone();
                    let (j, a) = (job.clone(), account.clone());
                    kit::job(cx, move || {
                        if pull {
                            let mode = nixa_git::accounts::settings::prefs_from(&kit::all_settings(&settings)).pull_mode;
                            if let Err(error) = pull_branch(&env, &j.root, &a, &j.branch, &mode, Some(j.cancel.clone())) {
                                if j.cancel.load(Ordering::SeqCst) {
                                    return Some(Sent::Stopped);
                                }
                                return Some(Sent::Failed(ForgeError { title: format!("Could not pull: {}", error.title), ..error }));
                            }
                            if j.cancel.load(Ordering::SeqCst) {
                                return Some(Sent::Stopped);
                            }
                        }
                        push_first(&env, &j, &a)
                    }, move |v, pushed, cx| match pushed {
                        Some(s) => {
                            let w = std::mem::take(&mut v.warnings);
                            finish(v, (s, w), cx)
                        }
                        None => look_then_make(v, job, account, cx),
                    });
                }
                None => look_then_make(v, job, account, cx),
            }
        }
    });
    cx.notify();
}

fn look_then_make(v: &mut PrView, job: Job, account: PrAccount, cx: &mut Context<PrView>) {
    say(v, "Looking for an open pull request…");
    let env = v.ctx.env.clone();
    let acting = acting_for(&job, &account);
    let (j, a) = (job.clone(), acting.clone());
    kit::job(cx, move || look_first(&env, &j, &a), move |v, found, cx| match found {
        Some(s) => {
            let w = std::mem::take(&mut v.warnings);
            finish(v, (s, w), cx)
        }
        None => {
            say(v, "Creating the pull request…");
            let env = v.ctx.env.clone();
            kit::job(cx, move || make(&env, &job, &acting), |v, sent, cx| {
                let w = std::mem::take(&mut v.warnings);
                finish(v, (sent, w), cx)
            });
        }
    });
}

fn finish(v: &mut PrView, (sent, warnings): (Sent, Vec<String>), cx: &mut Context<PrView>) {
    v.busy = false;
    v.cancel = None;
    v.step.clear();
    v.warnings = warnings;
    match sent {
        Sent::Created(o) => {
            v.created = Some(o);
            v.phase = Phase::Done;
        }
        Sent::Exists(f) => {
            v.existing = Some(f);
            v.phase = Phase::Exists;
        }
        Sent::Failed(e) => v.error = Some(e),
        Sent::Stopped => v.error = Some(stopped()),
    }
    cx.notify();
}

pub fn stop(v: &mut PrView, cx: &mut Context<PrView>) {
    if let Some(c) = &v.cancel {
        c.store(true, Ordering::SeqCst);
        v.step = "Stopping…".into();
        cx.notify();
    }
}

pub fn write_ai(v: &mut PrView, cx: &mut Context<PrView>) {
    if let Some(c) = v.ai_cancel.take() {
        c.store(true, Ordering::SeqCst);
        cx.notify();
        return;
    }
    let Some(root) = v.root.clone() else { return };
    let cancel = nixa_git::ai::cancel();
    v.ai_cancel = Some(cancel.clone());
    v.ai_error = None;
    let settings = nixa_git::ai::AiSettings::from_value(kit::get_setting(&v.ctx.db, nixa_git::ai::SETTINGS_KEY).as_ref());
    let base = match target(v) {
        Some(t) if v.bases.contains(&v.base) => format!("{}/{}", t.name, v.base),
        _ => v.base.clone(),
    };
    let branch = v.info.branch.clone();
    let deps = v.deps;
    let opts = nixa_git::ai::Opts { agent: None, model: None, timeout_sec: None, cancel: cancel.clone() };
    kit::job(
        cx,
        move || (deps.ai)(&settings, &root, Some(&base).filter(|b| !b.is_empty()).map(|b| b.as_str()), branch.as_deref(), &opts),
        move |v, got, cx| {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            v.ai_cancel = None;
            match got {
                Ok(text) => {
                    let current = kit::value(&v.title, cx).trim().to_string();
                    if !text.title.is_empty() && (current.is_empty() || current == v.auto_title) {
                        let t = text.title.clone();
                        v.title.update(cx, |i, cx| i.set_value(t, cx));
                    }
                    let body = text.body.clone();
                    v.body.update(cx, |a, cx| a.set_value(body, cx));
                }
                Err(e) => v.ai_error = Some(e.message),
            }
        },
    );
    cx.notify();
}

fn open_accounts(v: &PrView, cx: &mut App) {
    let repo = v.root.as_ref().map(|root| root.to_string_lossy().into_owned());
    let reason = v.error.as_ref().or(v.note.as_ref()).map(|error| error.title.clone());
    crate::screens::settings::parts::git_accounts::open_for(repo, "accounts", reason, cx);
}

fn pick_profile(v: &PrView, pull: bool, window: &Window, cx: &mut Context<PrView>) {
    let Some(repo) = v.root.clone() else { return };
    let view = cx.weak_entity();
    let handle = window.window_handle();
    super::identity_win::open(repo, move |cx| {
        handle.update(cx, |_, window, cx| {
            view.update(cx, |v, cx| {
                if v.error.as_ref().is_some_and(|error| error.kind == "identity") {
                    window.activate_window();
                    submit_with_pull(v, pull, cx);
                }
            }).ok();
        }).ok();
    }, cx);
}

fn close(v: &mut PrView, window: &mut Window) {
    if let Some(c) = &v.cancel {
        c.store(true, Ordering::SeqCst);
    }
    if let Some(c) = &v.ai_cancel {
        c.store(true, Ordering::SeqCst);
    }
    window.remove_window();
}

fn error_block(t: &Theme, e: &ForgeError, prefix: &str, cx: &mut Context<PrView>) -> AnyElement {
    let action = match e.kind.as_str() {
        "auth" => Some(kit::act(t, format!("{prefix}-accounts"), "Open accounts", ButtonKind::Secondary, false)
            .on_click(cx.listener(|v, _, _, cx| open_accounts(v, cx)))),
        "identity" => Some(kit::act(t, format!("{prefix}-accounts"), "Pick a profile", ButtonKind::Secondary, false)
            .on_click(cx.listener(|v, _, window, cx| pick_profile(v, v.retry_pull, window, cx)))),
        "rejected" => Some(kit::act(t, format!("{prefix}-pull-push"), "Pull, then push…", ButtonKind::Secondary, false)
            .on_click(cx.listener(|v, _, _, cx| submit_with_pull(v, true, cx)))),
        _ => None,
    };
    div()
        .pt(px(8.))
        .flex()
        .flex_col()
        .child(kit::error_text(t, e.title.clone()))
        .when(!e.hint.is_empty(), |el| el.child(kit::hint(t, e.hint.clone())))
        .when(!e.detail.is_empty(), |el| el.child(kit::hint(t, e.detail.clone()).pt(px(4.))))
        .when_some(action, |el, action| el.child(kit::row().pt(px(8.)).child(action)))
        .into_any_element()
}

fn info_row(t: &Theme, name: &str, children: Vec<AnyElement>) -> gpui::Div {
    kit::row().pt(px(10.)).min_w_0().child(div().w(px(84.)).flex_none().text_size(px(11.5)).text_color(t.muted).child(name.to_string())).children(children)
}

fn footer(items: Vec<AnyElement>, t: &Theme) -> gpui::Div {
    kit::row().flex_none().justify_end().pt(px(10.)).mt(px(8.)).border_t_1().border_color(t.border).children(items)
}

fn base_list(v: &PrView, t: &Theme, cx: &mut Context<PrView>) -> AnyElement {
    let mut names = v.bases.clone();
    for extra in [v.default_base.clone(), Some(v.base.clone())].into_iter().flatten() {
        if !extra.is_empty() && !names.contains(&extra) && !(Some(extra.as_str()) == v.info.branch.as_deref() && !cross_repo(v)) {
            names.insert(0, extra);
        }
    }
    if names.is_empty() {
        return kit::hint(t, "No branches to pick yet. Fetch first so the list fills in.").into_any_element();
    }
    let hover = t.hover;
    let mut list = div().id("pr-base-list").mt(px(6.)).max_h(px(150.)).overflow_y_scroll().p(px(4.)).rounded(px(8.)).border_1().border_color(t.border).bg(t.chrome).flex().flex_col();
    for (i, name) in names.iter().enumerate() {
        let n = name.clone();
        list = list.child(
            div()
                .id(("pr-base", i))
                .debug_selector(move || format!("pr-base-{i}"))
                .h(px(26.))
                .px(px(8.))
                .flex()
                .flex_row()
                .gap(px(8.))
                .flex_none()
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(*name == v.base, |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.base = n.clone();
                    v.base_touched = true;
                    v.show_bases = false;
                    cx.notify();
                }))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(t.foreground).child(name.clone()))
                .when(v.default_base.as_deref() == Some(name.as_str()), |el| el.child(kit::small(t, "default"))),
        );
    }
    list.into_any_element()
}

fn notes(v: &PrView, t: &Theme) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let host = v.account.as_ref().map(|a| a.host.clone()).unwrap_or_else(|| "GitHub".into());
    if v.info.upstream.is_none() {
        out.push(kit::warn_text(t, format!("This branch is not on {host} yet. Create will publish it first.")));
    } else if v.info.ahead > 0 {
        let them = if v.info.ahead == 1 { "it" } else { "them" };
        out.push(kit::warn_text(t, format!("{} not pushed yet. Create will push {them} first.", plural(v.info.ahead, "commit is", "commits are"))));
    }
    if v.info.behind > 0 {
        out.push(kit::hint(t, format!("The remote branch has {}. You may need to pull first.", plural(v.info.behind, "newer commit", "newer commits"))));
    }
    if let Some(tg) = target(v)
        && !v.checking
        && v.bases.is_empty()
    {
        out.push(kit::hint(t, format!("No branches found on {}. Fetch first so the list fills in.", tg.name)));
    }
    if same_base(v) {
        out.push(kit::warn_text(t, "The base branch is this same branch. Pick another one."));
    }
    if let Some(old) = &v.old_pr {
        out.push(kit::hint(t, format!("Pull request #{} for this branch is {}.", old.number, old.state.to_lowercase())));
    }
    for w in &v.warnings {
        out.push(kit::warn_text(t, w.clone()));
    }
    out.into_iter().map(|d| d.pt(px(8.)).into_any_element()).collect()
}

fn form(v: &PrView, t: &Theme, cx: &mut Context<PrView>) -> (AnyElement, AnyElement) {
    let Some(acct) = v.account.clone() else { return (div().into_any_element(), div().into_any_element()) };
    let tg = target(v);
    let mut into: Vec<AnyElement> = Vec::new();
    if v.targets.len() > 1 {
        for (i, item) in v.targets.iter().enumerate() {
            let name = item.name.clone();
            into.push(
                kit::pill(t, format!("pr-target-{i}"), format!("{}: {}/{}", item.name, item.owner, item.repo), Some(&item.name) == v.target_name.as_ref(), v.busy)
                    .on_click(cx.listener(move |v, _, _, cx| pick_target(v, &name, cx)))
                    .into_any_element(),
            );
        }
    }
    let base_text = if v.base.is_empty() { "Pick a base branch  ▾".to_string() } else { format!("{}  ▾", v.base) };
    let mut branches = vec![
        div().flex_none().px(px(8.)).h(px(22.)).flex().items_center().rounded(px(11.)).bg(t.selected).text_size(px(12.)).text_color(t.blue).child(head(v)).into_any_element(),
        div().flex_none().text_size(px(12.)).text_color(t.dim).child("→").into_any_element(),
        kit::pill(t, "pr-base", base_text, v.show_bases, v.busy)
            .on_click(cx.listener(|v, _, _, cx| {
                v.show_bases = !v.show_bases;
                cx.notify();
            }))
            .into_any_element(),
    ];
    if v.checking {
        branches.push(kit::busy_line(t, "pr-checking", "").w_auto().into_any_element());
    }
    let ai_busy = v.ai_cancel.is_some();
    let draft = v.draft;
    let content = kit::scroll_body("pr-create-body")
        .child(info_row(t, "Account", vec![
            div().flex_none().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(t.foreground).child(format!("{} @ {}", acct.username, acct.host)).into_any_element(),
            kit::small(t, account_reason(&acct)).flex_1().into_any_element(),
        ]))
        .child(if into.is_empty() {
            info_row(t, "Repository", vec![kit::small(t, tg.map(|x| format!("{}/{}", x.owner, x.repo)).unwrap_or_default()).into_any_element()])
        } else {
            info_row(t, "Into", into).flex_wrap()
        })
        .child(info_row(t, "Branches", branches))
        .when(v.show_bases, |el| el.child(base_list(v, t, cx)))
        .child(kit::label(t, "Title"))
        .child(kit::field(&v.title, "pr-title"))
        .child(
            kit::row()
                .child(kit::label(t, "Description"))
                .child(div().flex_1())
                .when(ai_busy, |el| el.child(kit::busy_line(t, "pr-ai-spin", "Writing…").w_auto()))
                .child(kit::act(t, "pr-ai", if ai_busy { "Stop AI" } else { "Write with AI" }, ButtonKind::Secondary, v.busy).on_click(cx.listener(|v, _, _, cx| write_ai(v, cx)))),
        )
        .child(div().w_full().h(px(168.)).flex().flex_col().debug_selector(|| "pr-body".into()).child(v.body.clone()))
        .when_some(v.ai_error.clone(), |el, e| el.child(kit::error_text(t, format!("AI: {e}"))))
        .child(div().pt(px(8.)).child(kit::check(t, "pr-draft", draft, "Create as draft").on_click(cx.listener(move |v, _, _, cx| {
            if !v.busy {
                v.draft = !draft;
                cx.notify();
            }
        }))))
        .children(notes(v, t))
        .when_some(v.note.clone(), |el, n| el.child(error_block(t, &n, "pr-note", cx)))
        .when_some(v.error.clone(), |el, e| el.child(error_block(t, &e, "pr-error", cx)))
        .into_any_element();
    let bottom = if v.busy {
        footer(
            vec![
                kit::row().flex_1().min_w_0().child(kit::busy_line(t, "pr-busy", v.step.clone())).into_any_element(),
                kit::act(t, "pr-stop", "Stop", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, _, cx| stop(v, cx))).into_any_element(),
            ],
            t,
        )
    } else {
        footer(
            vec![
                kit::act(t, "pr-cancel", "Cancel", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, window, _| close(v, window))).into_any_element(),
                kit::act(t, "pr-create", "Create pull request", ButtonKind::Primary, !ready(v, cx)).on_click(cx.listener(|v, _, _, cx| submit(v, cx))).into_any_element(),
            ],
            t,
        )
    };
    (content, bottom.into_any_element())
}

fn result_body(v: &PrView, t: &Theme, cx: &mut Context<PrView>) -> (AnyElement, AnyElement) {
    let (number, url, title) = match (&v.created, &v.existing) {
        (Some(o), _) if v.phase == Phase::Done => (o.number, o.url.clone(), String::new()),
        (_, Some(f)) => (Some(f.number).filter(|n| *n > 0), f.url.clone(), f.title.clone()),
        _ => (None, String::new(), String::new()),
    };
    let lead = if v.phase == Phase::Done {
        kit::ok_text(t, number.map(|n| format!("Pull request #{n} is created.")).unwrap_or_else(|| "The pull request is created.".into()))
    } else {
        kit::hint(t, number.map(|n| format!("Pull request #{n} is already open for this branch.")).unwrap_or_else(|| "A pull request is already open for this branch.".into()))
            .pt(px(10.))
            .text_size(px(12.5))
            .text_color(t.foreground)
    };
    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(lead)
        .when(!title.is_empty(), |el| el.child(div().pt(px(8.)).text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(t.foreground).child(title)))
        .when(!url.is_empty(), |el| el.child(div().pt(px(6.)).text_size(px(12.)).text_color(t.accent).child(url.clone())))
        .when(v.copied, |el| el.child(kit::hint(t, "Link copied.")))
        .into_any_element();
    let mut actions = vec![kit::act(t, "pr-done-close", "Close", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, window, _| close(v, window))).into_any_element()];
    if !url.is_empty() {
        let (u1, u2) = (url.clone(), url);
        actions.push(
            kit::act(t, "pr-copy", "Copy link", ButtonKind::Secondary, false)
                .on_click(cx.listener(move |v, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(u1.clone()));
                    v.copied = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        actions.push(kit::act(t, "pr-open", "Open", ButtonKind::Primary, false).on_click(move |_, _, cx| cx.open_url(&u2)).into_any_element());
    }
    (body, footer(actions, t).into_any_element())
}

fn failed_body(v: &PrView, t: &Theme, cx: &mut Context<PrView>) -> (AnyElement, AnyElement) {
    let e = v.error.clone().unwrap_or_else(|| fail("other", "Something went wrong", ""));
    (
        div().flex_1().min_h_0().pt(px(4.)).child(error_block(t, &e, "pr-failed", cx)).into_any_element(),
        footer(
            vec![
                kit::act(t, "pr-failed-close", "Close", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, window, _| close(v, window))).into_any_element(),
                kit::act(t, "pr-retry", "Try again", ButtonKind::Primary, false).on_click(cx.listener(|v, _, _, cx| load(v, cx))).into_any_element(),
            ],
            t,
        )
        .into_any_element(),
    )
}

impl Focusable for PrView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PrView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let (content, bottom) = match self.phase {
            Phase::Loading => (kit::busy_line(&t, "pr-loading", "Reading this repo and GitHub…").pt(px(16.)).into_any_element(), div().into_any_element()),
            Phase::Failed => failed_body(self, &t, cx),
            Phase::Exists | Phase::Done => result_body(self, &t, cx),
            Phase::Form => form(self, &t, cx),
        };
        div()
            .id("pr-create-view")
            .debug_selector(|| "pr-create-view".into())
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|v, event, window, cx| {
                match kit::window_key(event) {
                    Some(kit::WindowKey::Close) => close(v, window),
                    Some(kit::WindowKey::Submit) if ready(v, cx) => submit(v, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .size_full()
            .flex()
            .flex_col()
            .p(px(16.))
            .bg(t.background)
            .text_color(t.foreground)
            .child(div().pb(px(6.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(SharedString::from("Create pull request")))
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .child(bottom)
    }
}

pub fn open(root: Option<PathBuf>, cx: &mut App) {
    cx.defer(move |cx| open_now(kit::real(), real_deps(), root, cx));
}

pub fn open_now(ctx: Ctx, deps: PrDeps, root: Option<PathBuf>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("Create pull request".into()), appears_transparent: false, traffic_light_position: None }),
        window_min_size: Some(size(px(480.), px(420.))),
        ..WindowOptions::default()
    };
    cx.open_window(options, |window, cx| {
        let view = new(ctx, deps, root, cx);
        let title = view.read(cx).title.clone();
        window.focus(&title.focus_handle(cx));
        view
    })
    .ok();
}
