use super::load;
use super::model::{self, Band, Kind, Line, MAX_LINES, Parsed, Problem};
use crate::actions::ClosePane;
use crate::screens::git::tools::host::{self, Host};
use crate::screens::git::tools::kit::{self, ChipLook, Look, Pal, col, row};
use crate::screens::git::tools::run::{self, Cancel};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Div, Entity, FocusHandle, Focusable,
    FontWeight, KeyDownEvent, ListHorizontalSizingBehavior, ScrollStrategy, SharedString, Stateful,
    Subscription, UniformListScrollHandle, Window, div, px, uniform_list,
};
use nixa_ui::text::MONO_FONT;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::theme;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub const ROW: f32 = 20.0;
pub const ID_W: f32 = 62.0;
pub const AUTHOR_W: f32 = 128.0;
pub const AGE_W: f32 = 56.0;
pub const COPIED_FOR: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Open {
    pub repo: Option<PathBuf>,
    pub file: Option<String>,
    pub rev: String,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub file: String,
    pub rev: String,
    pub line: Option<u32>,
}

pub struct BlameView {
    host: Host,
    pub root: Option<PathBuf>,
    pub start_file: Option<String>,
    pub start_rev: String,
    pub file: Option<String>,
    pub rev: String,
    pub target: Option<u32>,
    pub loading: bool,
    pub background: bool,
    cancel: Option<Cancel>,
    pub run: u64,
    pub error: Option<Problem>,
    pub parsed: Parsed,
    pub texts: Vec<String>,
    pub bands: Vec<Band>,
    pub ages: HashMap<String, String>,
    pub commit_count: usize,
    pub width: f32,
    pub selected: Option<usize>,
    pub stack: Vec<Spot>,
    pub note: Option<String>,
    pub copied: Option<String>,
    pub find: Entity<TextInput>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _subs: Vec<Subscription>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn band_color(p: &Pal, band: usize) -> gpui::Hsla {
    [p.info, p.purple, p.cyan, p.ok, p.warn][band % model::BANDS]
}

impl BlameView {
    pub fn new(open: Open, host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| TextInput::new(cx, "Go to line, or find text"));
        let subs =
            vec![
                cx.subscribe_in(
                    &find,
                    window,
                    |view, _, event: &InputEvent, window, cx| match event {
                        InputEvent::Submit(_) => view.run_find(cx),
                        InputEvent::Cancel => window.remove_window(),
                        InputEvent::Changed(_) => {
                            if view.note.take().is_some() {
                                cx.notify();
                            }
                        }
                    },
                ),
            ];
        let mut view = BlameView {
            host,
            root: None,
            start_file: None,
            start_rev: String::new(),
            file: None,
            rev: String::new(),
            target: None,
            loading: false,
            background: false,
            cancel: None,
            run: 0,
            error: None,
            parsed: Parsed::default(),
            texts: Vec::new(),
            bands: Vec::new(),
            ages: HashMap::new(),
            commit_count: 0,
            width: 600.0,
            selected: None,
            stack: Vec::new(),
            note: None,
            copied: None,
            find,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            _subs: subs,
        };
        view.reset(open, cx);
        view
    }

    pub fn reset(&mut self, open: Open, cx: &mut Context<Self>) {
        self.root = open.repo;
        self.start_file = open.file.clone();
        self.start_rev = open.rev.clone();
        self.file = open.file;
        self.rev = open.rev;
        self.target = open.line.filter(|n| *n > 0);
        self.stack.clear();
        self.selected = None;
        self.load(false, cx);
    }

    fn scroll_to(&self, index: usize, strategy: ScrollStrategy) {
        self.scroll.scroll_to_item(index, strategy);
    }

