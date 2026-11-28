use super::io;
use super::model::{self, Choice, Conflict, MAX_AI_CHARS, MAX_SIDE_LINES, Parsed, Status};
use crate::actions::ClosePane;
use crate::screens::git::tools::blame::model::base_name;
use crate::screens::git::tools::host::{self, Answer, Changed, ConflictAsk, Host};
use crate::screens::git::tools::kit::{self, ChipLook, Look, Pal, col, row};
use crate::screens::git::tools::run::{self, Cancel};
use crate::screens::git::tools::state::{self, Op, RepoState};
use crate::screens::settings::parts::memory::text_area::{AreaEvent, TextArea};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    ScrollHandle, SharedString, Subscription, Window, div, px,
};
use nixa_ui::text::MONO_FONT;
use nixa_ui::theme;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Open {
    pub repo: Option<PathBuf>,
    pub file: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Choice(usize, Choice),
    Rebuild,
    Close,
}

#[derive(Clone, Debug, Default)]
pub struct Ai {
    pub busy: bool,
    pub text: Option<String>,
    pub error: Option<String>,
    pub agent: Option<String>,
    pub cancel: Option<Cancel>,
}

pub struct Loaded {
    pub state: Option<RepoState>,
    pub text: Result<String, String>,
}

pub struct ConflictView {
    host: Host,
    pub root: Option<PathBuf>,
    pub file: Option<String>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub info: Option<String>,
    pub note: Option<String>,
    pub repo_state: Option<RepoState>,
    pub original: String,
    pub parsed: Parsed,
    pub conflicts: Vec<Conflict>,
    pub starts: Vec<(usize, usize)>,
    pub choices: Vec<Option<Choice>>,
    pub built: String,
    pub hand_edited: bool,
    pub pending: Option<Pending>,
    pub saving: bool,
    pub done: bool,
    pub ai: Ai,
    pub ai_run: u64,
    pub load_run: u64,
    pub result: Entity<TextArea>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

fn read_all(root: Option<PathBuf>, file: Option<String>) -> Loaded {
    let Some(root) = root else {
        return Loaded {
            state: None,
            text: Err("No repo was given to this window.".into()),
        };
    };
    let Some(file) = file else {
        return Loaded {
            state: None,
            text: Err("No file was given to this window.".into()),
        };
    };
    Loaded {
        state: state::read_state(&root).ok(),
        text: io::read_file(&root, &file),
    }
}

impl ConflictView {
    pub fn new(open: Open, host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let result = cx.new(|cx| TextArea::new(cx, "The result shows here.").mono());
        let subs = vec![cx.subscribe_in(
            &result,
            window,
            |view, area, event: &AreaEvent, window, cx| match event {
                AreaEvent::Changed(_) => {
                    let hand = area.read(cx).value() != view.built;
                    if hand != view.hand_edited {
                        view.hand_edited = hand;
                        cx.notify();
                    }
                }
                AreaEvent::Save => view.save(cx),
                AreaEvent::Cancel => view.escape(window, cx),
            },
        )];
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |view, cx| {
                if view.has_progress() {
                    view.pending = Some(Pending::Close);
                    cx.notify();
                    false
                } else {
                    true
                }
            })
            .unwrap_or(true)
        });
        let mut view = ConflictView {
            host,
            root: open.repo,
            file: open.file,
            loading: true,
            loaded: false,
            error: None,
            info: None,
            note: None,
            repo_state: None,
            original: String::new(),
            parsed: Parsed::default(),
            conflicts: Vec::new(),
            starts: Vec::new(),
            choices: Vec::new(),
            built: String::new(),
            hand_edited: false,
            pending: None,
            saving: false,
            done: false,
            ai: Ai::default(),
            ai_run: 0,
            load_run: 0,
            result,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subs: subs,
        };
        view.load(None, cx);
        view
    }

    pub fn reset(&mut self, open: Open, cx: &mut Context<Self>) {
        if self.has_progress() && open.file == self.file && open.repo == self.root {
            return;
        }
        self.stop_ai(cx);
        self.root = open.repo;
        self.file = open.file;
        self.done = false;
        self.info = None;
        self.load(None, cx);
    }

    pub fn crlf(&self) -> bool {
        self.original.contains("\r\n")
    }

    pub fn chosen_count(&self) -> usize {
        self.conflicts
            .iter()
            .zip(&self.choices)
            .filter(|(c, choice)| model::is_chosen(c, **choice))
            .count()
    }

    pub fn has_progress(&self) -> bool {
        !self.done && (self.hand_edited || self.choices.iter().any(Option::is_some))
    }

    pub fn save_text(&self, cx: &App) -> String {
        if !self.hand_edited {
            return model::resolve_text(&self.parsed, &self.choices);
        }
        let text = self.result.read(cx).value().to_string();
        if self.crlf() {
            model::to_crlf(&text)
        } else {
            text
        }
    }

    pub fn can_save(&self, cx: &App) -> bool {
        self.loaded
            && !self.loading
            && !self.saving
            && !self.done
            && !model::has_markers(&self.save_text(cx))
    }

    pub fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.built = model::for_area(&model::resolve_text(&self.parsed, &self.choices));
        self.hand_edited = false;
        let text = self.built.clone();
        self.result.update(cx, |a, cx| a.set_value(text, cx));
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.loaded = false;
        self.error = None;
        self.pending = None;
        self.original.clear();
        self.parsed = Parsed::default();
        self.conflicts.clear();
        self.starts.clear();
        self.choices.clear();
        self.rebuild(cx);
    }

    fn take(&mut self, loaded: Loaded, info: Option<String>, cx: &mut Context<Self>) {
        self.loading = false;
        if loaded.state.is_some() {
            self.repo_state = loaded.state;
        }
        match loaded.text {
            Ok(text) => {
                self.parsed = model::parse_conflicts(&text);
                self.conflicts = model::conflicts_of(&self.parsed);
                self.starts = model::side_starts(&self.parsed);
                self.choices = vec![None; self.conflicts.len()];
                self.original = text;
                self.rebuild(cx);
                self.loaded = true;
                self.info = info;
            }
            Err(e) => {
                self.error = Some(format!(
                    "Could not read {}: {e}",
                    self.file.clone().unwrap_or_else(|| "the file".into())
                ));
            }
        }
        cx.notify();
    }

    pub fn load(&mut self, info: Option<String>, cx: &mut Context<Self>) {
        self.load_run += 1;
        let ticket = self.load_run;
        self.loading = true;
        self.clear(cx);
        let root = self.root.clone();
        let file = self.file.clone();
        let job = cx
            .background_executor()
            .spawn(async move { read_all(root, file) });
        cx.spawn(async move |this, cx| {
            let loaded = job.await;
            let _ = this.update(cx, |view, cx| {
                if view.load_run == ticket {
                    view.take(loaded, info, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_choice(&mut self, index: usize, choice: Choice, cx: &mut Context<Self>) {
        model::toggle(&mut self.choices, index, choice);
        self.info = None;
        self.rebuild(cx);
    }

    pub fn pick(&mut self, index: usize, choice: Choice, cx: &mut Context<Self>) {
        if self.loading || self.saving || self.done {
            return;
        }
        if self.hand_edited {
            self.pending = Some(Pending::Choice(index, choice));
        } else {
            self.apply_choice(index, choice, cx);
        }
        cx.notify();
    }

    pub fn stop_ai(&mut self, cx: &mut Context<Self>) {
        self.ai_run += 1;
        if let Some(c) = self.ai.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        self.ai.busy = false;
        cx.notify();
    }

    pub fn explain(&mut self, cx: &mut Context<Self>) {
        if self.ai.busy || self.loading || self.conflicts.is_empty() {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        self.ai_run += 1;
        let ticket = self.ai_run;
        let cancel = run::new_cancel();
        self.ai = Ai {
            busy: true,
            cancel: Some(cancel.clone()),
            ..Ai::default()
        };
        let ask = ConflictAsk {
            root,
            file: self.file.clone().unwrap_or_default(),
            ours: model::cut(&model::join_sides(&self.conflicts, true), MAX_AI_CHARS),
            theirs: model::cut(&model::join_sides(&self.conflicts, false), MAX_AI_CHARS),
        };
        let explain = self.host.explain_conflict;
        let job = cx
            .background_executor()
            .spawn(async move { explain(&ask, &cancel) });
        cx.spawn(async move |this, cx| {
            let answer = job.await;
            let _ = this.update(cx, |view, cx| {
                if view.ai_run != ticket {
                    return;
                }
                view.ai = match answer {
                    Ok(Answer { text, agent }) => {
                        let text = text.trim().to_string();
                        Ai {
                            error: text
                                .is_empty()
                                .then(|| "The AI gave an empty answer.".to_string()),
                            text: (!text.is_empty()).then_some(text),
                            agent,
                            ..Ai::default()
                        }
                    }
                    Err(e) => Ai {
                        error: Some(format!("AI failed: {}", e.trim())),
                        ..Ai::default()
                    },
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        let (Some(root), Some(file)) = (self.root.clone(), self.file.clone()) else {
            return;
        };
        let text = self.save_text(cx);
        self.saving = true;
        self.error = None;
        self.info = None;
        self.pending = None;
        let job = cx.background_executor().spawn({
            let root = root.clone();
            let file = file.clone();
            async move {
                io::write_resolved(&root, &file, &text)?;
                Ok::<Option<RepoState>, String>(state::read_state(&root).ok())
            }
        });
        cx.spawn(async move |this, cx| {
            let saved = job.await;
            let _ = this.update(cx, |view, cx| {
                view.saving = false;
                match saved {
                    Err(e) => {
                        view.error = Some(format!("Could not save {}: {e}", base_name(&file)))
                    }
                    Ok(state) => {
                        (view.host.changed)(&root, Changed::Conflicts, cx);
                        if state.is_some() {
                            view.repo_state = state;
                        }
                        let rest: Vec<String> = view
                            .repo_state
                            .as_ref()
                            .map(|s| {
                                s.conflicts
                                    .iter()
                                    .filter(|p| **p != file)
                                    .cloned()
                                    .collect()
                            })
                            .unwrap_or_default();
                        if let Some(next) = rest.first().cloned() {
                            let info = format!(
                                "Saved {}. {} more {} to fix. Now showing {}.",
                                base_name(&file),
                                rest.len(),
                                model::files_word(rest.len()),
                                base_name(&next)
                            );
                            view.file = Some(next);
                            view.load(Some(info), cx);
                        } else {
                            view.done = true;
                            view.info = Some(format!(
                                "Saved {}. All conflicts are fixed. {}",
                                base_name(&file),
                                model::finish_hint(view.op())
                            ));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn op(&self) -> Option<Op> {
        self.repo_state.as_ref().and_then(|s| s.op)
    }

    fn open_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(root), Some(file)) = (self.root.clone(), self.file.clone()) else {
            return;
        };
        if !(self.host.open_file)(&root, &file, cx) {
            self.note = Some(host::NO_EDITOR.into());
        }
    }

    pub fn ask_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_progress() {
            window.remove_window();
            return;
        }
        self.pending = Some(Pending::Close);
        cx.notify();
    }

    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.take().is_some() {
            cx.notify();
            return;
        }
        if self.ai.busy {
            self.stop_ai(cx);
            return;
        }
        self.ask_close(window, cx);
    }

    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pending.take() {
            Some(Pending::Choice(i, c)) => self.apply_choice(i, c, cx),
            Some(Pending::Rebuild) => self.rebuild(cx),
            Some(Pending::Close) => {
                self.done = true;
                window.remove_window();
                return;
            }
            None => {}
        }
        cx.notify();
    }

    pub fn status(&self, cx: &App) -> String {
        model::result_status(&Status {
            loading: self.loading,
            done: self.done,
            conflicts: self.conflicts.len(),
            chosen: self.chosen_count(),
            original: &self.original,
            hand_edited: self.hand_edited,
            result: self.result.read(cx).value(),
        })
    }

    fn side(&self, c: &Conflict, index: usize, ours: bool, p: &Pal) -> AnyElement {
        let lines = if ours { &c.ours_lines } else { &c.theirs_lines };
        let start = self
            .starts
            .get(index)
            .map(|s| if ours { s.0 } else { s.1 })
            .unwrap_or(1);
        let gutter = ((start + lines.len()).to_string().len() as f32 * 7.0 + 10.0).max(28.0);
        let chosen = self.choices.get(index).copied().flatten();
        let used = chosen == Some(if ours { Choice::Ours } else { Choice::Theirs })
            || chosen == Some(Choice::Both);
        let label = if ours { &c.ours_label } else { &c.theirs_label };
        col()
            .flex_1()
            .min_w(px(0.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(if used { p.accent } else { p.border })
            .bg(p.card)
            .overflow_hidden()
            .child(
                row()
                    .px(px(8.0))
                    .py(px(4.0))
                    .gap(px(6.0))
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if ours { p.info } else { p.ok })
                            .child(if ours { "Current (HEAD)" } else { "Incoming" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(p.hint)
                            .child(label.clone()),
                    ),
            )
            .child(
                col()
                    .py(px(4.0))
                    .font_family(MONO_FONT)
                    .text_size(px(11.5))
                    .when(lines.is_empty(), |el| {
                        el.child(
                            div()
                                .px(px(8.0))
                                .italic()
                                .text_color(p.hint)
                                .child("(no lines here)"),
                        )
                    })
                    .children(
                        lines
                            .iter()
                            .take(MAX_SIDE_LINES)
                            .enumerate()
                            .map(|(li, line)| {
                                let text = line.strip_suffix('\r').unwrap_or(line);
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_start()
                                    .gap(px(8.0))
                                    .px(px(4.0))
                                    .child(
                                        div()
                                            .w(px(gutter))
                                            .flex_none()
                                            .flex()
                                            .justify_end()
                                            .text_color(p.hint)
                                            .child((start + li).to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .whitespace_normal()
                                            .text_color(p.text)
                                            .child(if text.is_empty() {
                                                " ".to_string()
                                            } else {
                                                text.to_string()
                                            }),
                                    )
                            }),
                    )
                    .when(lines.len() > MAX_SIDE_LINES, |el| {
                        el.child(
                            div()
                                .px(px(8.0))
                                .pt(px(2.0))
                                .text_color(p.hint)
                                .child(format!(
                                    "{} more lines. Open in editor to see them all.",
                                    lines.len() - MAX_SIDE_LINES
                                )),
                        )
                    }),
            )
            .into_any_element()
    }

    fn block(&self, c: &Conflict, index: usize, p: &Pal, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.choices.get(index).copied().flatten();
        let picked = model::is_chosen(c, chosen);
        let off = self.loading || self.saving || self.done;
        let buttons: Vec<AnyElement> = c
            .choices()
            .into_iter()
            .map(|choice| {
                kit::button(
                    p,
                    &format!("cf-use-{index}-{}", choice.id()),
                    choice.button(),
                    Look {
                        small: true,
                        primary: chosen == Some(choice),
                        disabled: off,
                        ..Look::default()
                    },
                    choice.tip(),
                    cx,
                    move |v, _, cx| v.pick(index, choice, cx),
                )
                .into_any_element()
            })
            .collect();
        col()
            .id(("cf-block", index))
            .flex_none()
            .mb(px(10.0))
            .p(px(8.0))
            .gap(px(6.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(if picked { p.border } else { p.warn })
            .bg(p.raised)
            .child(
                row()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.label)
                            .child(format!(
                                "Conflict {} of {}",
                                index + 1,
                                self.conflicts.len()
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(p.hint)
                            .child(format!("line {}", c.line)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.0))
                            .text_color(if picked { p.ok } else { p.warn })
                            .child(match chosen {
                                Some(choice) if picked => choice.words(),
                                _ => "Not chosen yet",
                            }),
                    )
                    .child(div().flex_1())
                    .children(buttons),
            )
            .child(
                row()
                    .items_start()
                    .gap(px(8.0))
                    .child(self.side(c, index, true, p))
                    .child(self.side(c, index, false, p)),
            )
            .into_any_element()
    }

    fn ask_row(&self, pending: Pending, p: &Pal, cx: &mut Context<Self>) -> AnyElement {
        let (text, yes, no) = if pending == Pending::Close {
            (
                "Leave without saving? Your picks and edits will be lost.",
                "Leave",
                "Stay",
            )
        } else {
            (
                "This replaces your hand edits in the result.",
                "Replace my edits",
                "Keep my edits",
            )
        };
        row()
            .id("cf-ask")
            .flex_none()
            .mt(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .gap(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(p.warn)
            .bg(p.raised)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .whitespace_normal()
                    .text_size(px(12.0))
                    .text_color(p.warn)
                    .child(text),
            )
            .child(kit::button(
                p,
                "cf-ask-no",
                no,
                Look {
                    small: true,
                    ..Look::default()
                },
                "",
                cx,
                |v, _, cx| {
                    v.pending = None;
                    cx.notify();
                },
            ))
            .child(kit::button(
                p,
                "cf-ask-yes",
                yes,
                Look {
                    small: true,
                    primary: true,
                    danger: true,
                    ..Look::default()
                },
                "",
                cx,
                |v, window, cx| v.confirm(window, cx),
            ))
            .into_any_element()
    }

    fn ai_box(&self, p: &Pal) -> Option<AnyElement> {
        let ai = &self.ai;
        if !ai.busy && ai.text.is_none() && ai.error.is_none() {
            return None;
        }
        Some(
            col()
                .id("cf-ai")
                .flex_none()
                .max_h(px(170.0))
                .mt(px(8.0))
                .p(px(8.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .overflow_y_scroll()
                .when(ai.busy, |el| {
                    el.child(
                        row()
                            .gap(px(8.0))
                            .child(kit::spinner(p, "cf-ai-spin"))
                            .child(kit::hint(p, "Asking AI to explain this conflict…")),
                    )
                })
                .when_some(ai.error.clone(), |el, e| el.child(kit::error_text(p, e)))
                .when_some(ai.text.clone(), |el, text| {
                    el.child(
                        div()
                            .pb(px(4.0))
                            .text_size(px(11.5))
                            .text_color(p.label)
                            .child(match &ai.agent {
                                Some(agent) => format!("AI explanation ({agent})"),
                                None => "AI explanation".into(),
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(p.text)
                            .whitespace_normal()
                            .child(text),
                    )
                })
                .into_any_element(),
        )
    }

    pub fn heading(&self) -> String {
        match &self.file {
            Some(f) => format!("Resolve conflict: {}", base_name(f)),
            None => "Resolve conflict".into(),
        }
    }
}

impl Focusable for ConflictView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for ConflictView {
    fn drop(&mut self) {
        if let Some(c) = self.ai.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
    }
}

impl Render for ConflictView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = kit::pal(theme(cx));
        let heading = self.heading();
        window.set_window_title(&heading);
        let op_text = model::op_title(self.repo_state.as_ref());
        let save_ok = self.can_save(cx);
        let save_text = self.save_text(cx);
        let save_tip = if self.done {
            "Already saved"
        } else if self.loading {
            "Still loading"
        } else if model::has_markers(&save_text) {
            "Conflict markers are still in the result. Pick a side for each conflict, or edit them out."
        } else {
            "Write the result to the file and mark it resolved"
        };
        let empty_note = (!self.loading && self.error.is_none() && self.conflicts.is_empty()).then(|| {
            if model::has_markers(&self.original) {
                "This file has conflict markers that do not form a full block. Fix them in the result below."
            } else {
                "No conflict blocks found in this file."
            }
        });
        let blocks: Vec<AnyElement> = self
            .conflicts
            .clone()
            .iter()
            .enumerate()
            .map(|(i, c)| self.block(c, i, &p, cx))
            .collect();
        let ai = self.ai_box(&p);
        let ask = self.pending.map(|pending| self.ask_row(pending, &p, cx));
        let status = self.status(cx);
        let head = kit::head(
            &p,
            "conflict-view",
            SharedString::from(heading),
            cx,
            |v, window, cx| v.escape(window, cx),
        );
        let ai_button = if self.ai.busy {
            kit::button(
                &p,
                "cf-ai-stop",
                "Stop",
                Look {
                    danger: true,
                    ..Look::default()
                },
                "Stop the AI",
                cx,
                |v, _, cx| v.stop_ai(cx),
            )
        } else {
            kit::button(
                &p,
                "cf-ai-explain",
                "Explain with AI",
                Look {
                    disabled: self.loading || self.conflicts.is_empty(),
                    ..Look::default()
                },
                "Ask the AI what each side does and how to join them",
                cx,
                |v, _, cx| v.explain(cx),
            )
        };
        let no_file = self.file.is_none() || self.root.is_none();
        kit::root(&p, "conflict-view")
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &ClosePane, window, cx| v.ask_close(window, cx)))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    v.escape(window, cx);
                }
            }))
            .child(head)
            .child(
                row()
                    .flex_none()
                    .pt(px(6.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(p.label)
                            .child(self.file.clone().unwrap_or_else(|| "No file".into())),
                    )
                    .when_some(op_text, |el, text| {
                        el.child(kit::tag(&p, "cf-op", text, p.accent, ""))
                    })
                    .when(self.loading, |el| el.child(kit::spinner(&p, "cf-spin"))),
            )
            .child(
                div()
                    .flex_none()
                    .pt(px(4.0))
                    .child(kit::hint(&p, model::side_hint(self.op()))),
            )
            .when_some(self.error.clone(), |el, e| el.child(kit::error_text(&p, e)))
            .when_some(self.info.clone(), |el, e| el.child(kit::ok_text(&p, e)))
            .when_some(self.note.clone(), |el, e| {
                el.child(
                    div()
                        .pt(px(6.0))
                        .text_size(px(12.0))
                        .text_color(p.warn)
                        .child(e),
                )
            })
            .child(
                col()
                    .id("cf-blocks")
                    .flex_1()
                    .min_h(px(80.0))
                    .mt(px(8.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when_some(empty_note, |el, text| el.child(kit::hint(&p, text)))
                    .children(blocks),
            )
            .child(
                row()
                    .flex_none()
                    .pt(px(8.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.label)
                            .child("Result (you can edit it)"),
                    )
                    .child(div().flex_1())
                    .when(self.hand_edited && !self.done, |el| {
                        el.child(kit::chip(
                            &p,
                            "cf-rebuild",
                            "Rebuild from picks",
                            ChipLook::default(),
                            "Throw away hand edits and build the result from the picks",
                            cx,
                            |v, _, cx| {
                                v.pending = Some(Pending::Rebuild);
                                cx.notify();
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .id("cf-result")
                    .flex_none()
                    .h(px(170.0))
                    .child(self.result.clone()),
            )
            .child(div().flex_none().pt(px(4.0)).child(kit::hint(&p, status)))
            .children(ai)
            .children(ask)
            .child(
                row()
                    .flex_none()
                    .pt(px(12.0))
                    .gap(px(8.0))
                    .child(ai_button)
                    .child(kit::button(
                        &p,
                        "cf-open-editor",
                        "Open in editor",
                        Look {
                            disabled: no_file,
                            ..Look::default()
                        },
                        "Open this file in a new tab",
                        cx,
                        |v, _, cx| v.open_editor(cx),
                    ))
                    .child(div().flex_1())
                    .child(kit::button(
                        &p,
                        "cf-close",
                        "Close",
                        Look::default(),
                        "",
                        cx,
                        |v, window, cx| v.ask_close(window, cx),
                    ))
                    .child(kit::button(
                        &p,
                        "cf-save",
                        if self.saving {
                            "Saving…"
                        } else {
                            "Save and mark resolved"
                        },
                        Look {
                            primary: true,
                            disabled: !save_ok,
                            ..Look::default()
                        },
                        save_tip,
                        cx,
                        |v, _, cx| v.save(cx),
                    )),
            )
    }
}
