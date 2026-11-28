use super::model::{self, ACTIONS, Action, Check, Commit, MAX_LISTED, Message, Step};
use super::plan::{self, Kind, Outcome, Plan, Preview};
use crate::actions::ClosePane;
use crate::screens::git::tools::host::{self, Changed, Host, Notice};
use crate::screens::git::tools::kit::{self, ChipLook, Look, Pal, col, row};
use crate::screens::git::tools::run::{self, Cancel};
use crate::screens::settings::parts::memory::text_area::{AreaEvent, TextArea};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollHandle,
    Subscription, Window, div, px,
};
use nixa_ui::text::MONO_FONT;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::theme;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Open {
    pub repo: Option<PathBuf>,
    pub base: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub sha: String,
    pub action: Action,
    pub new_message: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorKind {
    Reword,
    Squash,
}

pub struct Editor {
    pub summary: Entity<TextInput>,
    pub body: Entity<TextArea>,
}

pub struct RebaseView {
    host: Host,
    pub repo: Option<PathBuf>,
    pub base: Option<String>,
    pub plan: Option<Plan>,
    pub loading: bool,
    pub rows: Vec<Row>,
    pub token: u64,
    pub editors: HashMap<(String, EditorKind), Editor>,
    pub preview: Option<(Vec<Step>, Preview)>,
    pub checking: bool,
    pub confirm: bool,
    pub running: bool,
    cancel: Option<Cancel>,
    pub stopping: bool,
    pub result: Option<Outcome>,
    pub finished: Option<Outcome>,
    pub locked: bool,
    pub note: Option<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    subs: Vec<Subscription>,
}

impl RebaseView {
    pub fn new(open: Open, host: Host, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = RebaseView {
            host,
            repo: open.repo,
            base: open.base,
            plan: None,
            loading: false,
            rows: Vec::new(),
            token: 0,
            editors: HashMap::new(),
            preview: None,
            checking: false,
            confirm: false,
            running: false,
            cancel: None,
            stopping: false,
            result: None,
            finished: None,
            locked: false,
            note: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            subs: Vec::new(),
        };
        view.load(cx);
        view
    }

    pub fn reset(&mut self, open: Open, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.repo = open.repo;
        self.base = open.base;
        self.editors.clear();
        self.subs.clear();
        self.finished = None;
        self.load(cx);
    }

    pub fn commits(&self) -> &[Commit] {
        self.plan
            .as_ref()
            .map(|p| p.commits.as_slice())
            .unwrap_or(&[])
    }

    pub fn commit_of(&self, sha: &str) -> Option<&Commit> {
        self.commits().iter().find(|c| c.sha == sha)
    }

    fn own_message(&self, sha: &str) -> Message {
        self.commit_of(sha).map(Commit::own).unwrap_or_default()
    }

    fn make_editor(
        &mut self,
        sha: &str,
        kind: EditorKind,
        message: &Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let summary_text = message.summary.clone();
        let body_text = message.body.clone();
        let summary = cx.new(|cx| {
            let mut i = TextInput::new(cx, "Summary (first line)");
            i.set_value(summary_text, cx);
            i
        });
        let body = cx.new(|cx| {
            let mut a = TextArea::new(cx, "Description (optional)");
            a.set_value(body_text, cx);
            a
        });
        self.subs.push(cx.subscribe_in(
            &summary,
            window,
            |v, _, e: &InputEvent, window, cx| match e {
                InputEvent::Changed(_) => v.changed(cx),
                InputEvent::Cancel => v.escape(window, cx),
                InputEvent::Submit(_) => {}
            },
        ));
        self.subs.push(
            cx.subscribe_in(&body, window, |v, _, e: &AreaEvent, window, cx| match e {
                AreaEvent::Changed(_) => v.changed(cx),
                AreaEvent::Cancel => v.escape(window, cx),
                AreaEvent::Save => {}
            }),
        );
        self.editors
            .insert((sha.to_string(), kind), Editor { summary, body });
    }

    fn editor_value(&self, sha: &str, kind: EditorKind, cx: &App) -> Option<Message> {
        self.editors.get(&(sha.to_string(), kind)).map(|e| Message {
            summary: e.summary.read(cx).value().to_string(),
            body: e.body.read(cx).value().to_string(),
        })
    }

    fn set_editor(&self, sha: &str, kind: EditorKind, message: &Message, cx: &mut App) {
        if let Some(e) = self.editors.get(&(sha.to_string(), kind)) {
            let s = message.summary.clone();
            let b = message.body.clone();
            e.summary.update(cx, |i, cx| i.set_value(s, cx));
            e.body.update(cx, |a, cx| a.set_value(b, cx));
        }
    }

    pub fn steps(&self, cx: &App) -> Vec<Step> {
        self.rows
            .iter()
            .map(|r| {
                let message = match r.action {
                    Action::Reword => Some(
                        self.editor_value(&r.sha, EditorKind::Reword, cx)
                            .unwrap_or_else(|| self.own_message(&r.sha)),
                    ),
                    Action::Squash if r.new_message => Some(
                        self.editor_value(&r.sha, EditorKind::Squash, cx)
                            .unwrap_or_default(),
                    ),
                    _ => None,
                };
                Step {
                    sha: r.sha.clone(),
                    action: r.action,
                    message,
                }
            })
            .collect()
    }

    fn fresh_rows(&self) -> Vec<Row> {
        self.commits()
            .iter()
            .map(|c| Row {
                sha: c.sha.clone(),
                action: Action::Pick,
                new_message: false,
            })
            .collect()
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            cx.notify();
            return;
        };
        self.token += 1;
        let token = self.token;
        self.loading = true;
        self.preview = None;
        self.result = None;
        self.confirm = false;
        let base = self.base.clone();
        let job = cx
            .background_executor()
            .spawn(async move { plan::rebase_plan(&repo, base.as_deref()) });
        cx.spawn(async move |this, cx| {
            let plan = job.await;
            let _ = this.update(cx, |v, cx| {
                if v.token != token {
                    return;
                }
                v.plan = Some(plan);
                v.rows = v.fresh_rows();
                v.loading = false;
                v.locked = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn editable(&self) -> bool {
        self.plan.as_ref().is_some_and(Plan::ok)
            && !self.loading
            && !self.running
            && !self.locked
            && !self.checking
            && self.finished.is_none()
    }

    pub fn changed(&mut self, cx: &mut Context<Self>) {
        self.preview = None;
        self.confirm = false;
        if self.result.as_ref().is_some_and(|r| !r.note.ok) && !self.locked {
            self.result = None;
        }
        cx.notify();
    }

    pub fn set_action(
        &mut self,
        index: usize,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable() {
            return;
        }
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        if row.action == action {
            return;
        }
        if action == Action::Reword
            && !self
                .editors
                .contains_key(&(row.sha.clone(), EditorKind::Reword))
        {
            let own = self.own_message(&row.sha);
            self.make_editor(&row.sha, EditorKind::Reword, &own, window, cx);
        }
        let r = &mut self.rows[index];
        r.action = action;
        if action != Action::Squash {
            r.new_message = false;
        }
        self.changed(cx);
    }

    pub fn toggle_new_message(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable() {
            return;
        }
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        if row.action != Action::Squash {
            return;
        }
        let turn_on = !row.new_message;
        if turn_on {
            let joined = model::joined_message(&self.steps(cx), self.commits(), index);
            if !self
                .editors
                .contains_key(&(row.sha.clone(), EditorKind::Squash))
            {
                self.make_editor(&row.sha, EditorKind::Squash, &joined, window, cx);
            }
            self.set_editor(&row.sha, EditorKind::Squash, &joined, cx);
        }
        self.rows[index].new_message = turn_on;
        self.changed(cx);
    }

    pub fn move_row(&mut self, index: usize, up: bool, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        self.rows = model::move_step(&self.rows, index, up);
        self.changed(cx);
    }

    pub fn start_over(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        self.rows = self.fresh_rows();
        let keys: Vec<(String, EditorKind)> = self.editors.keys().cloned().collect();
        for (sha, kind) in keys {
            let own = self.own_message(&sha);
            self.set_editor(&sha, kind, &own, cx);
        }
        self.changed(cx);
    }

    pub fn run_check(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let steps = self.steps(cx);
        let head = self
            .plan
            .as_ref()
            .map(|p| p.head.clone())
            .unwrap_or_default();
        let base = self.base.clone();
        self.checking = true;
        self.preview = None;
        self.result = None;
        self.confirm = false;
        let job = cx.background_executor().spawn({
            let steps = steps.clone();
            async move { plan::preview(&repo, base.as_deref(), &steps, &head) }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |v, cx| {
                v.checking = false;
                v.preview = Some((steps, result));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let steps = self.steps(cx);
        let head = self
            .plan
            .as_ref()
            .map(|p| p.head.clone())
            .unwrap_or_default();
        let base = self.base.clone();
        self.confirm = false;
        self.running = true;
        self.stopping = false;
        self.result = None;
        let cancel = run::new_cancel();
        self.cancel = Some(cancel.clone());
        let job = cx.background_executor().spawn({
            let repo = repo.clone();
            async move { plan::run_rebase(&repo, base.as_deref(), &steps, &head, Some(&cancel)) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = job.await;
            let _ = this.update_in(cx, |v, window, cx| {
                v.running = false;
                v.cancel = None;
                v.finish(repo, outcome, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish(
        &mut self,
        repo: PathBuf,
        outcome: Outcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if outcome.note.ok {
            (self.host.changed)(&repo, Changed::History, cx);
            let body = if !outcome.note.hint.is_empty() {
                outcome.note.hint.clone()
            } else {
                outcome
                    .summary
                    .as_ref()
                    .map(|s| s.text.clone())
                    .unwrap_or_default()
            };
            let notice = Notice {
                title: outcome.note.title.clone(),
                body,
                warning: outcome.warning || outcome.needs_force,
            };
            if (self.host.notify)(&notice, cx) {
                window.remove_window();
                return;
            }
            self.finished = Some(outcome);
            cx.notify();
            return;
        }
        if outcome.locked() {
            self.locked = true;
            (self.host.changed)(&repo, Changed::History, cx);
        }
        self.result = Some(outcome);
        cx.notify();
    }

    pub fn stop_run(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
            self.stopping = true;
        }
        cx.notify();
    }

    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm {
            self.confirm = false;
            cx.notify();
            return;
        }
        if self.running {
            return;
        }
        window.remove_window();
    }

    fn pick_profile(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if !(self.host.pick_identity)(&repo, cx) {
            self.note = Some(host::NO_PROFILES.into());
        }
    }

    pub fn check(&self, steps: &[Step]) -> Check {
        match &self.plan {
            Some(p) if p.ok() && !p.commits.is_empty() => model::validate(steps, Some(&p.commits)),
            _ => Check::default(),
        }
    }

    fn header(&self, p: &Pal) -> Vec<AnyElement> {
        let Some(_) = &self.repo else {
            return vec![
                div()
                    .pt(px(10.0))
                    .child(kit::hint(p, "Open this from a repo."))
                    .into_any_element(),
            ];
        };
        let plan = match &self.plan {
            Some(plan) if !(self.loading && plan.commits.is_empty()) => plan,
            _ => {
                return vec![
                    row()
                        .pt(px(12.0))
                        .gap(px(8.0))
                        .child(kit::spinner(p, "rebase-spin"))
                        .child(kit::hint(p, "Reading the commits…"))
                        .into_any_element(),
                ];
            }
        };
        let mut out: Vec<AnyElement> = Vec::new();
        if !plan.base.is_empty() {
            let place = match &plan.upstream {
                Some(up) => format!("Commits not yet on {up}"),
                None => format!("Commits after {} {}", plan.base_short, plan.base_subject),
            };
            let branch = plan
                .branch
                .as_ref()
                .map(|b| format!(", on {b}"))
                .unwrap_or_default();
            out.push(
                div()
                    .pt(px(6.0))
                    .text_size(px(12.5))
                    .text_color(p.label)
                    .truncate()
                    .child(format!("{place}{branch}"))
                    .into_any_element(),
            );
        }
        if plan.ok() && !plan.commits.is_empty() {
            out.push(
                div()
                    .pt(px(2.0))
                    .child(kit::hint(
                        p,
                        "Oldest at the top. Git applies them from top to bottom.",
                    ))
                    .into_any_element(),
            );
        }
        let warn = |text: String, color| {
            div()
                .pt(px(4.0))
                .text_size(px(12.0))
                .text_color(color)
                .whitespace_normal()
                .child(text)
                .into_any_element()
        };
        if plan.detached && !plan.base.is_empty() {
            out.push(warn(
                "You are not on a branch. The changed commits stay on a detached HEAD.".into(),
                p.warn,
            ));
        }
        if plan.pushed > 0 {
            let one = plan.pushed == 1;
            out.push(warn(
                format!(
                    "{} of these commits {} already pushed. If you change {}, you will need a force push.",
                    plan.pushed,
                    if one { "is" } else { "are" },
                    if one { "it" } else { "them" }
                ),
                p.warn,
            ));
        }
        if plan.dirty && plan.ok() {
            let staged = if plan.staged {
                " Staged changes come back as unstaged."
            } else {
                ""
            };
            out.push(warn(
                format!(
                    "You have local changes. Your changes are put away and brought back.{staged}"
                ),
                if plan.staged { p.warn } else { p.hint },
            ));
        }
        if !plan.ok() {
            out.push(kit::error_text(p, plan.note.text()).into_any_element());
            out.push(detail_view(&plan.note.files, &plan.note.detail, p));
        }
        out
    }

    fn action_hint(&self, steps: &[Step], index: usize) -> Option<String> {
        let step = &steps[index];
        match step.action {
            Action::Drop => return Some("This commit and its changes are removed.".into()),
            Action::Squash | Action::Fixup => {}
            _ => return None,
        }
        let actions: Vec<Action> = steps.iter().map(|s| s.action).collect();
        let name = model::join_target(&actions, index)
            .and_then(|t| self.commit_of(&steps[t].sha))
            .map(|c| format!("{} above", c.short))
            .unwrap_or_else(|| "the commit above".into());
        let new_message = self.rows.get(index).is_some_and(|r| r.new_message);
        Some(if step.action == Action::Fixup {
            format!("Joins into {name}. This message is dropped.")
        } else if new_message {
            format!("Joins into {name}. The joined commit gets the message below.")
        } else {
            format!("Joins into {name}. Both messages are kept.")
        })
    }

    fn editor_view(&self, sha: &str, kind: EditorKind, p: &Pal) -> Option<AnyElement> {
        let e = self.editors.get(&(sha.to_string(), kind))?;
        let id = format!(
            "rebase-{}-{}",
            if kind == EditorKind::Reword {
                "reword"
            } else {
                "squash"
            },
            sha
        );
        Some(
            col()
                .pt(px(4.0))
                .pl(px(58.0))
                .gap(px(6.0))
                .child(kit::field_box(
                    p,
                    &format!("{id}-summary"),
                    e.summary.clone(),
                ))
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("{id}-body")))
                        .h(px(70.0))
                        .child(e.body.clone()),
                )
                .into_any_element(),
        )
    }

    fn step_row(
        &self,
        index: usize,
        steps: &[Step],
        check: &Check,
        p: &Pal,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step = &steps[index];
        let Some(c) = self.commit_of(&step.sha).cloned() else {
            return div().into_any_element();
        };
        let free = self.editable();
        let dropped = step.action == Action::Drop;
        let joined = step.action.joins();
        let mine: Vec<String> = check
            .problems
            .iter()
            .filter(|pr| pr.index == Some(index))
            .map(|pr| pr.text.clone())
            .collect();
        let tip = self.action_hint(steps, index);
        let count = steps.len();
        let mut chips: Vec<AnyElement> = ACTIONS
            .iter()
            .map(|&a| {
                let blocked = index == 0 && a.joins();
                let (text, tip) = a.chip();
                let on = step.action == a;
                kit::chip(
                    p,
                    &format!("rebase-act-{}-{}", c.sha, a.word()),
                    text,
                    ChipLook {
                        active: on,
                        color: on.then_some(if a == Action::Drop { p.bad } else { p.accent }),
                        disabled: !free || blocked,
                    },
                    if blocked {
                        "The top commit has nothing above it to join into"
                    } else {
                        tip
                    },
                    cx,
                    move |v, window, cx| v.set_action(index, a, window, cx),
                )
                .into_any_element()
            })
            .collect();
        let new_message = self.rows.get(index).is_some_and(|r| r.new_message);
        if step.action == Action::Squash {
            chips.push(
                kit::chip(
                    p,
                    &format!("rebase-newmsg-{}", c.sha),
                    if new_message {
                        "Keep both messages"
                    } else {
                        "New message…"
                    },
                    ChipLook {
                        disabled: !free,
                        ..ChipLook::default()
                    },
                    if new_message {
                        "Go back to both messages joined"
                    } else {
                        "Write one new message for the joined commit"
                    },
                    cx,
                    move |v, window, cx| v.toggle_new_message(index, window, cx),
                )
                .into_any_element(),
            );
        }
        let reword = (step.action == Action::Reword)
            .then(|| self.editor_view(&c.sha, EditorKind::Reword, p))
            .flatten();
        let squash = (step.action == Action::Squash && new_message)
            .then(|| self.editor_view(&c.sha, EditorKind::Squash, p))
            .flatten();
        col()
            .id(gpui::SharedString::from(format!("rebase-row-{}", c.sha)))
            .flex_none()
            .px(px(8.0))
            .py(px(6.0))
            .gap(px(3.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if mine.is_empty() { p.border } else { p.bad })
            .bg(p.card)
            .child(
                row()
                    .gap(px(6.0))
                    .child(kit::chip(
                        p,
                        &format!("rebase-up-{}", c.sha),
                        "↑",
                        ChipLook {
                            disabled: !free || index == 0,
                            ..ChipLook::default()
                        },
                        "Move up (earlier)",
                        cx,
                        move |v, _, cx| v.move_row(index, true, cx),
                    ))
                    .child(kit::chip(
                        p,
                        &format!("rebase-down-{}", c.sha),
                        "↓",
                        ChipLook {
                            disabled: !free || index + 1 >= count,
                            ..ChipLook::default()
                        },
                        "Move down (later)",
                        cx,
                        move |v, _, cx| v.move_row(index, false, cx),
                    ))
                    .when(joined, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(px(12.0))
                                .text_color(p.hint)
                                .child("↳"),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .font_family(MONO_FONT)
                            .text_size(px(11.5))
                            .text_color(if dropped { p.hint } else { p.accent })
                            .child(c.short.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(if dropped { p.hint } else { p.text })
                            .when(dropped, |el| el.line_through())
                            .child(if c.subject.is_empty() {
                                "(no message)".to_string()
                            } else {
                                c.subject.clone()
                            }),
                    )
                    .when(c.pushed, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(px(10.5))
                                .text_color(p.warn)
                                .child("pushed"),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(140.0))
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(p.hint)
                            .child(c.author.clone()),
                    ),
            )
            .child(row().pl(px(58.0)).gap(px(4.0)).flex_wrap().children(chips))
            .when_some(tip, |el, tip| {
                el.child(
                    div()
                        .pl(px(58.0))
                        .text_size(px(11.0))
                        .text_color(if dropped { p.bad } else { p.hint })
                        .whitespace_normal()
                        .child(tip),
                )
            })
            .children(reword)
            .children(squash)
            .children(mine.into_iter().map(|text| {
                div()
                    .pl(px(58.0))
                    .text_size(px(11.5))
                    .text_color(p.bad)
                    .whitespace_normal()
                    .child(text)
            }))
            .into_any_element()
    }

    fn list_view(
        &self,
        steps: &[Step],
        check: &Check,
        p: &Pal,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(plan) = self.plan.as_ref().filter(|p| p.ok()) else {
            return div().flex_1().into_any_element();
        };
        if plan.commits.is_empty() {
            return col()
                .flex_1()
                .items_center()
                .justify_center()
                .child(kit::hint(
                    p,
                    "There are no commits after this one. Nothing to change.",
                ))
                .into_any_element();
        }
        let rows: Vec<AnyElement> = (0..steps.len())
            .map(|i| self.step_row(i, steps, check, p, cx))
            .collect();
        col()
            .id("rebase-list")
            .flex_1()
            .min_h(px(0.0))
            .mt(px(10.0))
            .p(px(4.0))
            .gap(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(p.border)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(rows)
            .into_any_element()
    }

    fn status_view(&self, steps: &[Step], check: &Check, p: &Pal) -> AnyElement {
        let Some(plan) = self
            .plan
            .as_ref()
            .filter(|p| p.ok() && !p.commits.is_empty())
        else {
            return div().into_any_element();
        };
        let sum = model::summarize(steps, &plan.commits);
        let mut out = col()
            .id("rebase-status")
            .flex_none()
            .pt(px(8.0))
            .gap(px(2.0))
            .child(div().text_size(px(12.0)).text_color(p.label).child(format!(
                "{} commits: {}",
                plan.commits.len(),
                sum.text
            )));
        for pr in check.problems.iter().filter(|pr| pr.index.is_none()) {
            out = out.child(
                div()
                    .text_size(px(12.0))
                    .text_color(p.bad)
                    .whitespace_normal()
                    .child(pr.text.clone()),
            );
        }
        if check.problems.iter().any(|pr| pr.index.is_some()) {
            out = out.child(
                div()
                    .text_size(px(12.0))
                    .text_color(p.bad)
                    .child("Fix the marked commits first."),
            );
        }
        if self.checking {
            out = out.child(
                row()
                    .gap(px(6.0))
                    .child(kit::spinner(p, "rebase-check-spin"))
                    .child(kit::hint(p, "Checking the new order…")),
            );
        } else if let Some((key, pre)) = &self.preview
            && key.as_slice() == steps
        {
            let text = pre.note.text();
            out = if !pre.note.ok {
                out.child(kit::error_text(p, text))
            } else if pre.clean && pre.empty.is_empty() {
                out.child(kit::ok_text(p, text))
            } else {
                out.child(
                    div()
                        .pt(px(8.0))
                        .text_size(px(12.0))
                        .text_color(p.warn)
                        .whitespace_normal()
                        .child(text),
                )
            };
            if !pre.note.detail.is_empty() {
                out = out.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(p.hint)
                        .whitespace_normal()
                        .child(pre.note.detail.clone()),
                );
            }
        }
        out.into_any_element()
    }

    fn result_view(&self, p: &Pal, cx: &mut Context<Self>) -> AnyElement {
        if let Some(done) = &self.finished {
            return col()
                .id("rebase-done")
                .flex_none()
                .pt(px(6.0))
                .child(kit::ok_text(p, done.note.text()))
                .into_any_element();
        }
        let Some(r) = &self.result else {
            return div().into_any_element();
        };
        let mut out = col()
            .id("rebase-result")
            .flex_none()
            .pt(px(6.0))
            .gap(px(2.0));
        if r.note.kind == Some(Kind::Conflict) {
            out = out.child(
                div()
                    .pt(px(6.0))
                    .text_size(px(12.5))
                    .text_color(p.warn)
                    .whitespace_normal()
                    .child(
                        "Stopped on a conflict. Fix it in the Changes panel, then press Continue.",
                    ),
            );
        } else {
            out = out.child(kit::error_text(p, r.note.text()));
        }
        out = out.child(detail_view(&r.note.files, &r.note.detail, p));
        if r.note.kind == Some(Kind::Identity) {
            out = out.child(row().pt(px(6.0)).child(kit::button(
                p,
                "rebase-profile",
                "Pick a profile",
                Look {
                    small: true,
                    ..Look::default()
                },
                "",
                cx,
                |v, _, cx| v.pick_profile(cx),
            )));
        }
        out.into_any_element()
    }

    fn confirm_text(&self, sum: &model::Summary) -> String {
        let Some(plan) = &self.plan else {
            return String::new();
        };
        let branch = plan
            .branch
            .as_ref()
            .map(|b| format!(" on {b}"))
            .unwrap_or_default();
        let mut parts = vec![format!("Change {} commits{branch}?", plan.commits.len())];
        if sum.dropped > 0 {
            parts.push(if sum.dropped == 1 {
                "1 commit and its changes are removed.".into()
            } else {
                format!("{} commits and their changes are removed.", sum.dropped)
            });
        }
        if sum.pushed_changed > 0 {
            parts.push(if sum.pushed_changed == 1 {
                "1 changed commit is already pushed, so you will need a force push after this.".into()
            } else {
                format!(
                    "{} changed commits are already pushed, so you will need a force push after this.",
                    sum.pushed_changed
                )
            });
        }
        parts.join(" ")
    }

    fn footer(&self, steps: &[Step], check: &Check, p: &Pal, cx: &mut Context<Self>) -> AnyElement {
        let bar = || row().flex_none().pt(px(14.0)).gap(px(8.0)).justify_end();
        if self.running {
            return bar()
                .child(
                    row()
                        .flex_1()
                        .gap(px(8.0))
                        .child(kit::spinner(p, "rebase-run-spin"))
                        .child(kit::hint(p, if self.stopping { "Stopping…" } else { "Changing commits…" })),
                )
                .child(kit::button(
                    p,
                    "rebase-stop",
                    "Stop",
                    Look { danger: true, disabled: self.stopping, ..Look::default() },
                    "Stops git now. If it stops part way, continue or cancel it in the Changes panel.",
                    cx,
                    |v, _, cx| v.stop_run(cx),
                ))
                .into_any_element();
        }
        if self.locked || self.finished.is_some() {
            return bar()
                .child(kit::button(
                    p,
                    "rebase-close",
                    "Close",
                    Look {
                        primary: true,
                        ..Look::default()
                    },
                    "",
                    cx,
                    |_, window, _| window.remove_window(),
                ))
                .into_any_element();
        }
        let commits = self.commits();
        let sum = model::summarize(steps, commits);
        if self.confirm {
            return bar()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(12.0))
                        .text_color(if sum.dropped > 0 { p.bad } else { p.text })
                        .whitespace_normal()
                        .child(self.confirm_text(&sum)),
                )
                .child(kit::button(
                    p,
                    "rebase-confirm-yes",
                    "Yes, do it",
                    Look {
                        primary: true,
                        danger: sum.dropped > 0,
                        ..Look::default()
                    },
                    "",
                    cx,
                    |v, window, cx| v.apply(window, cx),
                ))
                .child(kit::button(
                    p,
                    "rebase-confirm-no",
                    "No",
                    Look::default(),
                    "",
                    cx,
                    |v, _, cx| {
                        v.confirm = false;
                        cx.notify();
                    },
                ))
                .into_any_element();
        }
        let ready = self
            .plan
            .as_ref()
            .is_some_and(|p| p.ok() && !p.commits.is_empty())
            && !self.loading;
        let same = ready && model::is_unchanged(steps, commits);
        let blocked: Option<String> = if !ready {
            Some("There is nothing to change here".into())
        } else if !check.ok() {
            check.problem()
        } else if same {
            Some("Nothing is changed yet".into())
        } else {
            None
        };
        let free = self.editable();
        let off = !free || blocked.is_some();
        bar()
            .child(kit::button(
                p,
                "rebase-reload",
                "Load again",
                Look {
                    small: true,
                    disabled: self.loading || self.checking,
                    ..Look::default()
                },
                "Read the commits again",
                cx,
                |v, _, cx| v.load(cx),
            ))
            .child(kit::button(
                p,
                "rebase-reset",
                "Start over",
                Look {
                    small: true,
                    disabled: !free || same,
                    ..Look::default()
                },
                "Put every commit back as it was in this list",
                cx,
                |v, _, cx| v.start_over(cx),
            ))
            .child(div().flex_1())
            .child(kit::button(
                p,
                "rebase-check",
                "Check",
                Look {
                    disabled: off,
                    ..Look::default()
                },
                blocked.clone().unwrap_or_else(|| {
                    "See if the new order will have conflicts. Nothing is changed.".into()
                }),
                cx,
                |v, _, cx| v.run_check(cx),
            ))
            .child(kit::button(
                p,
                "rebase-apply",
                "Apply",
                Look {
                    primary: true,
                    disabled: off,
                    ..Look::default()
                },
                blocked.unwrap_or_else(|| "Change the commits (asks first)".into()),
                cx,
                |v, _, cx| {
                    v.confirm = true;
                    cx.notify();
                },
            ))
            .child(kit::button(
                p,
                "rebase-cancel",
                "Cancel",
                Look::default(),
                "",
                cx,
                |_, window, _| window.remove_window(),
            ))
            .into_any_element()
    }
}

fn detail_view(files: &[String], detail: &str, p: &Pal) -> AnyElement {
    if files.is_empty() {
        if detail.is_empty() {
            return div().into_any_element();
        }
        return div()
            .pt(px(2.0))
            .text_size(px(11.0))
            .text_color(p.hint)
            .whitespace_normal()
            .child(detail.to_string())
            .into_any_element();
    }
    let more = files.len().saturating_sub(MAX_LISTED);
    col()
        .pt(px(4.0))
        .pl(px(8.0))
        .gap(px(1.0))
        .children(files.iter().take(MAX_LISTED).map(|f| {
            div()
                .font_family(MONO_FONT)
                .text_size(px(11.5))
                .text_color(p.text)
                .truncate()
                .child(f.clone())
        }))
        .when(more > 0, |el| {
            el.child(
                div()
                    .text_size(px(11.0))
                    .text_color(p.hint)
                    .child(format!("and {more} more")),
            )
        })
        .into_any_element()
}

impl Focusable for RebaseView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for RebaseView {
    fn drop(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
    }
}

impl Render for RebaseView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = kit::pal(theme(cx));
        let steps = self.steps(cx);
        let check = self.check(&steps);
        let head = kit::head(&p, "rebase-view", "Change commits", cx, |v, window, cx| {
            v.escape(window, cx)
        });
        let header = self.header(&p);
        let list = self.list_view(&steps, &check, &p, cx);
        let status = self.status_view(&steps, &check, &p);
        let result = self.result_view(&p, cx);
        let footer = self.footer(&steps, &check, &p, cx);
        kit::root(&p, "rebase-view")
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &ClosePane, window, cx| v.escape(window, cx)))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    v.escape(window, cx);
                }
            }))
            .child(head)
            .children(header)
            .when_some(self.note.clone(), |el, n| {
                el.child(
                    div()
                        .pt(px(6.0))
                        .text_size(px(12.0))
                        .text_color(p.warn)
                        .whitespace_normal()
                        .child(n),
                )
            })
            .child(list)
            .child(status)
            .child(result)
            .child(footer)
    }
}
