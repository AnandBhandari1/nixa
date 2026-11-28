use super::links;
use super::stash_ops::{self, DiffKind, DiffRow, Stash, StashFile};
use super::widgets::{Spec, spec_button, busy_line, check, hint, note_view, selector, small_button, text_line, with_tip};
use super::words::{Outcome, ago, failure};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ListHorizontalSizingBehavior, SharedString, Subscription, Task,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use nixa_ui::button::ButtonKind;
use nixa_ui::text::MONO_FONT;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::path::PathBuf;
use std::time::Duration;

pub const WIDTH: f32 = 720.0;
pub const HEIGHT: f32 = 560.0;
pub const POLL: Duration = Duration::from_secs(3);
pub const FILE_LIMIT: usize = 300;
const DIFF_ROW: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    Push,
    Apply,
    Pop,
    Drop,
}

pub struct StashView {
    pub repo: Option<PathBuf>,
    pub stashes: Option<Vec<Stash>>,
    pub selected: Option<(String, String)>,
    pub files: Vec<StashFile>,
    pub file: Option<String>,
    pub rows: Vec<DiffRow>,
    pub width: f32,
    pub busy: Option<Job>,
    pub note: Option<Outcome>,
    pub confirm_drop: Option<String>,
    pub untracked: bool,
    pub keep_index: bool,
    pub restore_index: bool,
    pub loading_files: bool,
    pub closed: bool,
    pub message: Entity<TextInput>,
    listing: bool,
    again: bool,
    token: u64,
    diff_token: u64,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _sub: Subscription,
    _poll: Task<()>,
}

impl Focusable for StashView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn title_of(s: &Stash) -> String {
    if s.wip {
        format!("Saved on {}", s.branch.clone().unwrap_or_else(|| "a detached commit".into()))
    } else if s.message.is_empty() {
        "No name".into()
    } else {
        s.message.clone()
    }
}

pub fn meta_of(s: &Stash) -> String {
    let mut parts = vec![s.branch.clone().unwrap_or_else(|| "no branch".into()), ago(s.at)];
    if s.untracked {
        parts.push("with new files".into());
    }
    parts.join(" · ")
}

pub fn width_of(rows: &[DiffRow]) -> f32 {
    rows.iter().fold(300.0_f32, |w, r| w.max((r.text.chars().count() as f32 * 7.0 + 60.0).min(4000.0)))
}

