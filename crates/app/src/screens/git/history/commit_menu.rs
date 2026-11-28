use super::commit_ops::{self, CommitInfo, Expect, ResetMode, ResetPreview};
use super::links::{self, Level};
use super::log::Commit;
use super::widgets::{Spec, spec_button, busy_line, check, error_text, hint, radio, say, selector, small_button, text_line, with_tip};
use super::words::{Outcome, ago, branch_name_problem, plural};
use crate::screens::settings::parts::memory::text_area::TextArea;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    SharedString, Subscription, Window, div, px,
};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const WIDTH: f32 = 320.0;
pub const HEIGHT: f32 = 500.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Props {
    pub repo: PathBuf,
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub form: Option<Form>,
}

impl Props {
    pub fn of(repo: &Path, c: &Commit) -> Props {
        Props {
            repo: repo.to_path_buf(),
            sha: c.sha.clone(),
            short: c.short.clone(),
            subject: c.subject.clone(),
            author: c.author.clone(),
            form: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Checkout,
    Branch,
    Tag,
    Pick,
    Revert,
    Reset,
    Undo,
    Reword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Copy,
    Explain,
    Review,
    Web,
    Open(Form),
    Rebase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: String,
    pub tip: Option<String>,
    pub act: Act,
}

pub const MODES: [(ResetMode, &str, &str, &str); 3] = [
    (ResetMode::Soft, "soft", "Keep the changes, staged", "The branch moves here. Changes from the commits it leaves stay staged."),
    (ResetMode::Mixed, "mixed", "Keep the changes, not staged", "The branch moves here. Those changes stay in your files, not staged."),
    (ResetMode::Hard, "hard", "Throw the changes away", "The branch moves here and your files match this commit. Uncommitted changes are lost."),
];

type CommitJob = Arc<dyn Fn(PathBuf) -> Outcome + Send + Sync>;

pub struct CommitMenu {
    pub props: Props,
    pub info: Option<CommitInfo>,
    pub error: Option<String>,
    pub form: Option<Form>,
    pub busy: Option<String>,
    retry: Option<(String, CommitJob)>,
    pub note: Option<Outcome>,
    pub branch_name: String,
    pub branch_problem: Option<&'static str>,
    pub switch_to: bool,
    pub tag_name: String,
    pub tag_problem: Option<&'static str>,
    pub record_origin: bool,
    pub mode: ResetMode,
    pub preview: Option<Result<ResetPreview, Box<Outcome>>>,
    pub hard_ask: bool,
    pub summary: String,
    pub closed: bool,
    pub branch_input: Entity<TextInput>,
    pub tag_input: Entity<TextInput>,
    pub tag_message: Entity<TextArea>,
    pub summary_input: Entity<TextInput>,
    pub body_area: Entity<TextArea>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl Focusable for CommitMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn first(tips: Vec<Option<String>>) -> Option<String> {
    tips.into_iter().flatten().next()
}

pub fn items(info: Option<&CommitInfo>, error: bool, has_sha: bool) -> Vec<Vec<Item>> {
    let wait = || Some(if error { "The commit could not be read.".to_string() } else { "Loading…".to_string() });
    let no_sha = (!has_sha).then(|| "No commit was given.".to_string());
    let item = |id, icon, label: String, tip: Option<String>, act| Item { id, icon, label, tip, act };
    let place = info
        .map(|i| i.branch.clone().unwrap_or_else(|| if i.detached { "this detached commit".into() } else { "the current branch".into() }))
        .unwrap_or_else(|| "the current branch".into());
    let head_name = info.and_then(|i| i.branch.clone()).unwrap_or_else(|| "HEAD".into());
    let op_tip = info.and_then(|i| i.op).map(|op| format!("Finish the {} first.", op.lower()));
    let gate = |f: &dyn Fn(&CommitInfo) -> Vec<Option<String>>| match info {
        Some(i) => first(f(i)),
        None => wait(),
    };
    let change_tip = |i: &CommitInfo| first(vec![op_tip.clone(), (!i.on_head).then(|| format!("Only commits on {place} can be changed."))]);
    let web_name = info.and_then(|i| i.web.as_ref()).map(|w| w.name.clone()).unwrap_or_else(|| "GitHub".into());
    vec![
        vec![
            item("copy", "⧉", "Copy commit id".into(), no_sha.clone(), Act::Copy),
            item("explain", "✦", "Explain with AI".into(), no_sha.clone(), Act::Explain),
            item("review", "✦", "Review with AI".into(), no_sha.clone(), Act::Review),
            item(
                "web",
                "↗",
                format!("Open on {web_name}"),
                gate(&|i| vec![
                    i.web.is_none().then(|| "This repo has no remote with a web page.".to_string()),
                    (!i.pushed).then(|| "This commit is not on the remote yet.".to_string()),
                ]),
                Act::Web,
            ),
        ],
        vec![
            item(
                "checkout",
                "◉",
                "Check out this commit".into(),
                gate(&|i| vec![op_tip.clone(), (i.detached && i.is_head).then(|| "You are already on this commit.".to_string())]),
                Act::Open(Form::Checkout),
            ),
            item("branch", "⑂", "New branch here…".into(), gate(&|_| vec![]), Act::Open(Form::Branch)),
            item("tag", "⌗", "New tag here…".into(), gate(&|_| vec![]), Act::Open(Form::Tag)),
        ],
        vec![
            item(
                "pick",
                "⇢",
                format!("Cherry-pick onto {place}"),
                gate(&|i| vec![
                    op_tip.clone(),
                    i.head.is_none().then(|| "This branch has no commits yet.".to_string()),
                    i.on_head.then(|| format!("{place} already has this commit.")),
                ]),
                Act::Open(Form::Pick),
            ),
            item(
                "revert",
                "↶",
                "Revert this commit".into(),
                gate(&|i| vec![op_tip.clone(), (!i.on_head).then(|| format!("This commit is not on {place}."))]),
                Act::Open(Form::Revert),
            ),
        ],
        vec![
            item(
                "reset",
                "⇤",
                format!("Reset {head_name} to here…"),
                gate(&|i| vec![op_tip.clone(), i.is_head.then(|| format!("{place} is already here."))]),
                Act::Open(Form::Reset),
            ),
            item(
                "undo",
                "↺",
                "Undo this commit".into(),
                gate(&|i| vec![
                    op_tip.clone(),
                    (!i.is_head).then(|| "Only the newest commit can be undone. Use Reset for older ones.".to_string()),
                    i.is_root.then(|| "This is the first commit. There is nothing before it.".to_string()),
                    i.is_merge.then(|| "This is a merge. Use Reset instead.".to_string()),
                    i.pushed.then(|| "It is already on the remote. Use Revert instead.".to_string()),
                ]),
                Act::Open(Form::Undo),
            ),
            item(
                "reword",
                "✎",
                "Edit message…".into(),
                gate(&|i| vec![
                    change_tip(i),
                    (!i.is_head && i.is_merge).then(|| "This is a merge. Only the newest commit's message can be changed then.".to_string()),
                    (!i.is_head && i.merges_after > 0).then(|| "A merge comes after this commit, so its message cannot be changed here.".to_string()),
                ]),
                Act::Open(Form::Reword),
            ),
            item(
                "rebase",
                "≡",
                "Change commits after this one…".into(),
                gate(&|i| vec![
                    change_tip(i),
                    i.is_head.then(|| "There are no commits after this one.".to_string()),
                    (i.merges_after > 0).then(|| "A merge comes after this commit. Merges cannot be changed here.".to_string()),
                ]),
                Act::Rebase,
            ),
        ],
    ]
}

fn value(input: &Entity<TextInput>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

impl CommitMenu {
    pub fn new(props: Props, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let branch_input = cx.new(|cx| TextInput::new(cx, "feature/my-branch"));
        let tag_input = cx.new(|cx| TextInput::new(cx, "v1.0.0"));
        let tag_message = cx.new(|cx| TextArea::new(cx, "Tag message (optional)"));
        let summary_input = cx.new(|cx| TextInput::new(cx, "Summary (first line)"));
        let body_area = cx.new(|cx| TextArea::new(cx, "Description (optional)"));
        let subs = vec![
            cx.subscribe_in(&branch_input, window, |v: &mut CommitMenu, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(text) => {
                    v.branch_name = text.clone();
                    v.branch_problem = if text.trim().is_empty() { None } else { branch_name_problem(text.trim()) };
                    cx.notify();
                }
                InputEvent::Submit(_) => v.do_branch(window, cx),
                InputEvent::Cancel => v.back(window, cx),
            }),
            cx.subscribe_in(&tag_input, window, |v: &mut CommitMenu, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(text) => {
                    v.tag_name = text.clone();
                    v.tag_problem = if text.trim().is_empty() { None } else { branch_name_problem(text.trim()) };
                    cx.notify();
                }
                InputEvent::Submit(_) => v.do_tag(window, cx),
                InputEvent::Cancel => v.back(window, cx),
            }),
            cx.subscribe_in(&summary_input, window, |v: &mut CommitMenu, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(text) => {
                    v.summary = text.clone();
                    cx.notify();
                }
                InputEvent::Submit(_) => v.do_reword(window, cx),
                InputEvent::Cancel => v.back(window, cx),
            }),
        ];
        let form = props.form;
        let mut menu = CommitMenu {
            props,
            info: None,
            error: None,
            form: None,
            busy: None,
            retry: None,
            note: None,
            branch_name: String::new(),
            branch_problem: None,
            switch_to: true,
            tag_name: String::new(),
            tag_problem: None,
            record_origin: false,
            mode: ResetMode::Mixed,
            preview: None,
            hard_ask: false,
            summary: String::new(),
            closed: false,
            branch_input,
            tag_input,
            tag_message,
            summary_input,
            body_area,
            focus: cx.focus_handle(),
            _subs: subs,
        };
        menu.load_info(window, cx);
        if let Some(f) = form {
            menu.open_form(f, window, cx);
        }
        menu
    }

    fn load_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.props.sha.is_empty() {
            self.error = Some(if self.props.subject.is_empty() {
                "No commit was given.".into()
            } else {
                self.props.subject.clone()
            });
            return;
        }
        let root = self.props.repo.clone();
        let sha = self.props.sha.clone();
        let job = cx.background_executor().spawn(async move { commit_ops::commit_info(&root, &sha) });
        cx.spawn_in(window, async move |this, cx| {
            let out = job.await;
            this.update_in(cx, |v, _window, cx| {
                match out {
                    Ok(info) => {
                        v.summary = info.subject.clone();
                        let (s, b) = (info.subject.clone(), info.body.clone());
                        v.summary_input.update(cx, |i, cx| i.set_value(s, cx));
                        v.body_area.update(cx, |a, cx| a.set_value(b, cx));
                        v.info = Some(info);
                    }
                    Err(e) => v.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn short(&self) -> String {
        self.info.as_ref().map(|i| i.short.clone()).unwrap_or_else(|| {
            if self.props.short.is_empty() { super::words::short_sha(&self.props.sha) } else { self.props.short.clone() }
        })
    }

    pub fn place(&self) -> String {
        match &self.info {
            Some(i) => i.branch.clone().unwrap_or_else(|| if i.detached { "this detached commit".into() } else { "the current branch".into() }),
            None => "the current branch".into(),
        }
    }

    pub fn head_name(&self) -> String {
        self.info.as_ref().and_then(|i| i.branch.clone()).unwrap_or_else(|| "HEAD".into())
    }

    pub fn items(&self) -> Vec<Vec<Item>> {
        items(self.info.as_ref(), self.error.is_some(), !self.props.sha.is_empty())
    }

    pub fn close(&mut self, window: &mut Window) {
        self.closed = true;
        window.remove_window();
    }

    fn work(
        &mut self,
        label: &str,
        job: impl Fn(PathBuf) -> Outcome + Send + Sync + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_job(label.to_string(), Arc::new(job), window, cx);
    }

    fn run_job(&mut self, label: String, job: CommitJob, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.props.sha.is_empty() {
            return;
        }
        self.busy = Some(label.clone());
        self.retry = None;
        self.note = None;
        let root = self.props.repo.clone();
        let work = job.clone();
        let task = cx.background_executor().spawn(async move { work(root) });
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            this.update_in(cx, |v, window, cx| {
                v.busy = None;
                if out.needs_profile || out.kind == "identity" {
                    v.retry = Some((label, job));
                }
                v.finished(out, window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn finished(&mut self, r: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.props.repo.clone();
        if r.ok {
            let level = if r.warning { Level::Warning } else { Level::Success };
            links::notify(&r.title, &r.hint, level, cx);
            links::changed(&repo, cx);
            self.note = Some(r);
            self.close(window);
            return;
        }
        if r.kind == "conflict" || r.kind == "stopped" {
            links::changed(&repo, cx);
        }
        let again = self.form == Some(Form::Reset) && (r.kind == "untracked" || r.kind == "headChanged");
        self.note = Some(r);
        if again {
            self.hard_ask = false;
            self.load_preview(window, cx);
        }
    }

    fn load_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview = None;
        let root = self.props.repo.clone();
        let sha = self.props.sha.clone();
        let job = cx.background_executor().spawn(async move { commit_ops::reset_preview(&root, &sha) });
        cx.spawn_in(window, async move |this, cx| {
            let out = job.await;
            this.update_in(cx, |v, _, cx| {
                v.preview = Some(out);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_form(&mut self, form: Form, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.form = Some(form);
        self.note = None;
        self.hard_ask = false;
        match form {
            Form::Branch => window.focus(&self.branch_input.focus_handle(cx)),
            Form::Tag => window.focus(&self.tag_input.focus_handle(cx)),
            Form::Reword => window.focus(&self.summary_input.focus_handle(cx)),
            Form::Reset => self.load_preview(window, cx),
            _ => {}
        }
        cx.notify();
    }

    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.form = None;
        self.note = None;
        self.hard_ask = false;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn run(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.props.repo.clone();
        let sha = self.info.as_ref().map(|i| i.sha.clone()).unwrap_or(self.props.sha.clone());
        match act {
            Act::Copy => {
                cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()));
                links::notify("Commit id copied", &sha, Level::Info, cx);
                self.close(window);
            }
            Act::Explain | Act::Review => {
                let kind = if act == Act::Explain { "explain" } else { "review" };
                match links::links(cx).open_ai {
                    Some(open) => {
                        open(kind, &repo, &sha, cx);
                        self.close(window);
                    }
                    None => self.note = Some(super::words::refusal("later", "The AI window is not ready yet", "It comes with the Git AI part.")),
                }
            }
            Act::Web => {
                if let Some(url) = self.info.as_ref().and_then(|i| i.web.as_ref()).map(|w| w.url.clone()) {
                    cx.open_url(&url);
                    self.close(window);
                }
            }
            Act::Rebase => {
                let branch = self.info.as_ref().and_then(|i| i.branch.clone());
                match links::links(cx).open_rebase {
                    Some(open) => {
                        open(&repo, &sha, branch.as_deref(), cx);
                        self.close(window);
                    }
                    None => self.note = Some(super::words::refusal("later", "The change commits window is not ready yet", "It comes with the Git tools part.")),
                }
            }
            Act::Open(form) => self.open_form(form, window, cx),
        }
        cx.notify();
    }

    pub fn do_checkout(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let sha = self.props.sha.clone();
        self.work("Checking out…", move |root| commit_ops::checkout_commit(&root, &sha, force), window, cx);
    }

    pub fn do_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.branch_name.trim().to_string();
        if let Some(p) = branch_name_problem(&name) {
            self.branch_problem = Some(p);
            cx.notify();
            return;
        }
        let (sha, switch) = (self.props.sha.clone(), self.switch_to);
        self.work(&format!("Making {name}…"), move |root| commit_ops::create_branch_at(&root, &name, &sha, switch), window, cx);
    }

    pub fn do_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.tag_name.trim().to_string();
        if let Some(p) = branch_name_problem(&name) {
            self.tag_problem = Some(p);
            cx.notify();
            return;
        }
        let sha = self.props.sha.clone();
        let message = self.tag_message.read(cx).value().to_string();
        self.work(&format!("Making tag {name}…"), move |root| commit_ops::create_tag(&root, &name, &sha, &message), window, cx);
    }

    fn expect(&self) -> Option<Expect> {
        self.info.as_ref().map(CommitInfo::expect)
    }

    pub fn do_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(expect) = self.expect() else { return };
        let (sha, origin) = (self.props.sha.clone(), self.record_origin);
        self.work("Cherry-picking…", move |root| commit_ops::cherry_pick(&root, &sha, origin, 1, &expect), window, cx);
    }

    pub fn do_revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(expect) = self.expect() else { return };
        let sha = self.props.sha.clone();
        self.work("Reverting…", move |root| commit_ops::revert_commit(&root, &sha, 1, &expect), window, cx);
    }

    pub fn reset_ready(&self) -> bool {
        match &self.preview {
            Some(Ok(p)) => self.mode != ResetMode::Hard || p.untracked_lost.is_some(),
            _ => false,
        }
    }

    pub fn do_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Ok(p)) = &self.preview else { return };
        if !self.reset_ready() {
            return;
        }
        let expect = Expect { head: Some(p.head.clone()), branch: Some(p.branch.clone()) };
        let known = p.untracked_lost.clone().unwrap_or_default();
        let (sha, mode) = (self.props.sha.clone(), self.mode);
        self.work("Resetting…", move |root| commit_ops::reset_to(&root, &sha, mode, &expect, &known), window, cx);
    }

    pub fn do_undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(expect) = self.expect() else { return };
        self.work("Undoing…", move |root| commit_ops::undo_last_commit(&root, &expect), window, cx);
    }

    pub fn do_reword(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let summary = value(&self.summary_input, cx);
        let Some(expect) = self.expect() else { return };
        if summary.trim().is_empty() {
            return;
        }
        let body = self.body_area.read(cx).value().to_string();
        let sha = self.props.sha.clone();
        self.work("Saving the message…", move |root| commit_ops::reword_commit(&root, &sha, &summary, &body, &expect), window, cx);
    }

    fn header(&self, t: &Theme) -> Div {
        let i = self.info.as_ref();
        let author = i.map(|i| i.author.clone()).unwrap_or(self.props.author.clone());
        let meta = [author, i.map(|i| ago(i.at)).unwrap_or_default()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let subject = i.map(|i| i.subject.clone()).unwrap_or(self.props.subject.clone());
        let mut tags: Vec<(String, gpui::Hsla)> = Vec::new();
        if let Some(i) = i {
            tags.extend(i.refs.branches.iter().map(|b| (b.clone(), t.accent)));
            tags.extend(i.refs.tags.iter().map(|b| (b.clone(), t.magenta)));
            tags.push(if i.pushed { ("on the remote".into(), t.green) } else { ("not pushed yet".into(), t.yellow) });
            if i.is_merge {
                tags.push(("merge".into(), t.muted));
            }
            if i.is_head {
                tags.push(("newest".into(), t.muted));
            }
        }
        div()
            .flex_none()
            .px(px(8.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(div().flex_none().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.accent).child(self.short()))
                    .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).child(if subject.is_empty() { " ".to_string() } else { subject })),
            )
            .when(!meta.is_empty(), |el| el.child(hint(t, meta)))
            .when(!tags.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .children(tags.into_iter().take(8).map(|(text, color)| div().flex_none().text_size(px(10.5)).text_color(color).child(text))),
                )
            })
            .when(i.is_none() && self.error.is_none(), |el| el.child(busy_line(t, "cm-reading", "Reading the commit…")))
            .when_some(self.error.clone(), |el, e| el.child(error_text(t, e)))
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let mut col = div().id("cm-list").flex_1().min_h(px(0.0)).overflow_y_scroll().pt(px(4.0)).flex().flex_col().gap(px(1.0));
        for (g, group) in self.items().into_iter().enumerate() {
            if g > 0 {
                col = col.child(div().flex_none().mx(px(8.0)).my(px(3.0)).h(px(1.0)).bg(t.border));
            }
            for item in group {
                let off = item.tip.is_some() || busy;
                let hover = t.hover;
                let act = item.act;
                let name = format!("cm-{}", item.id);
                let row = div()
                    .id(SharedString::from(name.clone()))
                    .flex_none()
                    .h(px(28.0))
                    .px(px(8.0))
                    .flex()
                    .gap(px(9.0))
                    .items_center()
                    .rounded(px(6.0))
                    .when(!off, |el| {
                        el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |v, _, window, cx| v.run(act, window, cx)))
                    })
                    .child(div().w(px(16.0)).flex_none().text_size(px(12.0)).text_color(if off { t.dim } else { t.muted }).child(item.icon))
                    .child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_size(px(12.5)).text_color(if off { t.dim } else { t.foreground }).child(item.label));
                col = col.child(with_tip(selector(row, &name), item.tip));
            }
        }
        col.into_any_element()
    }

    fn buttons(children: Vec<AnyElement>) -> Div {
        div().flex_none().pt(px(10.0)).flex().gap(px(8.0)).items_center().flex_wrap().children(children)
    }

    fn btn(&self, t: &Theme, s: Spec, cx: &mut Context<Self>, f: fn(&mut Self, &mut Window, &mut Context<Self>)) -> AnyElement {
        let off = s.off || self.busy.is_some();
        spec_button(t, &s.off(off)).when(!off, |el| el.on_click(cx.listener(move |v, _, window, cx| f(v, window, cx)))).into_any_element()
    }

    fn back_btn(&self, t: &Theme, name: &str, label: &str, cx: &mut Context<Self>) -> AnyElement {
        self.btn(t, Spec::new(name, label.to_string(), ButtonKind::Secondary).off(false), cx, |v, w, cx| v.back(w, cx))
    }

    fn form_body(&self, form: Form, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let short = self.short();
        let place = self.place();
        let busy = self.busy.is_some();
        let has_info = self.info.is_some();
        match form {
            Form::Checkout => {
                let mut out = vec![
                    say(t, format!("Check out {short}?")).into_any_element(),
                    hint(t, "You will not be on any branch. Git calls this a detached HEAD. New commits made here are easy to lose, so make a branch if you want to keep work.").into_any_element(),
                ];
                if let Some(n) = self.note.as_ref().filter(|n| n.kind == "leaving") {
                    out.push(text_line(format!("{}. {}", n.title, n.hint), t.yellow, 12.0).pt(px(8.0)).into_any_element());
                    out.push(Self::buttons(vec![
                        self.btn(t, Spec::new("cm-checkout-force", "Check out anyway", ButtonKind::Danger).off(false), cx, |v, w, cx| v.do_checkout(true, w, cx)),
                        self.back_btn(t, "cm-checkout-no2", "No", cx),
                    ]).into_any_element());
                } else {
                    out.push(Self::buttons(vec![
                        self.btn(t, Spec::new("cm-checkout-yes", "Yes, do it", ButtonKind::Primary).off(false), cx, |v, w, cx| v.do_checkout(false, w, cx)),
                        self.back_btn(t, "cm-checkout-no", "No", cx),
                        self.btn(t, Spec::new("cm-checkout-branch", "New branch here instead", ButtonKind::Secondary).off(false), cx, |v, w, cx| v.open_form(Form::Branch, w, cx)),
                    ]).into_any_element());
                }
                out
            }
            Form::Branch => {
                let name_empty = self.branch_name.trim().is_empty();
                let mut out = vec![
                    say(t, format!("New branch at {short}")).into_any_element(),
                    div().pt(px(6.0)).child(self.branch_input.clone()).into_any_element(),
                ];
                if let Some(p) = self.branch_problem {
                    out.push(text_line(p, t.red, 11.0).pt(px(2.0)).into_any_element());
                }
                out.push(
                    div()
                        .pt(px(4.0))
                        .child(check(t, "cm-branch-switch", self.switch_to, "Switch to it").on_click(cx.listener(|v, _, _, cx| {
                            if v.busy.is_none() {
                                v.switch_to = !v.switch_to;
                                cx.notify();
                            }
                        })))
                        .into_any_element(),
                );
                out.push(Self::buttons(vec![
                    self.btn(t, Spec::new("cm-branch-create", "Create branch", ButtonKind::Primary).off(name_empty || self.branch_problem.is_some()), cx, |v, w, cx| v.do_branch(w, cx)),
                    self.back_btn(t, "cm-branch-back", "Back", cx),
                ]).into_any_element());
                out
            }
            Form::Tag => {
                let name_empty = self.tag_name.trim().is_empty();
                let mut out = vec![
                    say(t, format!("New tag at {short}")).into_any_element(),
                    div().pt(px(6.0)).child(self.tag_input.clone()).into_any_element(),
                ];
                if let Some(p) = self.tag_problem {
                    out.push(text_line(p, t.red, 11.0).pt(px(2.0)).into_any_element());
                }
                out.push(div().pt(px(6.0)).h(px(70.0)).flex().flex_col().child(self.tag_message.clone()).into_any_element());
                out.push(hint(t, "With a message it is an annotated tag. The tag stays on this computer until you push tags.").pt(px(4.0)).into_any_element());
                out.push(Self::buttons(vec![
                    self.btn(t, Spec::new("cm-tag-create", "Create tag", ButtonKind::Primary).off(name_empty || self.tag_problem.is_some()), cx, |v, w, cx| v.do_tag(w, cx)),
                    self.back_btn(t, "cm-tag-back", "Back", cx),
                ]).into_any_element());
                out
            }
            Form::Pick => {
                let mut out = vec![say(t, format!("Copy this commit onto {place} as a new commit.")).into_any_element()];
                if self.info.as_ref().is_some_and(|i| i.is_merge) {
                    out.push(hint(t, "This is a merge. The new commit gets what the merge brought into its first parent.").pt(px(4.0)).into_any_element());
                }
                out.push(
                    div()
                        .pt(px(6.0))
                        .child(check(t, "cm-pick-origin", self.record_origin, "Add a line that says where it came from").on_click(cx.listener(|v, _, _, cx| {
                            if v.busy.is_none() {
                                v.record_origin = !v.record_origin;
                                cx.notify();
                            }
                        })))
                        .into_any_element(),
                );
                out.push(hint(t, format!("The message gets \"(cherry picked from commit {short}…)\" at the end.")).into_any_element());
                out.push(Self::buttons(vec![
                    self.btn(t, Spec::new("cm-pick-run", "Cherry-pick", ButtonKind::Primary).off(!has_info), cx, |v, w, cx| v.do_pick(w, cx)),
                    self.back_btn(t, "cm-pick-back", "Back", cx),
                ]).into_any_element());
                out
            }
            Form::Revert => {
                let mut out = vec![
                    say(t, format!("Make a new commit on {place} that undoes {short}?")).into_any_element(),
                    hint(t, "The old commit stays in the history. This is the safe way to undo a commit that is already pushed.").into_any_element(),
                ];
                if self.info.as_ref().is_some_and(|i| i.is_merge) {
                    out.push(hint(t, "This is a merge. The new commit undoes everything it brought in.").pt(px(4.0)).into_any_element());
                }
                out.push(Self::buttons(vec![
                    self.btn(t, Spec::new("cm-revert-yes", "Yes, do it", ButtonKind::Primary).off(!has_info), cx, |v, w, cx| v.do_revert(w, cx)),
                    self.back_btn(t, "cm-revert-no", "No", cx),
                ]).into_any_element());
                out
            }
            Form::Reset => self.reset_form(t, cx),
            Form::Undo => vec![
                say(t, "Undo the last commit?").into_any_element(),
                hint(t, "Its changes go back to staged, so nothing is lost. You can change them and commit again.").into_any_element(),
                Self::buttons(vec![
                    self.btn(t, Spec::new("cm-undo-yes", "Yes, do it", ButtonKind::Primary).off(!has_info), cx, |v, w, cx| v.do_undo(w, cx)),
                    self.back_btn(t, "cm-undo-no", "No", cx),
                ])
                .into_any_element(),
            ],
            Form::Reword => {
                let i = self.info.as_ref();
                let mut out = vec![
                    say(t, format!("Edit the message of {short}")).into_any_element(),
                    div().pt(px(6.0)).child(self.summary_input.clone()).into_any_element(),
                    div().pt(px(6.0)).h(px(110.0)).flex().flex_col().child(self.body_area.clone()).into_any_element(),
                ];
                if i.is_some_and(|i| i.pushed) {
                    out.push(text_line("This commit is already on the remote. After this you need a force push.", t.yellow, 12.0).pt(px(6.0)).into_any_element());
                }
                if let Some(i) = i.filter(|i| !i.is_head) {
                    out.push(hint(t, format!("The {} after it get new ids too. Your local changes are kept.", plural(i.after, "commit"))).pt(px(4.0)).into_any_element());
                }
                out.push(Self::buttons(vec![
                    self.btn(t, Spec::new("cm-reword-save", "Save message", ButtonKind::Primary).off(!has_info || self.summary.trim().is_empty()), cx, |v, w, cx| v.do_reword(w, cx)),
                    self.back_btn(t, "cm-reword-back", "Back", cx),
                ]).into_any_element());
                let _ = busy;
                out
            }
        }
    }

    fn file_list(t: &Theme, files: &[String], count: usize) -> AnyElement {
        let mut shown: Vec<String> = files.iter().take(6).cloned().collect();
        if count > 6 {
            shown.push(format!("and {} more", count - 6));
        }
        div().pl(px(10.0)).flex().flex_col().children(shown.into_iter().map(|f| hint(t, f))).into_any_element()
    }

    pub fn preview_lines(&self, t: &Theme) -> Vec<AnyElement> {
        let Some(v) = &self.preview else {
            return vec![busy_line(t, "cm-reset-check", "Checking what would change…").pt(px(6.0)).into_any_element()];
        };
        let v = match v {
            Ok(v) => v,
            Err(o) => {
                let mut out = vec![error_text(t, super::widgets::note_text(o)).into_any_element()];
                if !o.detail.is_empty() {
                    out.push(hint(t, o.detail.clone()).into_any_element());
                }
                return out;
            }
        };
        let branch = v.branch.clone().unwrap_or_else(|| "HEAD".into());
        let mut out: Vec<AnyElement> = Vec::new();
        let line = |text: String, color: gpui::Hsla| text_line(text, color, 12.0).into_any_element();
        if v.leaving > 0 {
            out.push(line(format!("{} leave {branch}.", plural(v.leaving, "commit")), t.foreground));
        }
        if v.joining > 0 {
            out.push(line(format!("{} from another line join {branch}.", plural(v.joining, "commit")), t.foreground));
        }
        if v.leaving == 0 && v.joining == 0 {
            out.push(line(format!("{branch} is already at this commit."), t.foreground));
        }
        if v.pushed_leaving > 0 {
            let lead = if v.pushed_leaving == 1 { "1 of them is".to_string() } else { format!("{} of them are", v.pushed_leaving) };
            out.push(line(format!("{lead} on the remote. After this a normal push is refused, so you need a force push."), t.yellow));
        }
        if self.mode == ResetMode::Hard {
            if v.file_count > 0 {
                let text = if v.file_count == 1 { "1 changed file loses its changes:".to_string() } else { format!("{} changed files lose their changes:", v.file_count) };
                out.push(line(text, t.red));
                out.push(Self::file_list(t, &v.files, v.file_count));
            }
            match &v.untracked_lost {
                None => out.push(line("Cannot check the files that git does not track, so a hard reset is blocked.".into(), t.red)),
                Some(lost) if !lost.is_empty() => {
                    let text = if lost.len() == 1 { "This file that git does not track will be replaced:" } else { "These files that git does not track will be replaced:" };
                    out.push(line(text.into(), t.red));
                    out.push(Self::file_list(t, lost, lost.len()));
                }
                Some(_) if v.file_count == 0 => out.push(hint(t, "No changes in your files are lost.").into_any_element()),
                Some(_) => {}
            }
            if v.only_here > 0 {
                let (verb, it) = if v.only_here == 1 { ("is", "it") } else { ("are", "them") };
                out.push(line(format!("{} {verb} on no other branch. You can only get {it} back from the reflog.", plural(v.only_here, "commit")), t.red));
            }
        }
        out.into_iter().enumerate().map(|(k, el)| div().id(SharedString::from(format!("cm-reset-line-{k}"))).pt(px(4.0)).child(el).into_any_element()).collect()
    }

    fn reset_form(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let hard = self.mode == ResetMode::Hard;
        let ready = self.reset_ready();
        let mut out = vec![say(t, format!("Move {} to {}", self.head_name(), self.short())).into_any_element()];
        let radios = MODES.iter().map(|(mode, key, text, detail)| {
            let mode = *mode;
            radio(t, &format!("cm-reset-{key}"), self.mode == mode, text, detail)
                .on_click(cx.listener(move |v, _, _, cx| {
                    if v.busy.is_none() {
                        v.mode = mode;
                        v.hard_ask = false;
                        cx.notify();
                    }
                }))
                .into_any_element()
        });
        out.push(div().pt(px(6.0)).flex().flex_col().children(radios.collect::<Vec<_>>()).into_any_element());
        out.extend(self.preview_lines(t));
        if self.hard_ask {
            let lost = matches!(&self.preview, Some(Ok(p)) if p.untracked_lost.as_ref().is_some_and(|l| !l.is_empty()));
            let ask = if lost { "Throw away these changes and replace those files? This cannot be undone." } else { "Throw away these changes? This cannot be undone." };
            out.push(text_line(ask, t.red, 12.0).pt(px(8.0)).into_any_element());
            out.push(Self::buttons(vec![
                self.btn(t, Spec::new("cm-reset-hard-yes", "Yes, do it", ButtonKind::Danger).off(!ready), cx, |v, w, cx| v.do_reset(w, cx)),
                self.btn(t, Spec::new("cm-reset-hard-no", "No", ButtonKind::Secondary).off(false), cx, |v, _, cx| {
                    v.hard_ask = false;
                    cx.notify();
                }),
            ]).into_any_element());
        } else {
            let tip = match &self.preview {
                None => Some("Wait for the check to finish".to_string()),
                Some(Err(_)) => Some("The check failed".to_string()),
                Some(Ok(_)) if !ready => Some("Cannot check the files that git does not track".to_string()),
                _ => None,
            };
            let kind = if hard { ButtonKind::Danger } else { ButtonKind::Primary };
            let run = with_tip(
                small_button(t, "cm-reset-run", "Reset", kind, !ready || self.busy.is_some()).when(ready && self.busy.is_none(), |el| {
                    el.on_click(cx.listener(|v, _, window, cx| {
                        if v.mode == ResetMode::Hard {
                            v.hard_ask = true;
                            cx.notify();
                        } else {
                            v.do_reset(window, cx);
                        }
                    }))
                }),
                tip,
            )
            .into_any_element();
            out.push(Self::buttons(vec![run, self.back_btn(t, "cm-reset-back", "Back", cx)]).into_any_element());
        }
        out
    }

    fn pick_profile(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some((label, job)) = self.retry.clone() else { return };
        let handle = window.window_handle();
        let view = cx.weak_entity();
        crate::screens::git::accounts::identity_win::open(self.props.repo.clone(), move |cx| {
            handle.update(cx, |_, window, cx| {
                view.update(cx, |v, cx| {
                    if v.retry.as_ref().is_some_and(|(_, pending)| Arc::ptr_eq(pending, &job)) {
                        window.activate_window();
                        v.run_job(label, job, window, cx);
                    }
                }).ok();
            }).ok();
        }, cx);
    }

    fn note_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.note.as_ref()?;
        if n.kind == "leaving" {
            return None;
        }
        if n.kind == "identity" || n.needs_profile {
            let pick = self.retry.is_some();
            return Some(
                div()
                    .flex_none()
                    .pt(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(text_line(n.title.clone(), t.yellow, 12.0))
                    .child(hint(t, n.hint.clone()))
                    .when(pick, |el| {
                        el.child(
                            div().pt(px(2.0)).child(small_button(t, "cm-pick-profile", "Pick a profile", ButtonKind::Primary, false).on_click(cx.listener(|v, _, window, cx| {
                                v.pick_profile(window, cx);
                            }))),
                        )
                    })
                    .into_any_element(),
            );
        }
        if n.kind == "conflict" || n.kind == "stopped" {
            let text = if n.kind == "conflict" {
                "Stopped on a conflict. Fix it in the Changes panel, then press Continue.".to_string()
            } else {
                format!("{}. {}", n.title, n.hint).trim().to_string()
            };
            return Some(
                div()
                    .flex_none()
                    .pt(px(6.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(text_line(text, t.yellow, 12.0))
                    .when(!n.detail.is_empty(), |el| el.child(hint(t, n.detail.clone())))
                    .child(div().pt(px(2.0)).child(small_button(t, "cm-note-ok", "OK", ButtonKind::Primary, false).on_click(cx.listener(|v, _, window, _| v.close(window)))))
                    .into_any_element(),
            );
        }
        Some(super::widgets::note_view(t, n).into_any_element())
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key != "escape" {
            return;
        }
        if self.form.is_some() && self.busy.is_none() {
            self.back(window, cx);
        } else {
            self.close(window);
        }
        cx.stop_propagation();
    }
}

impl Render for CommitMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let body = match self.form {
            _ if self.props.sha.is_empty() => div().px(px(8.)).pt(px(6.))
                .child(error_text(&t, self.error.clone().unwrap_or_default())).into_any_element(),
            Some(form) => div()
                .id(SharedString::from(format!("cm-form-{form:?}")))
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .px(px(8.0))
                .pt(px(8.0))
                .flex()
                .flex_col()
                .children(self.form_body(form, &t, cx))
                .into_any_element(),
            None => self.list(&t, cx),
        };
        let note = self.note_view(&t, cx);
        div()
            .id("commit-menu")
            .track_focus(&self.focus)
            .size_full()
            .min_h(px(0.0))
            .p(px(6.0))
            .flex()
            .flex_col()
            .bg(t.popover)
            .text_color(t.foreground)
            .text_size(px(12.5))
            .on_key_down(cx.listener(Self::on_key))
            .when(!self.props.sha.is_empty(), |el| el.child(self.header(&t)))
            .child(body)
            .when_some(self.busy.clone(), |el, b| el.child(div().px(px(8.0)).pt(px(8.0)).child(busy_line(&t, "cm-busy", b))))
            .when_some(note, |el, n| el.child(div().flex_none().px(px(8.0)).pb(px(4.0)).child(n)))
    }
}

pub fn open(props: Props, cx: &mut App) {
    cx.defer(move |cx| super::open_window("Commit", WIDTH, HEIGHT, cx, move |window, cx| cx.new(|cx| CommitMenu::new(props, window, cx))));
}

pub fn open_empty(repo: Option<PathBuf>, request: &str, cx: &mut App) {
    let subject = match (&repo, request) {
        (None, _) => "Open this from a repo.",
        (Some(_), "git-commit") => "This repo has no commits yet.",
        _ => "This commit could not be found.",
    }.to_string();
    open(Props { repo: repo.unwrap_or_default(), subject, ..Props::default() }, cx);
}
