use super::diff::{self, Hunk, Pick, Row, RowKind, Side};
use super::git::{FileEntry, base_name, plural};
use super::patch::{self, Mode};
use super::words::{Level, Note, note};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, MouseButton, MouseDownEvent,
    ScrollStrategy, SharedString, UniformListScrollHandle, Window, div, px, uniform_list,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::modal::{dialog, overlay};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon, theme};

pub const ROW_H: f32 = 20.;
pub const SPACE_TIP: &str = "Turn off ignore spaces to stage or discard parts of a file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffEvent {
    Changed,
    Notify(Note),
    OpenFile { repo: String, path: String },
    Close,
}

pub struct DiffView {
    pub repo: String,
    pub path: String,
    pub file: Option<FileEntry>,
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
    pub pick: Option<Pick>,
    pub line_ask: bool,
    pub hunk_pick: Option<usize>,
    pub ask_hunk: Option<usize>,
    pub ignore_space: bool,
    pub busy: bool,
    pub loading: bool,
    pub again: bool,
    pub note: Option<String>,
    pub cursor: Option<usize>,
    pub starts: Vec<usize>,
    pub shift: bool,
    pub seq: u64,
    pub scroll: UniformListScrollHandle,
}

impl EventEmitter<DiffEvent> for DiffView {}

pub fn build(repo: String, path: String) -> DiffView {
    DiffView {
        repo,
        path,
        file: None,
        rows: Vec::new(),
        hunks: Vec::new(),
        pick: None,
        line_ask: false,
        hunk_pick: None,
        ask_hunk: None,
        ignore_space: false,
        busy: false,
        loading: false,
        again: false,
        note: None,
        cursor: None,
        starts: Vec::new(),
        shift: false,
        seq: 0,
        scroll: UniformListScrollHandle::new(),
    }
}

pub fn new(cx: &mut App, repo: String, path: String) -> Entity<DiffView> {
    let view = cx.new(|_| build(repo, path));
    refresh_entity(&view, cx);
    view
}

pub fn refresh_entity(view: &Entity<DiffView>, cx: &mut App) {
    view.update(cx, refresh);
}

pub fn set_rows(v: &mut DiffView, rows: Vec<Row>, hunks: Vec<Hunk>) {
    v.rows = rows;
    v.hunks = hunks;
    v.pick = diff::keep_pick(v.pick.take(), &v.hunks);
    if v.pick.is_none() {
        v.line_ask = false;
    }
    if let Some(h) = v.hunk_pick {
        let label = v.hunks.get(h).map(|x| x.label.clone());
        v.hunk_pick = None;
        if let Some(label) = label {
            v.hunk_pick = v.hunks.iter().find(|x| x.label == label).map(|x| x.id);
        }
    }
    v.starts = diff::change_starts(&v.rows);
}

struct Load {
    file: Option<FileEntry>,
    result: Result<diff::Loaded, String>,
}

fn load(repo: &str, path: &str, ignore_space: bool) -> Load {
    match diff::file_entry(repo, path) {
        Err(e) => Load {
            file: None,
            result: Err(e),
        },
        Ok(None) => Load {
            file: None,
            result: Ok(diff::finish(diff::note_rows(&["No changes left in this file"]), Vec::new(), None)),
        },
        Ok(Some(f)) => {
            let result = diff::load(repo, &f, ignore_space);
            Load { file: Some(f), result }
        }
    }
}

