use super::branch_ops::{self, Branch, Compare, MergePlan, RemoteDeletePlan, ResetPlan};
use super::links::{self, Level};
use super::repo;
use super::run::{Cancel, cancel_flag};
use super::widgets::{Spec, busy_line, check, error_text, hint, note_view, section, selector, spec_button, spin, text_line};
use super::words::{Outcome, branch_name_problem, few_paths, plural, refusal, sentence};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Div, Entity, FocusHandle, Focusable, KeyDownEvent, SharedString, Stateful, Subscription, Window, div, px};
use nixa_ui::button::ButtonKind;
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub const WIDTH: f32 = 340.0;
pub const HEIGHT: f32 = 460.0;
pub const LOCAL_LIMIT: usize = 80;
pub const REMOTE_LIMIT: usize = 50;
const LABEL_NAME: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    Merge,
    Rebase,
    Reset,
    RemoteDelete,
    Rename,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub kind: AskKind,
    pub name: String,
    pub place: Place,
    pub no_ff: bool,
    pub remote: String,
    pub branch: String,
}

#[derive(Clone, Debug)]
pub enum PlanData {
    Merge(Result<MergePlan, Box<Outcome>>),
    Reset(Result<ResetPlan, Box<Outcome>>),
    RemoteDelete(Result<RemoteDeletePlan, Box<Outcome>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompareState {
    Loading,
    Done(Compare),
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchAsk {
    pub name: String,
    pub create: bool,
    pub count: usize,
    pub clash: bool,
    pub files: Vec<String>,
}

pub enum SwitchStep {
    Ask(usize),
    Done(Outcome),
}

type BranchWork = Arc<dyn Fn(PathBuf) -> Outcome + Send + Sync>;

#[derive(Clone)]
struct BranchRetry {
    label: String,
    work: BranchWork,
    switch_to: Option<String>,
}

pub struct BranchesView {
    pub repo: Option<PathBuf>,
    pub list: Option<Vec<Branch>>,
    pub error: Option<String>,
    pub filter: String,
    pub new_name: String,
    pub problem: Option<&'static str>,
    pub open: Option<String>,
    pub confirm: Option<(String, bool)>,
    pub pick_upstream: Option<String>,
    pub busy: Option<String>,
    retry: Option<BranchRetry>,
    pub note: Option<Outcome>,
    pub worktree: Option<(String, String)>,
    pub ask: Option<Ask>,
    pub plan: Option<(String, Option<PlanData>)>,
    pub compare: HashMap<String, (String, CompareState)>,
    pub rename_text: String,
    pub rename_problem: Option<&'static str>,
    pub job: Option<Cancel>,
    pub switch_ask: Option<SwitchAsk>,
    pub closed: bool,
    pub filter_input: Entity<TextInput>,
    pub fresh_input: Entity<TextInput>,
    pub rename_input: Entity<TextInput>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl Focusable for BranchesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn short_name(name: &str, fallback: &str) -> String {
    if name.chars().count() <= LABEL_NAME { name.to_string() } else { fallback.to_string() }
}

fn come_in(n: usize) -> String {
    if n == 1 { "1 new commit comes in".into() } else { format!("{n} new commits come in") }
}

fn only_here(n: usize) -> String {
    if n == 1 { "1 commit is only on this branch. It will be gone.".into() } else { format!("{n} commits are only on this branch. They will be gone.") }
}

fn line(text: impl Into<SharedString>, color: gpui::Hsla) -> Div {
    text_line(text, color, 11.5)
}

fn strip(name: &str) -> Stateful<Div> {
    div().id(SharedString::from(name.to_string())).flex_none().pl(px(22.0)).pr(px(4.0)).pt(px(1.0)).pb(px(6.0)).flex().flex_wrap().gap(px(6.0)).items_center()
}

fn not_on(name: &str) -> Outcome {
    refusal("notOn", format!("You are not on {name} now"), "Nothing was changed. Look at the list again, then try once more.")
}

fn still_on(root: &std::path::Path, name: &str) -> Option<Outcome> {
    (repo::current_branch(root).as_deref() != Some(name)).then(|| not_on(name))
}

fn move_to(root: &std::path::Path, name: &str, create: bool) -> Outcome {
    if create { branch_ops::create_branch(root, name, None, true) } else { branch_ops::checkout_branch(root, name) }
}

pub fn pill(t: &Theme, name: &str, label: impl Into<SharedString>, active: bool, color: gpui::Hsla) -> Stateful<Div> {
    let hover = t.hover;
    let el = div()
        .id(SharedString::from(name.to_string()))
        .flex_none()
        .h(px(20.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .rounded(px(5.0))
        .border_1()
        .border_color(if active { t.accent } else { t.border })
        .when(active, |el| el.bg(t.selected))
        .text_size(px(11.0))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label.into());
    selector(el, name)
}

impl BranchesView {
    pub fn new(repo: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter_input = cx.new(|cx| TextInput::new(cx, "Filter branches"));
        let fresh_input = cx.new(|cx| TextInput::new(cx, "New branch name"));
        let rename_input = cx.new(|cx| TextInput::new(cx, "New name"));
        let subs = vec![
            cx.subscribe_in(&filter_input, window, |v: &mut BranchesView, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(text) => {
                    v.filter = text.clone();
                    cx.notify();
                }
                InputEvent::Submit(_) => {
                    if let Some(name) = v.first_match() {
                        v.checkout(&name, window, cx);
                    }
                }
                InputEvent::Cancel => v.escape(window, cx),
            }),
            cx.subscribe_in(&fresh_input, window, |v: &mut BranchesView, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(text) => {
                    v.new_name = text.clone();
                    v.problem = if text.trim().is_empty() { None } else { branch_name_problem(text.trim()) };
                    cx.notify();
                }
                InputEvent::Submit(_) => v.create(window, cx),
                InputEvent::Cancel => v.escape(window, cx),
            }),
            cx.subscribe_in(&rename_input, window, |v: &mut BranchesView, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(text) => {
                    v.rename_text = text.clone();
                    v.rename_problem = match &v.ask {
                        Some(a) if a.kind == AskKind::Rename => v.rename_problem_of(text, &a.name.clone()),
                        _ => None,
                    };
                    cx.notify();
                }
                InputEvent::Submit(_) => v.rename(window, cx),
                InputEvent::Cancel => v.escape(window, cx),
            }),
        ];
        window.focus(&filter_input.focus_handle(cx));
        let mut view = BranchesView {
            repo,
            list: None,
            error: None,
            filter: String::new(),
            new_name: String::new(),
            problem: None,
            open: None,
            confirm: None,
            pick_upstream: None,
            busy: None,
            retry: None,
            note: None,
            worktree: None,
            ask: None,
            plan: None,
            compare: HashMap::new(),
            rename_text: String::new(),
            rename_problem: None,
            job: None,
            switch_ask: None,
            closed: false,
            filter_input,
            fresh_input,
            rename_input,
            focus: cx.focus_handle(),
            _subs: subs,
        };
        view.load(cx);
        view
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo.clone() else {
            self.list = Some(Vec::new());
            return;
        };
        let job = cx.background_executor().spawn(async move { branch_ops::list_branches(&root) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |v, cx| {
                match out {
                    Ok(list) => {
                        v.list = Some(list);
                        v.error = None;
                    }
                    Err(e) => {
                        v.list = Some(Vec::new());
                        v.error = Some(e);
                    }
                }
                v.compare.clear();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn visible(&self) -> branch_ops::Shown {
        branch_ops::visible(self.list.as_deref().unwrap_or_default(), &self.filter)
    }

    pub fn current(&self) -> Option<Branch> {
        self.list.as_ref()?.iter().find(|b| !b.remote && b.current).cloned()
    }

    fn remote_of(&self, b: &Branch) -> Option<Branch> {
        let up = b.upstream.as_ref().filter(|_| !b.gone)?;
        self.list.as_ref()?.iter().find(|r| r.remote && &r.name == up).cloned()
    }

    fn first_match(&self) -> Option<String> {
        let s = self.visible();
        s.local.iter().find(|b| !b.current).or(s.remote_only.first()).map(|b| b.name.clone())
    }

    pub fn close(&mut self, window: &mut Window) {
        self.closed = true;
        window.remove_window();
    }

    fn work<T: Send + 'static>(
        &mut self,
        label: String,
        job: impl FnOnce(PathBuf) -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let Some(root) = self.repo.clone() else { return };
        self.busy = Some(label);
        self.retry = None;
        self.note = None;
        self.worktree = None;
        let task = cx.background_executor().spawn(async move { job(root) });
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            this.update_in(cx, |v, window, cx| {
                v.busy = None;
                v.job = None;
                then(v, out, window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(repo) = self.repo.clone() {
            links::changed(&repo, cx);
        }
        self.load(cx);
    }

    pub fn clear_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask.as_ref().is_some_and(|a| a.kind == AskKind::Rename) {
            window.focus(&self.filter_input.focus_handle(cx));
        }
        self.ask = None;
        self.plan = None;
        self.confirm = None;
        self.pick_upstream = None;
        self.switch_ask = None;
    }

    fn switched(&mut self, r: Outcome, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if r.ok {
            if let Some(repo) = self.repo.clone() {
                links::changed(&repo, cx);
            }
            if r.stashed {
                links::notify(&r.title, &r.hint, Level::Success, cx);
            }
            self.close(window);
            return;
        }
        match (&r.kind, &r.folder) {
            (&"worktree", Some(folder)) => self.worktree = Some((r.branch.clone().unwrap_or(name.to_string()), folder.clone())),
            _ => self.note = Some(r),
        }
    }

    pub fn take_along(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.switch_ask.clone() else { return };
        let label = if ask.create { format!("Making {}…", ask.name) } else { format!("Switching to {}…", ask.name) };
        self.switch_ask = None;
        let (name, create) = (ask.name.clone(), ask.create);
        self.work(label, move |root| move_to(&root, &name, create), move |v, r, window, cx| {
            if !r.ok && r.kind == "dirty" {
                v.switch_ask = Some(SwitchAsk { clash: true, files: r.files.clone(), ..ask });
                return;
            }
            let name = ask.name.clone();
            v.switched(r, &name, window, cx);
        }, window, cx);
    }

    pub fn stash_and_go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.switch_ask.take() else { return };
        let label = format!("Stashing your changes, then {} {}…", if ask.create { "making" } else { "switching to" }, ask.name);
        let from = self.current().map(|b| b.name).unwrap_or_else(|| "HEAD".into());
        let (to, create) = (ask.name.clone(), ask.create);
        self.retryable(BranchRetry {
            label,
            work: Arc::new(move |root| branch_ops::stash_then_switch(&root, &from, &to, create)),
            switch_to: Some(ask.name),
        }, window, cx);
    }

    fn start_switch(&mut self, name: &str, create: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !create && self.current().is_some_and(|c| c.name == name) {
            self.close(window);
            return;
        }
        let label = if create { format!("Making {name}…") } else { format!("Switching to {name}…") };
        self.clear_ask(window, cx);
        let n = name.to_string();
        let job = move |root: PathBuf| match branch_ops::switch_changes(&root) {
            Ok(count) if count > 0 => SwitchStep::Ask(count),
            _ => SwitchStep::Done(move_to(&root, &n, create)),
        };
        let n = name.to_string();
        self.work(label, job, move |v, step, window, cx| match step {
            SwitchStep::Ask(count) => v.switch_ask = Some(SwitchAsk { name: n, create, count, clash: false, files: Vec::new() }),
            SwitchStep::Done(r) => v.switched(r, &n, window, cx),
        }, window, cx);
    }

    pub fn checkout(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.start_switch(name, false, window, cx);
    }

    pub fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_name.trim().to_string();
        if let Some(p) = branch_name_problem(&name) {
            self.problem = Some(p);
            cx.notify();
            return;
        }
        self.start_switch(&name, true, window, cx);
    }

    pub fn remove(&mut self, name: &str, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let n = name.to_string();
        self.work(format!("Deleting {name}…"), move |root| branch_ops::delete_branch(&root, &n, force), {
            let name = name.to_string();
            move |v: &mut Self, r: Outcome, _: &mut Window, cx: &mut Context<Self>| {
                if r.kind == "unmerged" && !force {
                    v.confirm = Some((name, true));
                    return;
                }
                v.confirm = None;
                let ok = r.ok;
                v.note = Some(r);
                if ok {
                    v.open = None;
                    v.changed(cx);
                }
            }
        }, window, cx);
    }

    pub fn track(&mut self, branch: &str, upstream: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (b, u) = (branch.to_string(), upstream.to_string());
        self.work(format!("Linking {branch}…"), move |root| branch_ops::set_upstream(&root, &b, Some(&u)), |v, r, _, cx| {
            let ok = r.ok;
            v.note = Some(r);
            if ok {
                v.pick_upstream = None;
                v.changed(cx);
            }
        }, window, cx);
    }

    fn open_folder(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = links::links(cx).open_folder {
            open(folder, cx);
        }
        self.close(window);
    }

    fn pick_profile(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let Some(job) = self.retry.clone() else {
            if let Some(open) = links::links(cx).open_identity {
                open(&repo, cx);
            }
            return;
        };
        let handle = window.window_handle();
        let view = cx.weak_entity();
        crate::screens::git::accounts::identity_win::open(repo, move |cx| {
            handle.update(cx, |_, window, cx| {
                view.update(cx, |v, cx| {
                    if v.retry.as_ref().is_some_and(|pending| Arc::ptr_eq(&pending.work, &job.work)) {
                        window.activate_window();
                        v.retryable(job, window, cx);
                    }
                }).ok();
            }).ok();
        }, cx);
    }

    pub fn rename_problem_of(&self, text: &str, from: &str) -> Option<&'static str> {
        let name = text.trim();
        if name.is_empty() || name == from {
            return None;
        }
        if let Some(p) = branch_name_problem(name) {
            return Some(p);
        }
        let lower = name.to_lowercase();
        let clash = self.list.iter().flatten().any(|b| !b.remote && b.name != from && b.name.to_lowercase() == lower);
        clash.then_some("A branch with this name already exists.")
    }

    fn load_compare(&mut self, name: &str, cx: &mut Context<Self>) {
        let (Some(cur), Some(root)) = (self.current(), self.repo.clone()) else { return };
        if self.compare.contains_key(name) {
            return;
        }
        self.compare.insert(name.to_string(), (cur.name.clone(), CompareState::Loading));
        let (a, b) = (name.to_string(), cur.name.clone());
        let job = cx.background_executor().spawn(async move { branch_ops::compare_counts(&root, &a, &b) });
        let key = name.to_string();
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |v, cx| {
                let state = match out {
                    Ok(c) => CompareState::Done(c),
                    Err(_) => CompareState::Failed,
                };
                v.compare.insert(key, (cur.name, state));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_plan(&mut self, key: String, fetch: impl FnOnce(PathBuf) -> PlanData + Send + 'static, cx: &mut Context<Self>) {
        let Some(root) = self.repo.clone() else { return };
        self.plan = Some((key.clone(), None));
        let job = cx.background_executor().spawn(async move { fetch(root) });
        cx.spawn(async move |this, cx| {
            let data = job.await;
            this.update(cx, |v, cx| {
                if v.plan.as_ref().is_some_and(|(k, _)| k == &key) {
                    v.plan = Some((key, Some(data)));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_ask(&mut self, kind: AskKind, b: &Branch, place: Place, remote: Option<(String, String)>, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_ask(window, cx);
        self.note = None;
        let (remote, branch) = remote.unwrap_or_default();
        self.ask = Some(Ask { kind, name: b.name.clone(), place, no_ff: false, remote: remote.clone(), branch: branch.clone() });
        let key = format!("{kind:?}:{place:?}:{}", b.name);
        let name = b.name.clone();
        match kind {
            AskKind::Merge | AskKind::Rebase => {
                let rebase = kind == AskKind::Rebase;
                self.load_plan(key, move |root| PlanData::Merge(branch_ops::merge_plan(&root, &name, rebase)), cx);
            }
            AskKind::Reset => self.load_plan(key, |root| PlanData::Reset(branch_ops::reset_plan(&root)), cx),
            AskKind::RemoteDelete => {
                self.load_plan(key, move |root| PlanData::RemoteDelete(branch_ops::remote_delete_plan(&root, &remote, &branch)), cx)
            }
            AskKind::Rename => {
                self.rename_text = b.name.clone();
                self.rename_problem = None;
                let value = b.name.clone();
                self.rename_input.update(cx, |i, cx| i.set_value(value, cx));
                window.focus(&self.rename_input.focus_handle(cx));
            }
        }
        cx.notify();
    }

    fn asking(&self, kind: AskKind, b: &Branch, place: Place) -> bool {
        self.ask.as_ref().is_some_and(|a| a.kind == kind && a.name == b.name && a.place == place)
    }

    pub fn rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.ask.clone().filter(|a| a.kind == AskKind::Rename) else { return };
        let to = self.rename_text.trim().to_string();
        let problem = if to == ask.name { Some("Type a different name.") } else { self.rename_problem_of(&to, &ask.name).or(branch_name_problem(&to)) };
        if let Some(p) = problem {
            self.rename_problem = Some(p);
            cx.notify();
            return;
        }
        let (from, target) = (ask.name.clone(), to.clone());
        self.work(format!("Renaming {}…", ask.name), move |root| branch_ops::rename_branch(&root, &from, &target), move |v, r, window, cx| {
            let ok = r.ok;
            v.note = Some(r);
            if ok {
                v.clear_ask(window, cx);
                v.open = Some(to);
                v.changed(cx);
            }
        }, window, cx);
    }

    pub fn forward(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let n = name.to_string();
        let label = format!("Moving {name} forward…");
        let name = name.to_string();
        self.work(label, move |root| branch_ops::fast_forward(&root, &n), move |v, r, _, cx| {
            if let (&"worktree", Some(folder)) = (&r.kind, &r.folder) {
                v.worktree = Some((name, folder.clone()));
                return;
            }
            let moved = r.ok && !r.up_to_date;
            v.note = Some(r);
            if moved {
                v.changed(cx);
            }
        }, window, cx);
    }

    fn finish_big(&mut self, r: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        if r.kind == "notOn" {
            self.clear_ask(window, cx);
            self.note = Some(r);
            self.load(cx);
            return;
        }
        self.ask = None;
        self.plan = None;
        if r.ok && !r.warning {
            links::notify(&r.title, &r.hint, Level::Success, cx);
            if !r.up_to_date && let Some(repo) = self.repo.clone() {
                links::changed(&repo, cx);
            }
            self.close(window);
            return;
        }
        let kind = r.kind;
        let ok = r.ok;
        self.note = Some(r);
        if ok || kind == "conflict" || kind == "paused" {
            self.changed(cx);
        } else if kind == "headChanged" {
            self.load(cx);
        }
    }

    fn merge_heads(&self) -> (Option<String>, Option<String>) {
        match &self.plan {
            Some((_, Some(PlanData::Merge(Ok(p))))) => (Some(p.head.clone()), Some(p.theirs.clone())),
            _ => (None, None),
        }
    }

    fn retryable(&mut self, pending: BranchRetry, window: &mut Window, cx: &mut Context<Self>) {
        let work = pending.work.clone();
        self.work(pending.label.clone(), move |root| work(root), move |v, out, window, cx| {
            let switch_to = pending.switch_to.clone();
            if out.needs_profile {
                v.retry = Some(pending);
            }
            if let Some(branch) = switch_to {
                v.switched(out, &branch, window, cx);
            } else {
                v.finish_big(out, window, cx);
            }
        }, window, cx);
    }

    pub fn merge(&mut self, name: &str, into: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (head, theirs) = self.merge_heads();
        let no_ff = self.ask.as_ref().is_some_and(|a| a.no_ff);
        let (n, i) = (name.to_string(), into.to_string());
        self.retryable(BranchRetry {
            label: format!("Merging {name}…"),
            work: Arc::new(move |root| {
                still_on(&root, &i).unwrap_or_else(|| branch_ops::merge_into(&root, &n, no_ff, head.as_deref(), theirs.as_deref()))
            }),
            switch_to: None,
        }, window, cx);
    }

    pub fn rebase(&mut self, name: &str, from: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (head, theirs) = self.merge_heads();
        let (n, f) = (name.to_string(), from.to_string());
        self.retryable(BranchRetry {
            label: format!("Rebasing {from} onto {name}…"),
            work: Arc::new(move |root| {
                still_on(&root, &f).unwrap_or_else(|| branch_ops::rebase_onto(&root, &n, head.as_deref(), theirs.as_deref()))
            }),
            switch_to: None,
        }, window, cx);
    }

    pub fn reset(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (head, target) = match &self.plan {
            Some((_, Some(PlanData::Reset(Ok(p))))) => (p.head.clone(), Some(p.target.clone())),
            _ => (None, None),
        };
        let n = name.to_string();
        self.work(format!("Resetting {name}…"), move |root| {
            still_on(&root, &n).unwrap_or_else(|| branch_ops::reset_to_remote(&root, head.as_deref(), target.as_deref()))
        }, |v, r, window, cx| v.finish_big(r, window, cx), window, cx);
    }

    pub fn delete_remote(&mut self, remote: &str, branch: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let sha = match &self.plan {
            Some((_, Some(PlanData::RemoteDelete(Ok(p))))) => p.sha.clone(),
            _ => None,
        };
        let cancel = cancel_flag();
        let flag = cancel.clone();
        let (r, b) = (remote.to_string(), branch.to_string());
        self.work(format!("Deleting {branch} on {remote}…"), move |root| branch_ops::delete_remote_branch(&root, &r, &b, sha.as_deref(), Some(&flag)), |v, r, _, cx| {
            v.ask = None;
            v.plan = None;
            let (ok, kind) = (r.ok, r.kind);
            v.note = Some(r);
            if ok {
                v.changed(cx);
            } else if kind == "headChanged" {
                v.load(cx);
            }
        }, window, cx);
        self.job = Some(cancel);
    }

    pub fn toggle(&mut self, key: &str, b: &Branch, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.open.as_deref() == Some(key);
        self.open = if open { None } else { Some(key.to_string()) };
        self.clear_ask(window, cx);
        if !open && !b.current {
            self.load_compare(&b.name, cx);
        }
        cx.notify();
    }

    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask.is_some() || self.confirm.is_some() || self.pick_upstream.is_some() || self.switch_ask.is_some() {
            self.clear_ask(window, cx);
            cx.notify();
            return;
        }
        self.close(window);
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key == "escape" {
            cx.stop_propagation();
            self.escape(window, cx);
        }
    }

    fn btn(&self, t: &Theme, s: Spec, cx: &mut Context<Self>, f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> AnyElement {
        let off = s.off;
        spec_button(t, &s).when(!off, |el| el.on_click(cx.listener(move |v, _, window, cx| f(v, window, cx)))).into_any_element()
    }

    fn yes_no(&self, t: &Theme, s: Spec, cx: &mut Context<Self>, f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> Vec<AnyElement> {
        let name = s.name.clone();
        let off = self.busy.is_some() || s.off;
        vec![
            self.btn(t, Spec::new(&format!("{name}-yes"), s.label, s.kind).off(off), cx, f),
            self.btn(t, Spec::new(&format!("{name}-no"), "No", ButtonKind::Secondary).off(false).tip(None), cx, |v, w, cx| {
                v.clear_ask(w, cx);
                cx.notify();
            }),
        ]
    }

    fn track_info(t: &Theme, b: &Branch, i: usize) -> AnyElement {
        let name = SharedString::from(format!("br-track-{i}"));
        if b.gone {
            let up = b.upstream.clone().unwrap_or_default();
            return div().id(name).flex_none().text_size(px(10.5)).text_color(t.red).tooltip(tooltip(format!("{up} was deleted on the remote"))).child("gone").into_any_element();
        }
        let Some(up) = &b.upstream else {
            return div().id(name).flex_none().text_size(px(10.5)).text_color(t.dim).tooltip(tooltip("Not on the remote yet")).child("local").into_any_element();
        };
        div()
            .id(name)
            .flex_none()
            .flex()
            .gap(px(4.0))
            .text_size(px(10.5))
            .tooltip(tooltip(format!("Follows {up}")))
            .when(b.ahead > 0, |el| el.child(div().text_color(t.accent).child(format!("↑{}", b.ahead))))
            .when(b.behind > 0, |el| el.child(div().text_color(t.yellow).child(format!("↓{}", b.behind))))
            .when(b.ahead == 0 && b.behind == 0, |el| el.child(div().text_color(t.green).child("✓")))
            .into_any_element()
    }

    fn compare_line(&self, t: &Theme, b: &Branch) -> Option<AnyElement> {
        let (with, state) = self.compare.get(&b.name)?;
        match state {
            CompareState::Loading => Some(busy_line(t, &format!("br-cmp-{}", b.name), format!("Comparing with {with}…")).w_full().into_any_element()),
            CompareState::Failed => None,
            CompareState::Done(c) => {
                let text = if c.ahead == 0 && c.behind == 0 { format!("Same commits as {with}.") } else { format!("Compared with {with}: {} ahead, {} behind.", c.ahead, c.behind) };
                Some(hint(t, text).into_any_element())
            }
        }
    }

    fn checking(t: &Theme, name: &str) -> AnyElement {
        busy_line(t, name, "Checking…").w_full().into_any_element()
    }

    fn plan_error(t: &Theme, o: &Outcome) -> AnyElement {
        let title = sentence(if o.title.is_empty() { "Something went wrong" } else { &o.title });
        let text = [title, o.hint.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        line(sentence(&text), t.red).into_any_element()
    }

    fn merge_plan(&self) -> Option<&Result<MergePlan, Box<Outcome>>> {
        match &self.plan {
            Some((_, Some(PlanData::Merge(p)))) => Some(p),
            _ => None,
        }
    }

    fn plan_lines(&self, t: &Theme, b: &Branch, verb: &str, name: &str) -> (Vec<AnyElement>, bool) {
        let Some(plan) = self.merge_plan() else { return (vec![Self::checking(t, name)], true) };
        let plan = match plan {
            Ok(p) => p,
            Err(o) => return (vec![Self::plan_error(t, o)], true),
        };
        if plan.up_to_date {
            return (Vec::new(), true);
        }
        if plan.staged.mixed > 0 {
            return (vec![line("Some files have staged and unstaged parts. Commit or stash them first.", t.red).into_any_element()], true);
        }
        if !plan.blocked.is_empty() {
            let one = plan.blocked.len() == 1;
            let them = if one { "it" } else { "them" };
            let text = format!(
                "{} has {} that git does not track here: {}. The {verb} would write over {them}. Move {them} out of the folder first.",
                b.name,
                if one { "a file" } else { "files" },
                few_paths(&plan.blocked, 4)
            );
            return (vec![line(text, t.red).into_any_element()], true);
        }
        let staged = if plan.staged.staged > 0 { " Staged changes come back as unstaged." } else { "" };
        let lines = if plan.staged.dirty > 0 {
            vec![text_line(format!("Your changes in {} are put aside and brought back after.{staged}", plural(plan.staged.dirty, "file")), t.dim, 11.0).into_any_element()]
        } else {
            Vec::new()
        };
        (lines, false)
    }

    fn ask_merge(&self, t: &Theme, b: &Branch, name: &str, cur: &Branch, cx: &mut Context<Self>) -> AnyElement {
        let (lines, blocked) = self.plan_lines(t, b, "merge", &format!("{name}-check"));
        let no_ff = self.ask.as_ref().is_some_and(|a| a.no_ff);
        let plan = self.merge_plan().and_then(|p| p.as_ref().ok());
        let what = plan.map(|p| {
            if p.up_to_date {
                format!("{} already has every commit of {}. Nothing to merge.", cur.name, b.name)
            } else if p.fast_forward && !no_ff {
                format!("{}. {} just moves forward. No merge commit is made.", come_in(p.incoming), cur.name)
            } else {
                format!("{}, with a new merge commit on {}.", come_in(p.incoming), cur.name)
            }
        });
        let ff = plan.is_some_and(|p| p.fast_forward);
        let (bn, cn) = (b.name.clone(), cur.name.clone());
        strip(name)
            .child(text_line(format!("Merge {} into {}?", b.name, cur.name), t.foreground, 12.0))
            .when_some(what, |el, w| el.child(line(w, t.muted)))
            .children(lines)
            .when(ff, |el| {
                el.child(div().w_full().child(check(t, &format!("{name}-noff"), no_ff, "Always make a merge commit").on_click(cx.listener(|v, _, _, cx| {
                    if let Some(a) = v.ask.as_mut() {
                        a.no_ff = !a.no_ff;
                    }
                    cx.notify();
                }))))
            })
            .children(self.yes_no(t, Spec::new(name, "Yes, merge", ButtonKind::Primary).off(blocked), cx, move |v, w, cx| v.merge(&bn, &cn, w, cx)))
            .into_any_element()
    }

    fn ask_rebase(&self, t: &Theme, b: &Branch, name: &str, cur: &Branch, cx: &mut Context<Self>) -> AnyElement {
        let (lines, blocked) = self.plan_lines(t, b, "rebase", &format!("{name}-check"));
        let plan = self.merge_plan().and_then(|p| p.as_ref().ok());
        let what = plan.map(|p| {
            if p.up_to_date {
                format!("{} already has every commit of {}. Nothing to do.", cur.name, b.name)
            } else if p.fast_forward {
                format!("{} has no commits of its own, so it just moves forward to {}.", cur.name, b.name)
            } else if p.own == 1 {
                format!("Your 1 commit on {} is copied on top of {}. It gets a new id.", cur.name, b.name)
            } else {
                format!("Your {} commits on {} are copied on top of {}, one by one. They get new ids.", p.own, cur.name, b.name)
            }
        });
        let pushed = plan.filter(|p| !p.up_to_date && !p.fast_forward && p.pushed > 0).map(|p| {
            let already = if p.own == 1 { "It is".to_string() } else if p.pushed == p.own { "All of them are".to_string() } else { format!("{} of them are", p.pushed) };
            format!("{already} on {} already. After this you must force push {}.", p.upstream.clone().unwrap_or_default(), cur.name)
        });
        let (bn, cn) = (b.name.clone(), cur.name.clone());
        strip(name)
            .child(text_line(format!("Rebase {} onto {}?", cur.name, b.name), t.foreground, 12.0))
            .when_some(what, |el, w| el.child(line(w, t.muted)))
            .when_some(pushed, |el, w| el.child(line(w, t.yellow)))
            .children(lines)
            .children(self.yes_no(t, Spec::new(name, "Yes, rebase", ButtonKind::Primary).off(blocked), cx, move |v, w, cx| v.rebase(&bn, &cn, w, cx)))
            .into_any_element()
    }

    fn ask_reset(&self, t: &Theme, b: &Branch, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let up = b.upstream.clone().unwrap_or_default();
        let el = strip(name).child(text_line(format!("Reset {} to {up}?", b.name), t.red, 12.0));
        let bn = b.name.clone();
        let yes = |off: bool, cx: &mut Context<Self>| self.yes_no(t, Spec::new(name, "Yes, reset", ButtonKind::Danger).off(off), cx, move |v, w, cx| v.reset(&bn, w, cx));
        let plan = match &self.plan {
            Some((_, Some(PlanData::Reset(p)))) => p,
            _ => return el.child(Self::checking(t, &format!("{name}-check"))).children(yes(true, cx)).into_any_element(),
        };
        let plan = match plan {
            Ok(p) => p,
            Err(o) => return el.child(Self::plan_error(t, o)).children(yes(true, cx)).into_any_element(),
        };
        let lost: Vec<String> = [
            (plan.ahead > 0).then(|| format!("{} not pushed yet", plural(plan.ahead, "commit"))),
            (plan.dirty > 0).then(|| format!("the changes in {}", plural(plan.dirty, "file"))),
        ]
        .into_iter()
        .flatten()
        .collect();
        let files = &plan.untracked;
        let lost_text = if lost.is_empty() { String::new() } else { format!("This throws away {}.", lost.join(" and ")) };
        let files_text = match files.len() {
            0 => String::new(),
            1 => format!("{} also has a file that git does not track here. It will be written over: {}.", plan.upstream, few_paths(files, 4)),
            _ => format!("{} also has files that git does not track here. They will be written over: {}.", plan.upstream, few_paths(files, 4)),
        };
        let last = " It cannot be undone.";
        el.when(!lost_text.is_empty() || files.is_empty(), |el| {
            let text = if lost_text.is_empty() { "You have no local commits or changes, so nothing is lost.".to_string() } else { format!("{lost_text}{last}") };
            el.child(line(text, t.muted))
        })
        .when(!files.is_empty(), |el| el.child(line(format!("{files_text}{}", if lost_text.is_empty() { last } else { "" }), t.red)))
        .child(text_line(format!("{} will match {} from the last fetch.", b.name, plan.upstream), t.dim, 11.0))
        .children(yes(false, cx))
        .into_any_element()
    }

    fn ask_remote_delete(&self, t: &Theme, b: &Branch, name: &str, target: (&str, &str), local: bool, cx: &mut Context<Self>) -> AnyElement {
        let (remote, short) = target;
        let plan = match &self.plan {
            Some((_, Some(PlanData::RemoteDelete(p)))) => Some(p),
            _ => None,
        };
        let only = plan.and_then(|p| p.as_ref().ok()).map_or(0, |p| p.only);
        let ok = plan.is_some_and(|p| p.is_ok());
        let what = if local { format!("This removes it on the remote for everyone. {} here stays.", b.name) } else { "This removes it on the remote for everyone, and from this list.".to_string() };
        let (r, s) = (remote.to_string(), short.to_string());
        strip(name)
            .child(text_line(format!("Delete {short} on {remote}?"), t.red, 12.0))
            .child(line(what, t.muted))
            .when(plan.is_none(), |el| el.child(Self::checking(t, &format!("{name}-check"))))
            .when_some(plan.and_then(|p| p.as_ref().err()), |el, o| el.child(Self::plan_error(t, o)))
            .when(only > 0, |el| el.child(line(only_here(only), t.red)))
            .children(self.yes_no(t, Spec::new(name, "Yes, delete", ButtonKind::Danger).off(!ok), cx, move |v, w, cx| v.delete_remote(&r, &s, w, cx)))
            .into_any_element()
    }

    fn ask_rename(&self, t: &Theme, b: &Branch, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let text = self.rename_text.trim().to_string();
        let off = self.busy.is_some() || text.is_empty() || text == b.name || self.rename_problem.is_some();
        strip(name)
            .child(line(format!("New name for {}", b.name), t.muted))
            .child(div().w_full().child(self.rename_input.clone()))
            .when_some(self.rename_problem, |el, p| el.child(text_line(p, t.red, 11.0)))
            .when_some(b.upstream.clone().filter(|_| !b.gone), |el, up| el.child(text_line(format!("It keeps following {up}. The remote branch keeps its old name."), t.dim, 11.0)))
            .child(self.btn(t, Spec::new(&format!("{name}-yes"), "Rename", ButtonKind::Primary).off(off).tip(None), cx, |v, w, cx| v.rename(w, cx)))
            .child(self.btn(t, Spec::new(&format!("{name}-no"), "Cancel", ButtonKind::Secondary).off(false).tip(None), cx, |v, w, cx| {
                v.clear_ask(w, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    fn join_buttons(&self, t: &Theme, b: &Branch, i: usize, cur: Option<&Branch>, place: Place, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(cur) = cur else { return Vec::new() };
        if place == Place::Local && b.name == cur.name {
            return Vec::new();
        }
        let prefix = if place == Place::Local { "br" } else { "br-r" };
        let busy = self.busy.is_some();
        let short = short_name(&cur.name, "current");
        let (b1, b2) = (b.clone(), b.clone());
        vec![
            self.btn(t, Spec::new(&format!("{prefix}-merge-{i}"), format!("Merge into {short}"), ButtonKind::Secondary).off(busy).tip(Some(format!("Bring the commits of {} into {} (asks first)", b.name, cur.name))), cx, move |v, w, cx| {
                v.open_ask(AskKind::Merge, &b1, place, None, w, cx)
            }),
            self.btn(t, Spec::new(&format!("{prefix}-rebase-{i}"), format!("Rebase {short} onto this"), ButtonKind::Secondary).off(busy).tip(Some(format!("Move the commits of {} on top of {} (asks first)", cur.name, b.name))), cx, move |v, w, cx| {
                v.open_ask(AskKind::Rebase, &b2, place, None, w, cx)
            }),
        ]
    }

    fn action_strip(&self, t: &Theme, b: &Branch, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let name = format!("br-actions-{i}");
        let cur = self.current();
        let target = self.remote_of(b);
        if let Some((_, force)) = self.confirm.clone().filter(|(n, _)| n == &b.name) {
            let bn = b.name.clone();
            let text = if force { format!("{} has commits that are not merged. Delete it anyway?", b.name) } else { format!("Delete {}?", b.name) };
            return strip(&name)
                .child(line(text, t.red))
                .child(self.btn(t, Spec::new(&format!("br-del-yes-{i}"), if force { "Delete anyway" } else { "Delete" }, ButtonKind::Danger).off(busy).tip(None), cx, move |v, w, cx| v.remove(&bn, force, w, cx)))
                .child(self.btn(t, Spec::new(&format!("br-del-no-{i}"), "Keep", ButtonKind::Secondary).off(false).tip(None), cx, |v, _, cx| {
                    v.confirm = None;
                    cx.notify();
                }))
                .into_any_element();
        }
        if self.pick_upstream.as_deref() == Some(b.name.as_str()) {
            let choices: Vec<Branch> = self.list.iter().flatten().filter(|r| r.remote && r.short_name == b.name).take(6).cloned().collect();
            let text = if choices.is_empty() { "No remote branch has this name yet. Push this branch to publish it." } else { "Follow which remote branch?" };
            let mut el = strip(&name).child(line(text, t.muted));
            for (k, r) in choices.into_iter().enumerate() {
                let (bn, rn) = (b.name.clone(), r.name.clone());
                el = el.child(pill(t, &format!("br-up-pick-{i}-{k}"), r.name.clone(), false, t.accent).when(!busy, |el| el.on_click(cx.listener(move |v, _, w, cx| v.track(&bn, &rn, w, cx)))));
            }
            return el
                .child(self.btn(t, Spec::new(&format!("br-up-back-{i}"), "Back", ButtonKind::Secondary).off(false).tip(None), cx, |v, _, cx| {
                    v.pick_upstream = None;
                    cx.notify();
                }))
                .into_any_element();
        }
        if self.asking(AskKind::Rename, b, Place::Local) {
            return self.ask_rename(t, b, &format!("br-ask-rename-{i}"), cx);
        }
        if let Some(c) = cur.as_ref() {
            if self.asking(AskKind::Merge, b, Place::Local) {
                return self.ask_merge(t, b, &format!("br-ask-merge-{i}"), c, cx);
            }
            if self.asking(AskKind::Rebase, b, Place::Local) {
                return self.ask_rebase(t, b, &format!("br-ask-rebase-{i}"), c, cx);
            }
        }
        if self.asking(AskKind::Reset, b, Place::Local) {
            return self.ask_reset(t, b, &format!("br-ask-reset-{i}"), cx);
        }
        if let Some(tg) = target.as_ref().filter(|_| self.asking(AskKind::RemoteDelete, b, Place::Local)) {
            let remote = tg.remote_name.clone().unwrap_or_default();
            return self.ask_remote_delete(t, b, &format!("br-ask-rdel-{i}"), (&remote, &tg.short_name), true, cx);
        }
        let up = b.upstream.clone().filter(|_| !b.gone);
        let can_forward = up.is_some() && b.behind > 0 && b.ahead == 0 && b.worktree.is_none();
        let compare = if b.current { None } else { self.compare_line(t, b) };
        let (b1, b2, b3, b4) = (b.clone(), b.clone(), b.clone(), b.clone());
        let joins = self.join_buttons(t, b, i, cur.as_ref(), Place::Local, cx);
        let up_text = up.clone().unwrap_or_default();
        let mut el = strip(&name).when_some(compare, |el, c| el.child(c)).children(joins);
        if can_forward {
            el = el.child(self.btn(t, Spec::new(&format!("br-ff-{i}"), "Fast-forward", ButtonKind::Secondary).off(busy).tip(Some(format!("Move {} up to {up_text}. Uses the last fetch. No new commit.", b.name))), cx, move |v, w, cx| v.forward(&b1.name, w, cx)));
        }
        if b.current && up.is_some() {
            el = el.child(self.btn(t, Spec::new(&format!("br-reset-{i}"), "Reset to remote", ButtonKind::Danger).off(busy).tip(Some(format!("Make {} match {up_text} and throw away local work (asks first)", b.name))), cx, move |v, w, cx| v.open_ask(AskKind::Reset, &b2, Place::Local, None, w, cx)));
        }
        el = el.child(self.btn(t, Spec::new(&format!("br-rename-{i}"), "Rename", ButtonKind::Secondary).off(busy).tip(Some("Give this branch a new name".into())), cx, move |v, w, cx| v.open_ask(AskKind::Rename, &b3, Place::Local, None, w, cx)));
        if !b.current {
            let bn = b.name.clone();
            el = el.child(self.btn(t, Spec::new(&format!("br-del-{i}"), "Delete", ButtonKind::Danger).off(busy).tip(Some("Delete this branch (asks first)".into())), cx, move |v, w, cx| {
                v.clear_ask(w, cx);
                v.confirm = Some((bn.clone(), false));
                cx.notify();
            }));
        }
        if up.is_none() {
            let bn = b.name.clone();
            el = el.child(self.btn(t, Spec::new(&format!("br-up-{i}"), "Set upstream", ButtonKind::Secondary).off(busy).tip(Some("Pick the remote branch this one follows".into())), cx, move |v, w, cx| {
                v.clear_ask(w, cx);
                v.pick_upstream = Some(bn.clone());
                cx.notify();
            }));
        }
        if let Some(tg) = target {
            let pair = (tg.remote_name.clone().unwrap_or_default(), tg.short_name.clone());
            el = el.child(self.btn(t, Spec::new(&format!("br-rdel-{i}"), "Delete on remote", ButtonKind::Danger).off(busy).tip(Some(format!("Delete {up_text} on the remote (asks first)"))), cx, move |v, w, cx| {
                v.open_ask(AskKind::RemoteDelete, &b4, Place::Local, Some(pair.clone()), w, cx)
            }));
        }
        if let Some(folder) = b.worktree.clone() {
            el = el.child(self.btn(t, Spec::new(&format!("br-open-{i}"), "Open that worktree", ButtonKind::Secondary).off(false).tip(None), cx, move |v, w, cx| v.open_folder(&folder, w, cx)));
        }
        el.when(up.is_some(), |el| el.child(div().text_size(px(11.0)).text_color(t.dim).overflow_hidden().whitespace_nowrap().text_ellipsis().child(format!("Follows {up_text}"))))
            .when(!b.subject.is_empty(), |el| el.child(div().w_full().text_size(px(11.0)).text_color(t.dim).overflow_hidden().whitespace_nowrap().text_ellipsis().child(format!("Last commit: {}", b.subject))))
            .into_any_element()
    }

    fn remote_strip(&self, t: &Theme, b: &Branch, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let cur = self.current();
        let remote = b.remote_name.clone().unwrap_or_default();
        if let Some(c) = cur.as_ref() {
            if self.asking(AskKind::Merge, b, Place::Remote) {
                return self.ask_merge(t, b, &format!("br-r-ask-merge-{i}"), c, cx);
            }
            if self.asking(AskKind::Rebase, b, Place::Remote) {
                return self.ask_rebase(t, b, &format!("br-r-ask-rebase-{i}"), c, cx);
            }
        }
        if self.asking(AskKind::RemoteDelete, b, Place::Remote) {
            return self.ask_remote_delete(t, b, &format!("br-r-ask-rdel-{i}"), (&remote, &b.short_name), false, cx);
        }
        let compare = self.compare_line(t, b);
        let joins = self.join_buttons(t, b, i, cur.as_ref(), Place::Remote, cx);
        let bb = b.clone();
        let pair = (remote.clone(), b.short_name.clone());
        strip(&format!("br-ractions-{i}"))
            .when_some(compare, |el, c| el.child(c))
            .children(joins)
            .child(self.btn(t, Spec::new(&format!("br-r-rdel-{i}"), "Delete on remote", ButtonKind::Danger).off(busy).tip(Some(format!("Delete {} on {remote} (asks first)", b.short_name))), cx, move |v, w, cx| {
                v.open_ask(AskKind::RemoteDelete, &bb, Place::Remote, Some(pair.clone()), w, cx)
            }))
            .when(!b.subject.is_empty(), |el| el.child(div().w_full().text_size(px(11.0)).text_color(t.dim).overflow_hidden().whitespace_nowrap().text_ellipsis().child(format!("Last commit: {}", b.subject))))
            .into_any_element()
    }

    fn local_row(&self, t: &Theme, b: &Branch, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let open = self.open.as_deref() == Some(b.name.as_str());
        let busy = self.busy.is_some();
        let hover = t.hover;
        let (b1, b2) = (b.clone(), b.clone());
        let name = format!("br-local-{i}");
        let row = div()
            .id(SharedString::from(name.clone()))
            .h(px(26.0))
            .px(px(6.0))
            .flex()
            .gap(px(6.0))
            .items_center()
            .rounded(px(6.0))
            .when(b.current, |el| el.bg(t.selected))
            .when(!b.current && !busy, |el| el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |v, _, w, cx| v.checkout(&b1.name, w, cx))))
            .child(div().w(px(10.0)).flex_none().text_size(px(10.0)).text_color(t.accent).child(if b.current { "●" } else { "" }))
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).text_color(t.foreground).child(b.name.clone()))
            .when(b.worktree.is_some(), |el| el.child(div().flex_none().text_size(px(10.5)).text_color(t.dim).child("other folder")))
            .child(Self::track_info(t, b, i))
            .child(
                pill(t, &format!("br-more-{i}"), "⋯", open, t.muted)
                    .tooltip(tooltip("More for this branch"))
                    .on_click(cx.listener(move |v, _, w, cx| {
                        cx.stop_propagation();
                        let key = b2.name.clone();
                        v.toggle(&key, &b2, w, cx)
                    })),
            );
        div()
            .flex_none()
            .flex()
            .flex_col()
            .child(selector(row, &name))
            .when(open, |el| el.child(self.action_strip(t, b, i, cx)))
            .into_any_element()
    }

    fn remote_row(&self, t: &Theme, b: &Branch, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let key = format!("remote:{}", b.name);
        let open = self.open.as_deref() == Some(key.as_str());
        let hover = t.hover;
        let (b1, b2) = (b.clone(), b.clone());
        let name = format!("br-remote-{i}");
        let row = div()
            .id(SharedString::from(name.clone()))
            .flex_none()
            .h(px(26.0))
            .px(px(6.0))
            .flex()
            .gap(px(6.0))
            .items_center()
            .rounded(px(6.0))
            .when(!busy, |el| el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |v, _, w, cx| v.checkout(&b1.name, w, cx))))
            .child(div().w(px(10.0)).flex_none())
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).text_color(t.muted).child(b.name.clone()))
            .child(div().flex_none().text_size(px(10.5)).text_color(t.dim).child("check out"))
            .child(
                pill(t, &format!("br-rmore-{i}"), "⋯", open, t.muted)
                    .tooltip(tooltip("More for this remote branch"))
                    .on_click(cx.listener(move |v, _, w, cx| {
                        cx.stop_propagation();
                        v.toggle(&key, &b2, w, cx)
                    })),
            );
        div()
            .flex_none()
            .flex()
            .flex_col()
            .child(selector(row, &name))
            .when(open, |el| el.child(self.remote_strip(t, b, i, cx)))
            .into_any_element()
    }

    fn switch_view(&self, t: &Theme, ask: &SwitchAsk, cx: &mut Context<Self>) -> AnyElement {
        let here = self.current().map(|b| b.name).unwrap_or_else(|| "this branch".into());
        let target = if ask.create { format!("the new branch {}", ask.name) } else { ask.name.clone() };
        let head = if ask.clash {
            let files = if ask.files.is_empty() {
                String::new()
            } else {
                let more = if ask.files.len() > 4 { format!(" and {} more", ask.files.len() - 4) } else { String::new() };
                format!(" In the way: {}{more}.", ask.files.iter().take(4).cloned().collect::<Vec<_>>().join(", "))
            };
            format!("Git cannot take your changes to {}: they clash with files there.{files}", ask.name)
        } else {
            format!("You have changes in {}. What should happen to them?", plural(ask.count, "file"))
        };
        let tip = if ask.clash {
            format!("Stash them: they wait on {here}, and the panel offers them back when you return.")
        } else {
            format!("Take them: they come with you to {target}. Stash them: they wait on {here}, and the panel offers them back when you return.")
        };
        let mut buttons: Vec<AnyElement> = Vec::new();
        if !ask.clash {
            buttons.push(self.btn(t, Spec::new("br-switch-take", format!("Take them to {}", short_name(&ask.name, "it")), ButtonKind::Primary).off(false).tip(Some(format!("Switch to {} with your changes", ask.name))), cx, |v, w, cx| v.take_along(w, cx)));
        }
        let kind = if ask.clash { ButtonKind::Primary } else { ButtonKind::Secondary };
        buttons.push(self.btn(t, Spec::new("br-switch-stash", "Stash them here", kind).off(false).tip(Some(format!("Stash the changes on {here}, then switch to {}", ask.name))), cx, |v, w, cx| v.stash_and_go(w, cx)));
        buttons.push(self.btn(t, Spec::new("br-switch-cancel", "Cancel", ButtonKind::Secondary).off(false).tip(Some("Stay on this branch".into())), cx, |v, _, cx| {
            v.switch_ask = None;
            cx.notify();
        }));
        let el = div()
            .id("br-switch-ask")
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.border)
            .bg(t.raised)
            .child(text_line(head, if ask.clash { t.yellow } else { t.foreground }, 12.0))
            .child(hint(t, tip))
            .child(div().flex().gap(px(6.0)).flex_wrap().children(buttons));
        selector(el, "br-switch-ask").into_any_element()
    }

    fn status_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(ask) = self.switch_ask.as_ref().filter(|_| self.busy.is_none()) {
            return Some(self.switch_view(t, &ask.clone(), cx));
        }
        if let Some(label) = &self.busy {
            let stopping = self.job.as_ref().map(|j| j.load(Ordering::SeqCst));
            return Some(
                div()
                    .flex_none()
                    .flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(spin(t, "br-busy"))
                    .child(div().flex_1().min_w(px(0.0)).child(hint(t, label.clone())))
                    .when_some(stopping, |el, stopping| {
                        el.child(self.btn(t, Spec::new("br-job-stop", if stopping { "Stopping…" } else { "Stop" }, ButtonKind::Secondary).off(stopping).tip(Some("Stop waiting for the remote".into())), cx, |v, _, cx| {
                            if let Some(j) = &v.job {
                                j.store(true, Ordering::SeqCst);
                            }
                            cx.notify();
                        }))
                    })
                    .into_any_element(),
            );
        }
        if let Some((name, folder)) = self.worktree.clone() {
            let f = folder.clone();
            return Some(
                div()
                    .id("br-worktree")
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(text_line(format!("{name} is open in another folder:"), t.yellow, 12.0))
                    .child(div().text_size(px(11.0)).text_color(t.dim).overflow_hidden().whitespace_nowrap().text_ellipsis().child(folder))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(self.btn(t, Spec::new("br-open-worktree", "Open that worktree", ButtonKind::Primary).off(false).tip(None), cx, move |v, w, cx| v.open_folder(&f, w, cx)))
                            .child(self.btn(t, Spec::new("br-stay", "Stay here", ButtonKind::Secondary).off(false).tip(None), cx, |v, _, cx| {
                                v.worktree = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        let n = self.note.as_ref()?;
        let profile = n.needs_profile && (self.retry.is_some() || links::links(cx).open_identity.is_some());
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .child(note_view(t, n))
                .when(profile, |el| {
                    el.child(div().pt(px(6.0)).flex().gap(px(6.0)).child(self.btn(t, Spec::new("br-pick-profile", "Pick a profile", ButtonKind::Primary).off(false).tip(Some("Choose the name and email for commits in this repo".into())), cx, |v, w, cx| v.pick_profile(w, cx))))
                })
                .into_any_element(),
        )
    }
}

impl Render for BranchesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let busy = self.busy.is_some();
        let shown = self.visible();
        let status = self.status_view(&t, cx);
        let mut list = div().id("branches-list").flex_1().min_h(px(0.0)).overflow_y_scroll().flex().flex_col().gap(px(1.0));
        if self.list.is_none() {
            list = list.child(busy_line(&t, "br-loading", "Loading branches…").p(px(6.0)));
        } else {
            if let Some(e) = &self.error {
                list = list.child(error_text(&t, e.clone()));
            }
            if shown.local.is_empty() && shown.remote_only.is_empty() && self.error.is_none() {
                let text = if self.repo.is_none() {
                    "Open this from a repo."
                } else if !self.filter.trim().is_empty() {
                    "No branch matches."
                } else {
                    "No branches yet. Make the first commit."
                };
                list = list.child(div().p(px(6.0)).child(hint(&t, text)));
            }
            if !shown.local.is_empty() {
                list = list.child(section(&t, "Branches"));
            }
            for (i, b) in shown.local.iter().take(LOCAL_LIMIT).enumerate() {
                list = list.child(self.local_row(&t, b, i, cx));
            }
            if shown.local.len() > LOCAL_LIMIT {
                list = list.child(div().px(px(6.0)).py(px(4.0)).child(hint(&t, format!("{} more. Type to filter.", shown.local.len() - LOCAL_LIMIT))));
            }
            if !shown.remote_only.is_empty() {
                list = list.child(section(&t, "Only on the remote"));
            }
            for (i, b) in shown.remote_only.iter().take(REMOTE_LIMIT).enumerate() {
                list = list.child(self.remote_row(&t, b, i, cx));
            }
            if shown.remote_only.len() > REMOTE_LIMIT {
                list = list.child(div().px(px(6.0)).py(px(4.0)).child(hint(&t, format!("{} more. Type to filter.", shown.remote_only.len() - REMOTE_LIMIT))));
            }
        }
        let create_off = busy || self.repo.is_none() || self.new_name.trim().is_empty() || self.problem.is_some();
        let create = self.btn(&t, Spec::new("br-new-create", "Create", ButtonKind::Primary).off(create_off).tip(Some("Make the branch and switch to it".into())), cx, |v, w, cx| v.create(w, cx));
        div()
            .id("branches-view")
            .track_focus(&self.focus)
            .size_full()
            .min_h(px(0.0))
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.5))
            .on_key_down(cx.listener(Self::on_key))
            .when(self.repo.is_some(), |el| el.child(div().flex_none().child(self.filter_input.clone())))
            .child(list)
            .when_some(status, |el, s| el.child(s))
            .when(self.repo.is_some(), |el| el.child(
                div()
                    .id("br-new")
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .pt(px(6.0))
                    .border_t_1()
                    .border_color(t.border)
                    .child(text_line("New branch from HEAD", t.muted, 11.0))
                    .child(div().flex().gap(px(6.0)).items_center().child(div().flex_1().min_w(px(0.0)).child(self.fresh_input.clone())).child(create))
                    .when_some(self.problem, |el, p| el.child(text_line(p, t.red, 11.0))),
            ))
    }
}

pub fn open(repo: Option<PathBuf>, cx: &mut App) {
    cx.defer(move |cx| super::open_window("Branches", WIDTH, HEIGHT, cx, move |window, cx| cx.new(|cx| BranchesView::new(repo, window, cx))));
}