impl StashView {
    pub fn new(repo: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| TextInput::new(cx, "What are these changes? (optional)"));
        let sub = cx.subscribe_in(&message, window, |v: &mut StashView, _, e: &InputEvent, window, cx| match e {
            InputEvent::Submit(_) => v.push(window, cx),
            InputEvent::Cancel => v.close(window),
            InputEvent::Changed(_) => {}
        });
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |v, cx| {
                    if v.busy.is_none() {
                        v.load(true, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut view = StashView {
            repo,
            stashes: None,
            selected: None,
            files: Vec::new(),
            file: None,
            rows: Vec::new(),
            width: 300.0,
            busy: None,
            note: None,
            confirm_drop: None,
            untracked: true,
            keep_index: false,
            restore_index: false,
            loading_files: false,
            closed: false,
            message,
            listing: false,
            again: false,
            token: 0,
            diff_token: 0,
            scroll: UniformListScrollHandle::new(),
            focus,
            _sub: sub,
            _poll: poll,
        };
        view.load(false, cx);
        view
    }

    fn set_rows(&mut self, rows: Vec<DiffRow>) {
        self.width = width_of(&rows);
        self.rows = rows;
    }

    pub fn close(&mut self, window: &mut Window) {
        self.closed = true;
        window.remove_window();
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        self.diff_token += 1;
        let token = self.diff_token;
        let (Some(root), Some((reference, _)), Some(file)) = (
            self.repo.clone(),
            self.selected.clone(),
            self.file.as_ref().and_then(|p| self.files.iter().find(|f| &f.path == p)).cloned(),
        ) else {
            self.set_rows(Vec::new());
            return;
        };
        let job = cx.background_executor().spawn(async move { stash_ops::diff(&root, &reference, &file.path, Some(file.untracked)) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |v, cx| {
                if v.diff_token != token {
                    return;
                }
                match out {
                    Ok(text) => v.set_rows(stash_ops::diff_rows(&text)),
                    Err(e) => v.set_rows(vec![DiffRow { kind: DiffKind::Other, number: None, text: format!("Cannot show this diff: {e}") }]),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn pick_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.file = Some(path.to_string());
        self.set_rows(Vec::new());
        self.load_diff(cx);
        cx.notify();
    }

    fn load_files(&mut self, cx: &mut Context<Self>) {
        self.token += 1;
        let token = self.token;
        self.files.clear();
        self.file = None;
        self.set_rows(Vec::new());
        let (Some(root), Some((reference, _))) = (self.repo.clone(), self.selected.clone()) else { return };
        self.loading_files = true;
        let job = cx.background_executor().spawn(async move { stash_ops::files(&root, &reference) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |v, cx| {
                if v.token != token {
                    return;
                }
                v.loading_files = false;
                match out {
                    Ok(files) => {
                        v.file = files.first().map(|f| f.path.clone());
                        v.files = files;
                        v.load_diff(cx);
                    }
                    Err(e) => {
                        let mut o = failure(&e);
                        o.title = "Cannot read this stash".into();
                        o.hint = String::new();
                        o.detail = e;
                        v.note = Some(o);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn pick(&mut self, s: &Stash, cx: &mut Context<Self>) {
        if self.selected.as_ref().is_some_and(|(_, sha)| sha == &s.sha) {
            return;
        }
        self.selected = Some((s.reference.clone(), s.sha.clone()));
        self.confirm_drop = None;
        self.load_files(cx);
        cx.notify();
    }

    pub fn load(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(root) = self.repo.clone() else {
            self.stashes = Some(Vec::new());
            return;
        };
        if self.listing {
            self.again = true;
            return;
        }
        self.listing = true;
        let job = cx.background_executor().spawn(async move { stash_ops::list(&root) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |v, cx| {
                v.listing = false;
                match out {
                    Ok(list) => {
                        let same = if keep { v.selected.as_ref().and_then(|(_, sha)| list.iter().find(|s| &s.sha == sha)).cloned() } else { None };
                        let first = list.first().cloned();
                        v.stashes = Some(list);
                        match (same, first) {
                            (Some(s), _) => v.selected = Some((s.reference, s.sha)),
                            (None, Some(f)) => {
                                v.selected = None;
                                v.confirm_drop = None;
                                v.pick(&f, cx);
                            }
                            (None, None) => {
                                v.selected = None;
                                v.confirm_drop = None;
                                v.files.clear();
                                v.file = None;
                                v.set_rows(Vec::new());
                            }
                        }
                    }
                    Err(e) => {
                        v.stashes = Some(Vec::new());
                        let mut o = failure(&e);
                        o.title = "Cannot list stashes".into();
                        o.hint = String::new();
                        o.detail = e;
                        v.note = Some(o);
                    }
                }
                if v.again {
                    v.again = false;
                    v.load(true, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn work(&mut self, job: Job, run: impl FnOnce(PathBuf) -> Outcome + Send + 'static, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(root) = self.repo.clone() else { return };
        self.busy = Some(job);
        self.note = None;
        let repo = root.clone();
        let task = cx.background_executor().spawn(async move { run(root) });
        cx.spawn(async move |this, cx| {
            let out = task.await;
            this.update(cx, |v, cx| {
                v.busy = None;
                v.confirm_drop = None;
                if out.ok {
                    if job == Job::Push {
                        v.message.update(cx, |m, cx| m.set_value("", cx));
                    }
                    if job != Job::Apply {
                        v.selected = None;
                    }
                }
                if out.ok || out.kind == "conflict" {
                    links::changed(&repo, cx);
                }
                v.note = Some(out);
                v.load(true, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn push(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().to_string();
        let (untracked, keep) = (self.untracked, self.keep_index);
        self.work(Job::Push, move |root| stash_ops::push(&root, &message, untracked, keep), cx);
    }

    pub fn apply(&mut self, pop: bool, cx: &mut Context<Self>) {
        let Some((reference, sha)) = self.selected.clone() else { return };
        let index = self.restore_index;
        let job = if pop { Job::Pop } else { Job::Apply };
        self.work(job, move |root| stash_ops::apply(&root, &reference, pop, index, Some(&sha)), cx);
    }

    pub fn drop_selected(&mut self, cx: &mut Context<Self>) {
        let Some((reference, sha)) = self.selected.clone() else { return };
        self.work(Job::Drop, move |root| stash_ops::drop(&root, &reference, Some(&sha)), cx);
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key != "escape" {
            return;
        }
        cx.stop_propagation();
        if self.confirm_drop.is_some() {
            self.confirm_drop = None;
            cx.notify();
        } else {
            self.close(window);
        }
    }

    fn btn(&self, t: &Theme, s: Spec, cx: &mut Context<Self>, f: fn(&mut Self, &mut Window, &mut Context<Self>)) -> AnyElement {
        let off = s.off;
        spec_button(t, &s).when(!off, |el| el.on_click(cx.listener(move |v, _, w, cx| f(v, w, cx)))).into_any_element()
    }

    fn stash_row(&self, t: &Theme, s: &Stash, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let on = self.selected.as_ref().is_some_and(|(_, sha)| sha == &s.sha);
        let hover = t.hover;
        let name = format!("stash-row-{i}");
        let st = s.clone();
        let el = div()
            .id(SharedString::from(name.clone()))
            .flex_none()
            .px(px(8.0))
            .py(px(5.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .when(on, |el| el.bg(t.selected))
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |v, _, _, cx| v.pick(&st, cx)))
            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).text_color(t.foreground).child(title_of(s)))
            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.0)).text_color(t.dim).child(meta_of(s)));
        selector(el, &name).into_any_element()
    }

    fn file_row(&self, t: &Theme, f: &StashFile, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let on = self.file.as_deref() == Some(f.path.as_str());
        let hover = t.hover;
        let name = format!("stash-file-{i}");
        let path = f.path.clone();
        let color = if f.status == 'D' { t.red } else if f.untracked { t.green } else { t.muted };
        let el = div()
            .id(SharedString::from(name.clone()))
            .flex_none()
            .h(px(22.0))
            .px(px(6.0))
            .flex()
            .gap(px(8.0))
            .items_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .when(on, |el| el.bg(t.selected))
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |v, _, _, cx| v.pick_file(&path, cx)))
            .child(div().w(px(70.0)).flex_none().text_size(px(11.0)).text_color(color).child(stash_ops::status_word(f.status, f.untracked)))
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.0)).text_color(t.foreground).child(f.path.clone()));
        selector(el, &name).into_any_element()
    }

    fn diff_row(t: &Theme, row: &DiffRow, width: f32) -> AnyElement {
        if row.kind == DiffKind::Hunk {
            return div().min_w(px(width)).w_full().px(px(10.0)).h(px(DIFF_ROW)).flex_none().whitespace_nowrap().bg(t.raised).text_color(t.dim).child(row.text.clone()).into_any_element();
        }
        let color = match row.kind {
            DiffKind::Del => t.red,
            DiffKind::Add => t.green,
            _ => t.foreground,
        };
        div()
            .min_w(px(width))
            .w_full()
            .h(px(DIFF_ROW))
            .flex_none()
            .flex()
            .gap(px(10.0))
            .whitespace_nowrap()
            .when(row.kind == DiffKind::Add, |el| el.bg(t.green.opacity(0.09)))
            .when(row.kind == DiffKind::Del, |el| el.bg(t.red.opacity(0.09)))
            .child(div().w(px(34.0)).flex_none().text_right().text_color(t.dim).child(row.number.map(|n| n.to_string()).unwrap_or_default()))
            .child(div().flex_1().text_color(color).child(row.text.clone()))
            .into_any_element()
    }

    fn actions(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let sha = self.selected.as_ref().map(|(_, s)| s.clone());
        if self.confirm_drop.is_some() && self.confirm_drop == sha {
            let label = if self.busy == Some(Job::Drop) { "Dropping…" } else { "Drop" };
            let el = div()
                .id("stash-drop-confirm")
                .flex_none()
                .flex()
                .gap(px(8.0))
                .items_center()
                .flex_wrap()
                .child(text_line("Drop this stash? This cannot be undone.", t.red, 12.0).w_auto())
                .child(self.btn(t, Spec::new("stash-drop-yes", label, ButtonKind::Danger).off(busy), cx, |v, _, cx| v.drop_selected(cx)))
                .child(self.btn(t, Spec::new("stash-drop-no", "Keep it", ButtonKind::Secondary).off(false), cx, |v, _, cx| {
                    v.confirm_drop = None;
                    cx.notify();
                }));
            return selector(el, "stash-drop-confirm").into_any_element();
        }
        let apply = if self.busy == Some(Job::Apply) { "Applying…" } else { "Apply" };
        let pop = if self.busy == Some(Job::Pop) { "Popping…" } else { "Pop" };
        div()
            .id("stash-actions")
            .flex_none()
            .flex()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(self.btn(t, Spec::new("stash-apply", apply, ButtonKind::Primary).off(busy).tip(Some("Put these changes back and keep the stash".into())), cx, |v, _, cx| v.apply(false, cx)))
            .child(self.btn(t, Spec::new("stash-pop", pop, ButtonKind::Secondary).off(busy).tip(Some("Put these changes back and remove the stash".into())), cx, |v, _, cx| v.apply(true, cx)))
            .child(self.btn(t, Spec::new("stash-drop", "Drop", ButtonKind::Danger).off(busy).tip(Some("Delete this stash (asks first)".into())), cx, |v, _, cx| {
                v.confirm_drop = v.selected.as_ref().map(|(_, s)| s.clone());
                cx.notify();
            }))
            .child(div().flex_1())
            .child(check(t, "stash-restore-index", self.restore_index, "Restore staged state").on_click(cx.listener(|v, _, _, cx| {
                v.restore_index = !v.restore_index;
                cx.notify();
            })))
            .into_any_element()
    }

    fn detail(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.selected.is_none() {
            let text = if self.stashes.as_ref().is_some_and(|s| !s.is_empty()) { "Pick a stash to see its files." } else { "Stashes you make show here." };
            return div().flex_1().min_w(px(0.0)).flex().flex_col().items_center().justify_center().child(hint(t, text).w_auto()).into_any_element();
        }
        let mut files = div().id("stash-files").flex_none().max_h(px(130.0)).overflow_y_scroll().p(px(3.0)).flex().flex_col().rounded(px(8.0)).border_1().border_color(t.border);
        if self.loading_files {
            files = files.child(busy_line(t, "stash-files-loading", "Loading files…").p(px(4.0)));
        } else if self.files.is_empty() {
            files = files.child(div().p(px(4.0)).child(hint(t, "This stash has no file changes.")));
        }
        for (i, f) in self.files.iter().take(FILE_LIMIT).enumerate() {
            files = files.child(self.file_row(t, f, i, cx));
        }
        let width = self.width;
        let list = uniform_list("stash-diff", self.rows.len(), cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
            let t = theme(cx).clone();
            this.rows[range].iter().map(|r| Self::diff_row(&t, r, width)).collect::<Vec<_>>()
        }))
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(self.scroll.clone())
        .size_full();
        let empty = if self.file.is_some() { "No text changes to show." } else { "Pick a file." };
        div()
            .id("stash-detail")
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.actions(t, cx))
            .child(files)
            .child(
                div()
                    .id("stash-diff-zone")
                    .flex_1()
                    .min_h(px(60.0))
                    .relative()
                    .overflow_hidden()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.surface)
                    .font_family(MONO_FONT)
                    .text_size(px(11.0))
                    .child(list)
                    .when(self.rows.is_empty(), |el| el.child(div().absolute().top(px(8.0)).px(px(10.0)).child(hint(t, empty)))),
            )
            .into_any_element()
    }
}

impl Render for StashView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let busy = self.busy.is_some();
        let no_repo = self.repo.is_none();
        let view = div()
            .id("stash-view")
            .track_focus(&self.focus)
            .size_full()
            .min_h(px(0.0))
            .px(px(14.0))
            .pb(px(14.0))
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.5))
            .on_key_down(cx.listener(Self::on_key));
        if no_repo {
            return view.child(div().pt(px(10.0)).child(hint(&t, "Open this from a repo to see its stashes.")));
        }
        let push_label = if self.busy == Some(Job::Push) { "Stashing…" } else { "Stash my changes" };
        let push = with_tip(
            small_button(&t, "stash-push", push_label, ButtonKind::Primary, busy)
                .h(px(30.0))
                .when(!busy, |el| el.on_click(cx.listener(|v, _, w, cx| v.push(w, cx)))),
            Some("Put your uncommitted changes away for later".into()),
        );
        let mut list = div().id("stash-list").w(px(250.0)).flex_none().min_h(px(0.0)).overflow_y_scroll().p(px(4.0)).flex().flex_col().gap(px(2.0)).rounded(px(8.0)).border_1().border_color(t.border);
        match &self.stashes {
            None => list = list.child(busy_line(&t, "stash-loading", "Loading stashes…").p(px(6.0))),
            Some(s) if s.is_empty() => list = list.child(div().p(px(6.0)).child(hint(&t, "No stashes yet."))),
            Some(_) => {}
        }
        let stashes = self.stashes.clone().unwrap_or_default();
        for (i, s) in stashes.iter().enumerate() {
            list = list.child(self.stash_row(&t, s, i, cx));
        }
        let detail = self.detail(&t, cx);
        view
            .child(div().id("stash-push-row").flex_none().pt(px(12.0)).flex().gap(px(8.0)).items_center().child(div().flex_1().min_w(px(0.0)).child(self.message.clone())).child(push))
            .child(
                div()
                    .id("stash-push-options")
                    .flex_none()
                    .pt(px(4.0))
                    .flex()
                    .gap(px(16.0))
                    .items_center()
                    .child(check(&t, "stash-untracked", self.untracked, "Include new files").on_click(cx.listener(|v, _, _, cx| {
                        v.untracked = !v.untracked;
                        cx.notify();
                    })))
                    .child(check(&t, "stash-keep-index", self.keep_index, "Keep staged changes too").on_click(cx.listener(|v, _, _, cx| {
                        v.keep_index = !v.keep_index;
                        cx.notify();
                    }))),
            )
            .when_some(self.note.as_ref(), |el, n| el.child(note_view(&t, n)))
            .child(div().id("stash-main").flex_1().min_h(px(0.0)).pt(px(12.0)).flex().gap(px(12.0)).child(list).child(detail))
    }
}

pub fn open(repo: Option<PathBuf>, cx: &mut App) {
    cx.defer(move |cx| super::open_window("Stashes", WIDTH, HEIGHT, cx, move |window, cx| cx.new(|cx| StashView::new(repo, window, cx))));
}