pub fn refresh(v: &mut DiffView, cx: &mut Context<DiffView>) {
    if v.loading || v.busy {
        v.again = true;
        return;
    }
    v.loading = true;
    v.again = false;
    v.seq += 1;
    let seq = v.seq;
    let repo = v.repo.clone();
    let path = v.path.clone();
    let space = v.ignore_space;
    let job = cx.background_executor().spawn(async move { load(&repo, &path, space) });
    cx.spawn(async move |this, cx| {
        let done = job.await;
        this.update(cx, |v, cx| {
            if v.seq != seq {
                return;
            }
            v.loading = false;
            match done.result {
                Ok(mut l) => {
                    let mut f = done.file;
                    if let (Some(f), Some(add)) = (f.as_mut(), l.add) {
                        f.add = add;
                    }
                    v.file = f;
                    v.note = None;
                    let rows = std::mem::take(&mut l.rows);
                    set_rows(v, rows, l.hunks);
                }
                Err(e) => {
                    v.note = Some(format!("Cannot load this diff: {e}"));
                    set_rows(v, Vec::new(), Vec::new());
                }
            }
            if v.again {
                refresh(v, cx);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn line_ready(v: &DiffView, h: &Hunk) -> bool {
    h.whole && !v.ignore_space && h.side.lines_ok()
}

pub fn click_row(v: &mut DiffView, index: usize, shift: bool, cx: &mut Context<DiffView>) {
    let Some(row) = v.rows.get(index).cloned() else { return };
    if row.kind == RowKind::Bar {
        v.hunk_pick = if v.hunk_pick == row.hunk { None } else { row.hunk };
        v.line_ask = false;
        cx.notify();
        return;
    }
    let Some(id) = row.hunk else { return };
    let Some(h) = v.hunks.get(id).cloned() else { return };
    if !line_ready(v, &h) || v.busy {
        return;
    }
    v.pick = diff::toggle_pick(v.pick.take(), &h, row.at, shift);
    v.line_ask = false;
    v.hunk_pick = Some(id);
    cx.notify();
}

fn words(mode: Mode, lines: Option<usize>) -> (String, &'static str) {
    match (mode, lines) {
        (Mode::Stage, None) => ("Staged hunk".into(), "Stage hunk failed"),
        (Mode::Unstage, None) => ("Unstaged hunk".into(), "Unstage hunk failed"),
        (Mode::Discard, None) => ("Discarded hunk".into(), "Discard hunk failed"),
        (Mode::Stage, Some(n)) => (format!("Staged {}", plural(n, "line", "lines")), "Stage lines failed"),
        (Mode::Unstage, Some(n)) => (format!("Unstaged {}", plural(n, "line", "lines")), "Unstage lines failed"),
        (Mode::Discard, Some(n)) => (format!("Discarded {}", plural(n, "line", "lines")), "Discard lines failed"),
    }
}

pub fn run_patch(v: &mut DiffView, text: String, mode: Mode, lines: Option<usize>, cx: &mut Context<DiffView>) {
    if v.busy || text.is_empty() {
        return;
    }
    v.busy = true;
    let repo = v.repo.clone();
    let path = v.path.clone();
    cx.notify();
    let job = cx.background_executor().spawn(async move { patch::apply(&repo, &text, mode) });
    cx.spawn(async move |this, cx| {
        let r = job.await;
        this.update(cx, |v, cx| {
            let (ok, fail) = words(mode, lines);
            if r.ok {
                cx.emit(DiffEvent::Notify(note(ok.clone(), path.clone(), Level::Success, ok)));
                v.pick = None;
                v.line_ask = false;
            } else {
                let body = if r.body().is_empty() { r.title.clone() } else { r.body() };
                cx.emit(DiffEvent::Notify(note(fail, format!("{body} ({path})"), Level::Error, fail)));
            }
            v.busy = false;
            cx.emit(DiffEvent::Changed);
            refresh(v, cx);
        })
        .ok();
    })
    .detach();
}

pub fn apply_hunk(v: &mut DiffView, id: usize, mode: Mode, cx: &mut Context<DiffView>) {
    let Some(h) = v.hunks.get(id) else { return };
    let text = h.patch.clone();
    run_patch(v, text, mode, None, cx);
}

pub fn apply_lines(v: &mut DiffView, mode: Mode, cx: &mut Context<DiffView>) {
    v.line_ask = false;
    let Some(p) = v.pick.clone() else { return };
    let Some(h) = v.hunks.get(p.hunk).cloned() else { return };
    if h.patch != p.patch || !line_ready(v, &h) {
        cx.notify();
        return;
    }
    match patch::line_patch(&h.header, &h.lines, &p.lines, mode) {
        Ok(text) => run_patch(v, text, mode, Some(p.lines.len()), cx),
        Err(e) => {
            let (_, fail) = words(mode, Some(p.lines.len()));
            cx.emit(DiffEvent::Notify(note(fail, e, Level::Warning, fail)));
            cx.notify();
        }
    }
}

pub fn set_ignore_space(v: &mut DiffView, on: bool, cx: &mut Context<DiffView>) {
    if v.ignore_space == on {
        return;
    }
    v.ignore_space = on;
    v.pick = None;
    v.line_ask = false;
    v.hunk_pick = None;
    v.cursor = None;
    refresh(v, cx);
}

pub fn jump(v: &mut DiffView, down: bool, cx: &mut Context<DiffView>) {
    if v.starts.is_empty() {
        return;
    }
    let target = diff::next_change(&v.starts, v.cursor, down).unwrap_or(if down {
        v.starts[0]
    } else {
        v.starts[v.starts.len() - 1]
    });
    v.cursor = Some(target);
    v.scroll.scroll_to_item(target.saturating_sub(2), ScrollStrategy::Top);
    cx.notify();
}

fn small(t: &Theme, id: impl Into<SharedString>, label: impl Into<SharedString>, off: bool, tip: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = id.into();
    let sel = id.to_string();
    let hover = t.hover;
    let accent = t.accent;
    div()
        .id(id)
        .debug_selector(move || sel)
        .h(px(18.))
        .px(px(6.))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(t.border)
        .bg(t.raised)
        .text_size(px(10.5))
        .text_color(if off { t.dim } else { t.muted })
        .when(!off, |el| el.cursor_pointer().hover(move |s| s.bg(hover).border_color(accent)))
        .tooltip(tooltip(tip.into()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label.into())
}

pub fn check_box(t: &Theme, id: &'static str, on: bool, label: &'static str, off: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.))
        .when(!off, |el| el.cursor_pointer())
        .when(off, |el| el.opacity(0.5))
        .child(
            div()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if on { t.accent } else { t.border_strong })
                .when(on, |el| el.bg(t.accent))
                .when(on, |el| el.child(icon(Icon::Check).size(px(10.)).text_color(t.accent_foreground))),
        )
        .child(div().text_size(px(11.5)).text_color(t.muted).child(label))
}

fn header(v: &DiffView, t: &Theme, cx: &mut Context<DiffView>) -> AnyElement {
    let status = v.file.as_ref().map(|f| f.status.to_string()).unwrap_or_default();
    let staged = v.file.as_ref().is_some_and(|f| f.staged);
    div()
        .h(px(34.))
        .flex_none()
        .px(px(10.))
        .gap(px(8.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .bg(t.chrome)
        .child(div().flex_none().text_size(px(11.)).text_color(if staged { t.accent } else { t.muted }).child(status))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(t.foreground).child(v.path.clone()))
        .child(div().flex_none().max_w(px(120.)).truncate().text_size(px(11.)).text_color(t.dim).child(base_name(&v.repo).to_string()))
        .child(small(t, "diff-open-file", "open file", false, format!("Open {} in the editor", v.path)).on_click(cx.listener(|v, _, _, cx| {
            cx.emit(DiffEvent::OpenFile {
                repo: v.repo.clone(),
                path: v.path.clone(),
            })
        })))
        .child(
            small(t, "diff-refresh", "refresh", v.loading || v.busy, "Load the changes again")
                .on_click(cx.listener(|v, _, _, cx| refresh(v, cx))),
        )
        .child(
            icon_button(t, "diff-close", Icon::X, IconSize::Small, false)
                .debug_selector(|| "diff-close".into())
                .tooltip(tooltip("Close the diff"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DiffEvent::Close))),
        )
        .into_any_element()
}

fn controls(v: &DiffView, t: &Theme, cx: &mut Context<DiffView>) -> AnyElement {
    let n = v.starts.len();
    let tip = |dir: &str| {
        if n > 0 {
            format!("Go to the change {dir} ({})", plural(n, "change", "changes"))
        } else {
            "No changes to move to".to_string()
        }
    };
    let on = v.ignore_space;
    div()
        .h(px(28.))
        .flex_none()
        .px(px(10.))
        .gap(px(6.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .child(
            check_box(t, "diff-ignore-space", on, "ignore spaces", false)
                .tooltip(tooltip("Hide changes that only add or remove spaces. Stage and discard buttons are off while this is on."))
                .on_click(cx.listener(move |v, _, _, cx| set_ignore_space(v, !on, cx))),
        )
        .child(div().flex_1())
        .child(small(t, "diff-prev-change", "↑ previous change", n == 0, tip("above")).on_click(cx.listener(|v, _, _, cx| jump(v, false, cx))))
        .child(small(t, "diff-next-change", "↓ next change", n == 0, tip("below")).on_click(cx.listener(|v, _, _, cx| jump(v, true, cx))))
        .into_any_element()
}

fn hunk_actions(v: &DiffView, t: &Theme, id: usize, cx: &mut Context<DiffView>) -> Option<AnyElement> {
    let h = v.hunks.get(id)?.clone();
    let bar = div()
        .id("hunk-actions")
        .debug_selector(|| "hunk-actions".into())
        .w_full()
        .h(px(26.))
        .flex_none()
        .px(px(10.))
        .gap(px(6.))
        .flex()
        .items_center()
        .justify_end()
        .bg(t.canvas)
        .border_b_1()
        .border_color(t.border);
    let pick = v.pick.as_ref().filter(|p| p.hunk == id).cloned();
    let working = h.side.working();
    let busy = v.busy;
    if let Some(p) = pick {
        let what = plural(p.lines.len(), "line", "lines");
        if v.line_ask {
            return Some(
                bar.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(10.5))
                        .text_color(t.red)
                        .child(format!("Discard {what} in {}? This cannot be undone.", base_name(&h.path))),
                )
                .child(small(t, "hunk-lines-yes", "Yes, discard", busy, format!("Discard the picked {what} of {}", h.path)).on_click(cx.listener(|v, _, _, cx| apply_lines(v, Mode::Discard, cx))))
                .child(small(t, "hunk-lines-no", "No", false, "Keep the lines").on_click(cx.listener(|v, _, _, cx| {
                    v.line_ask = false;
                    cx.notify();
                })))
                .into_any_element(),
            );
        }
        let mut bar = bar.child(div().flex_1().min_w_0().truncate().text_size(px(10.5)).text_color(t.accent).child(format!("{what} picked")));
        if working {
            bar = bar
                .child(small(t, "hunk-lines-stage", "Stage lines", busy, format!("Stage only the picked {what} of {}", h.path)).on_click(cx.listener(|v, _, _, cx| apply_lines(v, Mode::Stage, cx))))
                .child(small(t, "hunk-lines-discard", "Discard lines", busy, format!("Discard only the picked {what} of {} (asks first)", h.path)).on_click(cx.listener(|v, _, _, cx| {
                    v.line_ask = true;
                    cx.notify();
                })));
        }
        if h.side == Side::Staged {
            bar = bar.child(small(t, "hunk-lines-unstage", "Unstage lines", busy, format!("Unstage only the picked {what} of {}", h.path)).on_click(cx.listener(|v, _, _, cx| apply_lines(v, Mode::Unstage, cx))));
        }
        bar = bar.child(small(t, "hunk-lines-clear", "✕", false, "Clear the picked lines").on_click(cx.listener(|v, _, _, cx| {
            v.pick = None;
            v.line_ask = false;
            cx.notify();
        })));
        return Some(bar.into_any_element());
    }
    let blocked = v.ignore_space || !h.whole;
    let why = if v.ignore_space {
        SPACE_TIP.to_string()
    } else {
        "This part of the diff is not complete. Stage or discard the whole file instead.".to_string()
    };
    let tip = |text: String| if blocked { why.clone() } else { text };
    let off = busy || blocked;
    let stage_tip = if h.side == Side::Untracked {
        format!("Stage all of {}", h.path)
    } else {
        format!("Stage these lines of {}", h.path)
    };
    let discard_tip = if h.side == Side::Untracked {
        format!("Delete the new file {} (asks first)", h.path)
    } else {
        format!("Discard these lines of {} (asks first)", h.path)
    };
    Some(
        bar.child(div().flex_1().min_w_0().truncate().text_size(px(10.5)).text_color(t.accent).child(h.label.clone()))
            .child(small(t, "hunk-stage", "Stage hunk", off || !working, tip(stage_tip)).when(!off && working, |el| {
                el.on_click(cx.listener(move |v, _, _, cx| apply_hunk(v, id, Mode::Stage, cx)))
            }))
            .child(small(t, "hunk-unstage", "Unstage hunk", off || h.side != Side::Staged, tip(format!("Unstage these lines of {}", h.path))).when(
                !off && h.side == Side::Staged,
                |el| el.on_click(cx.listener(move |v, _, _, cx| apply_hunk(v, id, Mode::Unstage, cx))),
            ))
            .child(small(t, "hunk-discard", "Discard hunk", off || !working, tip(discard_tip)).when(!off && working, |el| {
                el.on_click(cx.listener(move |v, _, _, cx| {
                    v.ask_hunk = Some(id);
                    cx.notify();
                }))
            }))
            .into_any_element(),
    )
}

fn line_text(row: &Row, strong: gpui::Hsla) -> AnyElement {
    if row.spans.is_empty() {
        return div().flex_1().min_w_0().truncate().child(row.text.clone()).into_any_element();
    }
    let mut parts: Vec<AnyElement> = Vec::new();
    let mut at = 0;
    for (s, e) in &row.spans {
        let (s, e) = (*s.min(&row.text.len()), *e.min(&row.text.len()));
        if s > at {
            parts.push(div().flex_none().child(row.text[at..s].to_string()).into_any_element());
        }
        if e > s {
            parts.push(div().flex_none().rounded(px(2.)).bg(strong).child(row.text[s..e].to_string()).into_any_element());
        }
        at = e.max(at);
    }
    if at < row.text.len() {
        parts.push(div().flex_none().child(row.text[at..].to_string()).into_any_element());
    }
    div().flex_1().min_w_0().flex().overflow_hidden().whitespace_nowrap().children(parts).into_any_element()
}

fn diff_row(v: &DiffView, index: usize, t: &Theme, cx: &mut Context<DiffView>) -> AnyElement {
    let Some(row) = v.rows.get(index) else {
        return div().into_any_element();
    };
    let base = div().id(("diff-row", index)).w_full().h(px(ROW_H)).flex_none();
    match row.kind {
        RowKind::Section => {
            return base
                .px(px(12.))
                .flex()
                .items_center()
                .bg(t.raised)
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(row.text.clone())
                .into_any_element();
        }
        RowKind::Bar => {
            let picked = v.hunk_pick == row.hunk && row.hunk.is_some();
            let pick = v.pick.as_ref().filter(|p| Some(p.hunk) == row.hunk);
            let hint = match pick {
                Some(p) => format!("{} picked", plural(p.lines.len(), "line", "lines")),
                None if picked => "chosen · use the buttons above".into(),
                None => "click to choose".into(),
            };
            let hover = t.hover;
            let sel = format!("hunkbar-{}", row.hunk.unwrap_or(0));
            return base
                .debug_selector(move || sel)
                .px(px(12.))
                .gap(px(6.))
                .flex()
                .items_center()
                .bg(if picked { t.selected } else { t.canvas })
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .when(picked, |el| el.border_l_2().border_color(t.accent))
                .tooltip(tooltip(format!("{}\nClick to choose this part. Its buttons show above the diff. Click a + or − line to pick single lines.", row.text)))
                .on_click(cx.listener(move |v, _, _, cx| click_row(v, index, false, cx)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(10.5)).text_color(if picked { t.accent } else { t.muted }).child(row.text.clone()))
                .child(div().flex_none().text_size(px(10.5)).text_color(if picked { t.accent } else { t.dim }).child(hint))
                .into_any_element();
        }
        RowKind::At => {
            return base
                .px(px(12.))
                .flex()
                .items_center()
                .bg(t.raised)
                .text_color(t.dim)
                .child(div().truncate().child(row.text.clone()))
                .into_any_element();
        }
        _ => {}
    }
    let hunk = row.hunk.and_then(|h| v.hunks.get(h));
    let pickable = hunk.is_some_and(|h| line_ready(v, h)) && !v.busy;
    let picked = hunk.is_some() && v.pick.as_ref().is_some_and(|p| Some(p.hunk) == row.hunk && p.lines.contains(&row.at));
    let strong = if row.kind == RowKind::Add { t.green.opacity(0.33) } else { t.red.opacity(0.33) };
    let hover = t.hover;
    let sel = format!("line-{index}");
    base.debug_selector(move || sel)
        .gap(px(10.))
        .flex()
        .items_center()
        .when(row.kind == RowKind::Add, |el| el.bg(t.green.opacity(0.09)))
        .when(row.kind == RowKind::Del, |el| el.bg(t.red.opacity(0.09)))
        .when(row.kind == RowKind::Plain, |el| el.px(px(12.)).text_color(t.dim))
        .when(picked, |el| el.bg(t.selected).border_l_2().border_color(t.accent))
        .when(pickable, |el| {
            el.cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_mouse_down(MouseButton::Left, cx.listener(|v, e: &MouseDownEvent, _, _| v.shift = e.modifiers.shift))
                .on_click(cx.listener(move |v, _, _, cx| {
                    let shift = v.shift;
                    click_row(v, index, shift, cx)
                }))
        })
        .when(row.kind != RowKind::Plain, |el| {
            el.child(
                div()
                    .w(px(34.))
                    .flex_none()
                    .text_right()
                    .text_color(if picked { t.accent } else { t.dim })
                    .child(row.number.map(|n| n.to_string()).unwrap_or_default()),
            )
            .child(
                div()
                    .w(px(10.))
                    .flex_none()
                    .text_color(if row.kind == RowKind::Add { t.green } else if row.kind == RowKind::Del { t.red } else { t.dim })
                    .child(match row.kind {
                        RowKind::Add => "+",
                        RowKind::Del => "−",
                        _ => " ",
                    }),
            )
        })
        .child(line_text(row, strong))
        .into_any_element()
}

fn ask_layer(v: &DiffView, t: &Theme, cx: &mut Context<DiffView>) -> Option<AnyElement> {
    let id = v.ask_hunk?;
    let h = v.hunks.get(id)?;
    let text = if h.side == Side::Untracked {
        format!("Delete the new file {}. This cannot be undone.", h.path)
    } else {
        format!("Discard this hunk of {}. This cannot be undone.", h.path)
    };
    let body = div().text_size(px(12.5)).text_color(t.foreground).child(text);
    let footer = div()
        .flex()
        .gap(px(8.))
        .child(button(t, "diff-ask-no", "Cancel", ButtonKind::Secondary).debug_selector(|| "diff-ask-no".into()).on_click(cx.listener(|v, _, _, cx| {
            v.ask_hunk = None;
            cx.notify();
        })))
        .child(
            disabled(button(t, "diff-ask-yes", "Discard", ButtonKind::Danger).debug_selector(|| "diff-ask-yes".into()), v.busy)
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.ask_hunk = None;
                    apply_hunk(v, id, Mode::Discard, cx);
                })),
        )
        .into_any_element();
    let this = cx.entity().downgrade();
    let this2 = this.clone();
    let card = dialog(t, "diff-ask", "Discard these changes?", 420., body, Some(footer), move |_, app| {
        this.update(app, |v, cx| {
            v.ask_hunk = None;
            cx.notify();
        })
        .ok();
    });
    Some(
        overlay(t, "diff-ask-scrim", card, move |_, app| {
            this2
                .update(app, |v, cx| {
                    v.ask_hunk = None;
                    cx.notify();
                })
                .ok();
        })
        .into_any_element(),
    )
}

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let count = self.rows.len();
        let list = uniform_list(
            "changes-diff",
            count,
            cx.processor(|v: &mut DiffView, range: std::ops::Range<usize>, _, cx| {
                let t = theme(cx).clone();
                range.map(|i| diff_row(v, i, &t, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full();
        let actions = self.hunk_pick.and_then(|id| hunk_actions(self, &t, id, cx));
        let ask = ask_layer(self, &t, cx);
        div()
            .id("diff-view")
            .debug_selector(|| "diff-view".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.))
            .child(header(self, &t, cx))
            .when_some(self.note.clone(), |el, n| {
                el.child(div().px(px(10.)).py(px(4.)).text_size(px(11.)).text_color(t.red).child(n))
            })
            .child(controls(self, &t, cx))
            .children(actions)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .child(list),
            )
            .when(self.loading && self.rows.is_empty(), |el| {
                el.child(div().p(px(8.)).text_color(t.dim).child("Loading the diff…"))
            })
            .children(ask)
    }
}