    pub fn select(&mut self, index: usize, scroll: bool, cx: &mut Context<Self>) {
        if index >= self.parsed.lines.len() {
            return;
        }
        self.selected = Some(index);
        if scroll {
            self.scroll_to(index, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn take(&mut self, result: Result<Parsed, Problem>) {
        self.loading = false;
        self.background = false;
        self.cancel = None;
        self.selected = None;
        match result {
            Err(problem) => {
                self.error = Some(problem);
                self.parsed = Parsed::default();
                self.texts.clear();
                self.bands.clear();
                self.commit_count = 0;
            }
            Ok(parsed) => {
                self.error = None;
                self.texts = parsed
                    .lines
                    .iter()
                    .map(|l| model::expand_tabs(&l.text, 4))
                    .collect();
                self.bands = model::bands(&parsed.lines);
                self.commit_count = model::commit_count(&parsed);
                let now = now_secs();
                self.ages = parsed
                    .commits
                    .values()
                    .map(|c| {
                        let age = if c.uncommitted {
                            String::new()
                        } else {
                            model::age_text(c.at, now)
                        };
                        (c.sha.clone(), age)
                    })
                    .collect();
                let longest = self
                    .texts
                    .iter()
                    .map(|t| t.chars().count())
                    .max()
                    .unwrap_or(0) as f32;
                self.width = 3.0
                    + model::gutter_width(parsed.lines.len())
                    + ID_W
                    + AUTHOR_W
                    + AGE_W
                    + 5.0 * 8.0
                    + longest * model::CHAR
                    + 40.0;
                self.parsed = parsed;
                match self.target.take() {
                    Some(n) => {
                        if let Some(at) = model::find_line(&self.parsed.lines, &n.to_string(), None)
                        {
                            self.selected = Some(at);
                            self.scroll_to(at, ScrollStrategy::Center);
                        }
                    }
                    None => self.scroll_to(0, ScrollStrategy::Top),
                }
            }
        }
    }

    pub fn load(&mut self, background: bool, cx: &mut Context<Self>) {
        let (Some(root), Some(file)) = (self.root.clone(), self.file.clone()) else {
            let title = if self.root.is_none() {
                "No repo was given to this window."
            } else {
                "No file was given to this window."
            };
            self.error = Some(model::problem(Kind::Bad, title, ""));
            cx.notify();
            return;
        };
        self.run += 1;
        let ticket = self.run;
        if let Some(old) = self.cancel.take() {
            old.store(true, Ordering::SeqCst);
        }
        let cancel = run::new_cancel();
        self.cancel = Some(cancel.clone());
        self.loading = true;
        self.background = background;
        self.error = None;
        self.note = None;
        let rev = self.rev.clone();
        let job = cx
            .background_executor()
            .spawn(async move { load::blame_file(&root, &file, &rev, background, Some(&cancel)) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |view, cx| {
                if view.run != ticket {
                    return;
                }
                view.take(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        self.run += 1;
        self.loading = false;
        self.background = false;
        self.error = Some(model::problem(
            Kind::Cancelled,
            "Stopped.",
            "Press Reload to try again.",
        ));
        cx.notify();
    }

    fn go_to(&mut self, spot: Spot, cx: &mut Context<Self>) {
        self.file = Some(spot.file);
        self.rev = spot.rev;
        self.target = spot.line;
        self.selected = None;
        self.load(false, cx);
    }

    pub fn go_before(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(line) = self
            .selected
            .and_then(|i| self.parsed.lines.get(i))
            .cloned()
        else {
            return;
        };
        let Some(previous) = self
            .parsed
            .commits
            .get(&line.sha)
            .and_then(|c| c.previous.clone())
        else {
            return;
        };
        let Some(file) = self.file.clone() else {
            return;
        };
        self.stack.push(Spot {
            file: file.clone(),
            rev: self.rev.clone(),
            line: Some(line.n),
        });
        let next = if previous.file.is_empty() {
            file
        } else {
            previous.file
        };
        self.go_to(
            Spot {
                file: next,
                rev: previous.sha,
                line: Some(line.orig),
            },
            cx,
        );
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        if let Some(last) = self.stack.pop() {
            self.go_to(last, cx);
        }
    }

    pub fn go_now(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let line = self.stack.first().and_then(|s| s.line);
        self.stack.clear();
        let file = self
            .start_file
            .clone()
            .or_else(|| self.file.clone())
            .unwrap_or_default();
        self.go_to(
            Spot {
                file,
                rev: self.start_rev.clone(),
                line,
            },
            cx,
        );
    }

    pub fn run_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find.read(cx).value().trim().to_string();
        if query.is_empty() || self.parsed.lines.is_empty() {
            return;
        }
        match model::find_line(&self.parsed.lines, &query, self.selected) {
            None => self.note = Some("No line has that text.".into()),
            Some(at) => {
                self.note = model::line_number(&query)
                    .filter(|n| self.parsed.lines[at].n != *n)
                    .map(|_| {
                        format!(
                            "This file has {}.",
                            model::plural(self.parsed.lines.len(), "line", "lines")
                        )
                    });
                self.select(at, true, cx);
            }
        }
        cx.notify();
    }

    pub fn copy_id(&mut self, sha: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()));
        self.copied = Some(sha.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |view, cx| {
                if view.copied.as_deref() == Some(sha.as_str()) {
                    view.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn step(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let count = self.parsed.lines.len();
        if count == 0 {
            return false;
        }
        let next = match self.selected {
            None => 0,
            Some(i) if down => (i + 1).min(count - 1),
            Some(i) => i.saturating_sub(1),
        };
        self.selected = Some(next);
        self.scroll
            .scroll_to_item_strict(next, ScrollStrategy::Center);
        cx.notify();
        true
    }

    fn show_commit(&mut self, sha: String, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if !(self.host.show_commit)(&root, &sha, cx) {
            self.note = Some(host::NO_HISTORY.into());
        }
    }

    fn explain(&mut self, sha: String, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if !(self.host.explain_commit)(&root, &sha, cx) {
            self.note = Some(host::NO_AI.into());
        }
    }

    fn open_file(&mut self, cx: &mut Context<Self>) {
        let (Some(root), Some(file)) = (self.root.clone(), self.file.clone()) else {
            return;
        };
        if !(self.host.open_file)(&root, &file, cx) {
            self.note = Some(host::NO_EDITOR.into());
        }
    }

    pub fn heading(&self) -> String {
        let name = self
            .file
            .as_deref()
            .map(model::base_name)
            .unwrap_or_default();
        format!(
            "Who changed each line: {}",
            if name.is_empty() {
                "no file".to_string()
            } else {
                name
            }
        )
    }

    pub fn stats(&self) -> String {
        if self.parsed.lines.is_empty() {
            return String::new();
        }
        format!(
            "{} · {}",
            model::plural(self.parsed.lines.len(), "line", "lines"),
            model::plural(self.commit_count, "commit", "commits")
        )
    }

    pub fn message(&self) -> Option<&'static str> {
        if self.loading {
            return Some(if self.background {
                "Git is working in the background. Big files with a long history take a while."
            } else {
                "Reading who changed each line…"
            });
        }
        (self.error.is_none() && self.parsed.lines.is_empty()).then_some("This file is empty.")
    }

    fn top(&self, p: &Pal, cx: &mut Context<Self>) -> Div {
        let busy = self.loading;
        row()
            .flex_none()
            .pt(px(8.0))
            .gap(px(6.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(12.0))
                    .text_color(p.label)
                    .child(self.file.clone().unwrap_or_default()),
            )
            .when(!self.rev.is_empty(), |el| {
                el.child(kit::tag(
                    p,
                    "blame-rev",
                    format!("at {}", model::short_rev(&self.rev)),
                    p.accent,
                    "You are looking at the file as it was in this commit",
                ))
            })
            .when(!self.stack.is_empty(), |el| {
                let off = ChipLook {
                    disabled: busy,
                    ..ChipLook::default()
                };
                el.child(kit::chip(
                    p,
                    "blame-back",
                    "← Back",
                    off,
                    "Go back to the version you saw before",
                    cx,
                    |v, _, cx| v.go_back(cx),
                ))
                .child(kit::chip(
                    p,
                    "blame-now",
                    if self.start_rev.is_empty() {
                        "As it is now"
                    } else {
                        "Start again"
                    },
                    off,
                    "Show the file as it was when this window opened",
                    cx,
                    |v, _, cx| v.go_now(cx),
                ))
            })
            .when(self.rev.is_empty(), |el| {
                el.child(kit::chip(
                    p,
                    "blame-open",
                    "Open file",
                    ChipLook::default(),
                    "Open this file in the editor",
                    cx,
                    |v, _, cx| v.open_file(cx),
                ))
            })
            .child(kit::chip(
                p,
                "blame-reload",
                "↻ Reload",
                ChipLook {
                    disabled: busy,
                    ..ChipLook::default()
                },
                "Read the file again",
                cx,
                |v, _, cx| v.load(false, cx),
            ))
    }

    fn tools(&self, p: &Pal, cx: &mut Context<Self>) -> Div {
        let empty = self.parsed.lines.is_empty();
        row()
            .flex_none()
            .pt(px(8.0))
            .gap(px(8.0))
            .child(div().w(px(240.0)).flex_none().child(kit::field_box(
                p,
                "blame-find",
                self.find.clone(),
            )))
            .child(kit::chip(
                p,
                "blame-find-go",
                "Go",
                ChipLook {
                    disabled: empty,
                    ..ChipLook::default()
                },
                "Jump to the line number, or to the next line with this text",
                cx,
                |v, _, cx| v.run_find(cx),
            ))
            .when_some(self.note.clone(), |el, note| {
                el.child(
                    div()
                        .flex_none()
                        .text_size(px(11.5))
                        .text_color(p.warn)
                        .child(note),
                )
            })
            .child(div().flex_1())
            .when(self.parsed.truncated, |el| {
                el.child(
                    div()
                        .flex_none()
                        .text_size(px(11.5))
                        .text_color(p.warn)
                        .child(format!(
                            "Showing the first {} lines",
                            model::commas(MAX_LINES)
                        )),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(p.hint)
                    .child(self.stats()),
            )
            .when(self.loading && self.background, |el| {
                el.child(kit::button(
                    p,
                    "blame-stop",
                    "Stop",
                    Look {
                        small: true,
                        danger: true,
                        ..Look::default()
                    },
                    "Stop git",
                    cx,
                    |v, _, cx| v.stop(cx),
                ))
            })
    }

    fn problem(&self, p: &Pal, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let e = self.error.as_ref()?;
        let slow = e.kind == Kind::Timeout;
        let text = if !e.detail.is_empty() && e.detail != e.title {
            format!("{}\n{}", e.title, e.detail)
        } else {
            e.title.clone()
        };
        let hint = if slow {
            "This file has a long history. Git can keep working in the background.".to_string()
        } else {
            e.hint.clone()
        };
        Some(
            col()
                .id("blame-problem")
                .flex_none()
                .mt(px(8.0))
                .gap(px(6.0))
                .child(kit::error_text(p, text))
                .when(!hint.is_empty(), |el| el.child(kit::hint(p, hint)))
                .when(slow, |el| {
                    el.child(row().gap(px(6.0)).child(kit::button(
                        p,
                        "blame-bg",
                        "Try again with no time limit",
                        Look {
                            small: true,
                            primary: true,
                            ..Look::default()
                        },
                        "",
                        cx,
                        |v, _, cx| v.load(true, cx),
                    )))
                }),
        )
    }

    fn line_row(
        &self,
        i: usize,
        p: &Pal,
        gutter: f32,
        sel_sha: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let line: &Line = &self.parsed.lines[i];
        let band = self.bands.get(i).copied().unwrap_or(Band {
            first: true,
            band: 0,
        });
        let color = band_color(p, band.band);
        let selected = self.selected == Some(i);
        let same = !selected && sel_sha == Some(line.sha.as_str());
        let first = band.first;
        let hover = p.hover;
        let selector = format!("blame-line-{i}");
        let id_text = if !first {
            String::new()
        } else if line.uncommitted {
            "changed".into()
        } else {
            line.short.clone()
        };
        let who = if !first {
            String::new()
        } else if line.uncommitted {
            "Not committed yet".into()
        } else {
            line.author.clone()
        };
        let age = if first {
            self.ages.get(&line.sha).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        let text = self
            .texts
            .get(i)
            .cloned()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| " ".into());
        div()
            .id(("blame-line", i))
            .debug_selector(move || selector.clone())
            .flex()
            .flex_row()
            .min_w(px(self.width))
            .w_full()
            .h(px(ROW))
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .pr(px(12.0))
            .cursor_pointer()
            .when(selected, |el| el.bg(p.selected))
            .when(same, |el| el.bg(color.opacity(0.13)))
            .when(!selected && !same, |el| el.hover(move |s| s.bg(hover)))
            .when(first && i > 0, |el| el.border_t_1().border_color(p.border))
            .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.select(i, false, cx)))
            .child(div().w(px(3.0)).h_full().flex_none().bg(color))
            .child(
                div()
                    .w(px(gutter))
                    .flex_none()
                    .text_right()
                    .text_color(p.hint)
                    .child(line.n.to_string()),
            )
            .child(
                div()
                    .w(px(ID_W))
                    .flex_none()
                    .truncate()
                    .text_color(if line.uncommitted { p.warn } else { p.accent })
                    .child(id_text),
            )
            .child(
                div()
                    .w(px(AUTHOR_W))
                    .flex_none()
                    .truncate()
                    .text_color(if line.uncommitted { p.warn } else { p.label })
                    .child(who),
            )
            .child(
                div()
                    .w(px(AGE_W))
                    .flex_none()
                    .truncate()
                    .text_color(p.hint)
                    .child(age),
            )
            .child(
                div()
                    .flex_1()
                    .whitespace_nowrap()
                    .text_color(p.text)
                    .child(text),
            )
            .into_any_element()
    }

    fn list_zone(&self, p: &Pal, cx: &mut Context<Self>) -> Stateful<Div> {
        let show = self.error.is_none() && !self.parsed.lines.is_empty();
        let count = if show { self.parsed.lines.len() } else { 0 };
        let message = self.message();
        let list = uniform_list(
            "blame-lines",
            count,
            cx.processor(|v: &mut BlameView, range: std::ops::Range<usize>, _, cx| {
                let p = kit::pal(theme(cx));
                let gutter = model::gutter_width(v.parsed.lines.len());
                let sel_sha = v
                    .selected
                    .and_then(|i| v.parsed.lines.get(i))
                    .map(|l| l.sha.clone());
                range
                    .filter(|i| *i < v.parsed.lines.len())
                    .map(|i| v.line_row(i, &p, gutter, sel_sha.as_deref(), cx))
                    .collect::<Vec<_>>()
            }),
        )
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(self.scroll.clone())
        .size_full();
        col()
            .id("blame-zone")
            .flex_1()
            .min_h(px(80.0))
            .mt(px(8.0))
            .relative()
            .rounded(px(8.0))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .overflow_hidden()
            .font_family(MONO_FONT)
            .text_size(px(11.5))
            .child(list)
            .when_some(message, |el, message| {
                el.child(
                    row()
                        .absolute()
                        .top(px(10.0))
                        .left(px(12.0))
                        .gap(px(8.0))
                        .when(self.loading, |el| el.child(kit::spinner(p, "blame-spin")))
                        .child(div().text_size(px(12.0)).text_color(p.hint).child(message)),
                )
            })
    }

    fn detail(&self, p: &Pal, cx: &mut Context<Self>) -> AnyElement {
        if self.parsed.lines.is_empty() {
            return div().into_any_element();
        }
        let card = col()
            .id("blame-detail")
            .flex_none()
            .mt(px(8.0))
            .px(px(12.0))
            .py(px(9.0))
            .gap(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(p.border)
            .bg(p.raised);
        let Some(line) = self
            .selected
            .and_then(|i| self.parsed.lines.get(i))
            .cloned()
        else {
            return card
                .child(kit::hint(
                    p,
                    "Click a line to see who changed it and why. Use ↑ ↓ to move.",
                ))
                .into_any_element();
        };
        if line.uncommitted {
            return card
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.warn)
                        .child(format!("Line {} is not committed yet", line.n)),
                )
                .child(kit::hint(p, "You changed this line in the working copy. It gets a history when you commit it."))
                .into_any_element();
        }
        let c = self
            .parsed
            .commits
            .get(&line.sha)
            .cloned()
            .unwrap_or_default();
        let summary = if !c.summary.is_empty() {
            c.summary.clone()
        } else if !line.summary.is_empty() {
            line.summary.clone()
        } else {
            "(no message)".into()
        };
        let who = if !c.email.is_empty() {
            format!("{} <{}>", c.author, c.email)
        } else if !c.author.is_empty() {
            c.author.clone()
        } else {
            line.author.clone()
        };
        let place = match self.file.as_deref() {
            Some(f) if !line.filename.is_empty() && line.filename != f => {
                format!("line {}, was {}", line.n, line.filename)
            }
            _ => format!("line {}", line.n),
        };
        let copied = self.copied.as_deref() == Some(line.sha.as_str());
        let busy = self.loading;
        let small = Look {
            small: true,
            ..Look::default()
        };
        let sha = line.sha.clone();
        let sha2 = line.sha.clone();
        let sha3 = line.sha.clone();
        card.child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .whitespace_normal()
                .child(summary),
        )
        .child(
            row()
                .flex_wrap()
                .gap(px(8.0))
                .text_size(px(11.5))
                .child(div().flex_none().text_color(p.label).child(who))
                .child(
                    div()
                        .flex_none()
                        .text_color(p.hint)
                        .child(model::format_date(c.at, &c.tz)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(p.hint)
                        .child(self.ages.get(&line.sha).cloned().unwrap_or_default()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(p.accent)
                        .child(line.short.clone()),
                )
                .child(div().flex_none().text_color(p.hint).child(place)),
        )
        .when(c.boundary, |el| {
            el.child(kit::hint(p, "Git has no older history for this line."))
        })
        .child(
            row()
                .flex_wrap()
                .gap(px(6.0))
                .pt(px(2.0))
                .child(kit::button(
                    p,
                    "blame-show",
                    "Show in history",
                    small,
                    "Open the History tab at this commit",
                    cx,
                    move |v, _, cx| v.show_commit(sha.clone(), cx),
                ))
                .child(kit::button(
                    p,
                    "blame-explain",
                    "Explain with AI",
                    small,
                    "Ask the AI what this commit does",
                    cx,
                    move |v, _, cx| v.explain(sha2.clone(), cx),
                ))
                .child(kit::button(
                    p,
                    "blame-copy",
                    if copied { "Copied" } else { "Copy id" },
                    small,
                    line.sha.clone(),
                    cx,
                    move |v, _, cx| v.copy_id(sha3.clone(), cx),
                ))
                .child(kit::button(
                    p,
                    "blame-before",
                    "Before this commit",
                    Look {
                        disabled: busy || c.previous.is_none(),
                        ..small
                    },
                    if c.previous.is_some() {
                        "Show who changed each line just before this commit"
                    } else {
                        "This commit made the file. There is nothing before it."
                    },
                    cx,
                    |v, _, cx| v.go_before(cx),
                )),
        )
        .into_any_element()
    }
}

impl Focusable for BlameView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for BlameView {
    fn drop(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
    }
}

impl Render for BlameView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = kit::pal(theme(cx));
        let heading = self.heading();
        window.set_window_title(&heading);
        let head = kit::head(
            &p,
            "blame-view",
            SharedString::from(heading),
            cx,
            |_, window, _| window.remove_window(),
        );
        let top = self.top(&p, cx);
        let tools = self.tools(&p, cx);
        let problem = self.problem(&p, cx);
        let list = self.list_zone(&p, cx);
        let detail = self.detail(&p, cx);
        kit::root(&p, "blame-view")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &ClosePane, window, _| window.remove_window()))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "down" => {
                        if v.step(true, cx) {
                            cx.stop_propagation();
                        }
                    }
                    "up" => {
                        if v.step(false, cx) {
                            cx.stop_propagation();
                        }
                    }
                    "escape" => {
                        cx.stop_propagation();
                        window.remove_window();
                    }
                    _ => {}
                }
            }))
            .child(head)
            .child(top)
            .child(tools)
            .children(problem)
            .child(list)
            .child(detail)
    }
}
