use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Global,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, ScrollWheelEvent, SharedString, Style, Styled, TextRun, TextStyle, UTF16Selection, UnderlineStyle,
    Window, WrappedLine, actions, div, fill, point, prelude::FluentBuilder, px, relative, size,
};
use nixa_ui::text::{BASE, MONO_FONT};
use nixa_ui::text_input::{from_utf16, next_char, prev_char, splice, to_utf16};
use nixa_ui::theme;
use std::ops::Range;

mod preview;
pub use preview::PreviewSpan;

actions!(
    nixa_text_area,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        LineStart,
        LineEnd,
        DocStart,
        DocEnd,
        Paste,
        Copy,
        Cut,
        Newline,
        LineBreak,
        Indent,
        Save,
        Cancel
    ]
);

pub const KEY_CONTEXT: &str = "NixaTextArea";
pub const LINE_HEIGHT: f32 = 19.0;

struct KeysBound;

impl Global for KeysBound {}

pub fn bind_keys(cx: &mut App) {
    if cx.try_global::<KeysBound>().is_some() {
        return;
    }
    let ctx = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("home", LineStart, ctx),
        KeyBinding::new("cmd-left", LineStart, ctx),
        KeyBinding::new("end", LineEnd, ctx),
        KeyBinding::new("cmd-right", LineEnd, ctx),
        KeyBinding::new("cmd-up", DocStart, ctx),
        KeyBinding::new("cmd-down", DocEnd, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("cmd-x", Cut, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", LineBreak, ctx),
        KeyBinding::new("tab", Indent, ctx),
        KeyBinding::new("cmd-s", Save, ctx),
        KeyBinding::new("escape", Cancel, ctx),
    ]);
    cx.set_global(KeysBound);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AreaEvent {
    Changed(String),
    Save,
    Cancel,
}

pub fn line_starts(value: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(value.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

pub fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|s| *s <= offset).saturating_sub(1)
}

pub fn line_end(value: &str, starts: &[usize], line: usize) -> usize {
    starts
        .get(line + 1)
        .map(|next| next - 1)
        .unwrap_or(value.len())
}

pub fn clean_paste(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn wrap_starts(line: &WrappedLine) -> Vec<usize> {
    line.wrap_boundaries()
        .iter()
        .filter_map(|b| line.runs().get(b.run_ix)?.glyphs.get(b.glyph_ix).map(|g| g.index))
        .collect()
}

struct Row {
    start: usize,
    top: Pixels,
    line: WrappedLine,
}

struct Shaped {
    rows: Vec<Row>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    height: Pixels,
}

impl Shaped {
    fn row_height(&self, row: &Row) -> Pixels {
        self.line_height * (row.line.wrap_boundaries().len() + 1) as f32
    }

    fn row_at(&self, y: Pixels) -> usize {
        let mut at = 0;
        for (i, row) in self.rows.iter().enumerate() {
            if row.top <= y {
                at = i;
            }
        }
        at
    }

    fn position(&self, value: &str, offset: usize) -> Point<Pixels> {
        let starts = line_starts(value);
        let line = line_of(&starts, offset);
        let Some(row) = self.rows.get(line) else {
            return point(px(0.), px(0.));
        };
        let local = offset.saturating_sub(row.start).min(row.line.len());
        let at = row
            .line
            .position_for_index(local, self.line_height)
            .unwrap_or_default();
        point(at.x, row.top + at.y)
    }

    fn position_down(&self, value: &str, offset: usize) -> Point<Pixels> {
        let starts = line_starts(value);
        if let Some(row) = self.rows.get(line_of(&starts, offset)) {
            let local = offset.saturating_sub(row.start).min(row.line.len());
            if let Some(k) = wrap_starts(&row.line).iter().position(|start| *start == local) {
                return point(px(0.), row.top + self.line_height * (k + 1) as f32);
            }
        }
        self.position(value, offset)
    }

    fn caret(&self, value: &str, offset: usize, upstream: bool) -> Point<Pixels> {
        if upstream {
            self.position(value, offset)
        } else {
            self.position_down(value, offset)
        }
    }

    fn place(&self, value: &str, content: Point<Pixels>) -> (usize, bool) {
        let offset = self.offset_at(content);
        (offset, self.position_down(value, offset).y > content.y)
    }

    fn offset_at(&self, content: Point<Pixels>) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        let index = self.row_at(content.y.max(px(0.)));
        let row = &self.rows[index];
        let mut local = point(content.x.max(px(0.)), (content.y - row.top).max(px(0.)));
        let last = self.row_height(row) - self.line_height;
        if local.y > last {
            local.y = last + self.line_height / 2.;
        }
        let found = match row.line.closest_index_for_position(local, self.line_height) {
            Ok(i) | Err(i) => i,
        };
        row.start + found.min(row.line.len())
    }
}

pub struct TextArea {
    focus: FocusHandle,
    value: String,
    placeholder: SharedString,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    shaped: Option<Shaped>,
    scroll: Pixels,
    reveal: bool,
    dragging: bool,
    goal_x: Option<Pixels>,
    mono: bool,
    max_chars: Option<usize>,
    auto_rows: Option<(u16, u16)>,
    preview: Option<Vec<PreviewSpan>>,
    press: Option<Point<Pixels>>,
    enter_sends: bool,
    upstream: bool,
}

impl EventEmitter<AreaEvent> for TextArea {}

impl Focusable for TextArea {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextArea {
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        bind_keys(cx);
        TextArea {
            focus: cx.focus_handle(),
            value: String::new(),
            placeholder: placeholder.into(),
            selection: 0..0,
            reversed: false,
            marked: None,
            shaped: None,
            scroll: px(0.),
            reveal: false,
            dragging: false,
            goal_x: None,
            mono: false,
            max_chars: None,
            auto_rows: None,
            preview: None,
            press: None,
            enter_sends: false,
            upstream: false,
        }
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn enter_sends(mut self) -> Self {
        self.enter_sends = true;
        self
    }

    pub fn limit(mut self, chars: usize) -> Self {
        self.max_chars = Some(chars);
        self
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        self.value = value.into();
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.scroll = px(0.);
        self.goal_x = None;
        self.upstream = false;
        cx.notify();
    }

    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.value.len());
        self.selection = offset..offset;
        self.reversed = false;
        self.reveal = true;
        self.upstream = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.value.len());
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        self.reveal = true;
        self.upstream = false;
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, insert: &str, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            return;
        }
        let mut insert = insert.to_string();
        if let Some(max) = self.max_chars {
            let kept = self.value.chars().count()
                - self
                    .value
                    .get(range.clone())
                    .map(|s| s.chars().count())
                    .unwrap_or(0);
            let room = max.saturating_sub(kept);
            insert = insert.chars().take(room).collect();
        }
        let (value, at) = splice(&self.value, range, &insert);
        self.value = value;
        self.selection = at..at;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        self.reveal = true;
        self.upstream = false;
        cx.emit(AreaEvent::Changed(self.value.clone()));
        cx.notify();
    }

    fn vertical(&mut self, down: bool) -> Option<(usize, bool)> {
        let shaped = self.shaped.as_ref()?;
        let at = shaped.caret(&self.value, self.cursor(), self.upstream);
        let x = *self.goal_x.get_or_insert(at.x);
        let step = shaped.line_height;
        let y = if down { at.y + step } else { at.y - step };
        if y < px(0.) {
            return Some((0, false));
        }
        if y >= shaped.height {
            return Some((self.value.len(), false));
        }
        Some(shaped.place(&self.value, point(x, y + step / 2.)))
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selection.is_empty() {
            prev_char(&self.value, self.cursor())
        } else {
            self.selection.start
        };
        self.goal_x = None;
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selection.is_empty() {
            next_char(&self.value, self.cursor())
        } else {
            self.selection.end
        };
        self.goal_x = None;
        self.move_to(to, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let goal = self.goal_x;
        if let Some((to, upstream)) = self.vertical(false) {
            let keep = self.goal_x.or(goal);
            self.move_to(to, cx);
            self.goal_x = keep;
            self.upstream = upstream;
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let goal = self.goal_x;
        if let Some((to, upstream)) = self.vertical(true) {
            let keep = self.goal_x.or(goal);
            self.move_to(to, cx);
            self.goal_x = keep;
            self.upstream = upstream;
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(prev_char(&self.value, self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(next_char(&self.value, self.cursor()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((to, upstream)) = self.vertical(false) {
            self.select_to(to, cx);
            self.upstream = upstream;
        }
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((to, upstream)) = self.vertical(true) {
            self.select_to(to, cx);
            self.upstream = upstream;
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = 0..self.value.len();
        self.reversed = false;
        cx.notify();
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let starts = line_starts(&self.value);
        let to = starts[line_of(&starts, self.cursor())];
        self.goal_x = None;
        self.move_to(to, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let starts = line_starts(&self.value);
        let to = line_end(&self.value, &starts, line_of(&starts, self.cursor()));
        self.goal_x = None;
        self.move_to(to, cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(0, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(self.value.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() {
            prev_char(&self.value, self.cursor())..self.cursor()
        } else {
            self.selection.clone()
        };
        self.replace(range, "", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() {
            self.cursor()..next_char(&self.value, self.cursor())
        } else {
            self.selection.clone()
        };
        self.replace(range, "", cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(clip) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(self.selection.clone(), &clean_paste(&clip), cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.value[self.selection.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.value[self.selection.clone()].to_string(),
            ));
            self.replace(self.selection.clone(), "", cx);
        }
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.enter_sends {
            cx.emit(AreaEvent::Save);
            return;
        }
        self.replace(self.selection.clone(), "\n", cx);
    }

    fn line_break(&mut self, _: &LineBreak, _: &mut Window, cx: &mut Context<Self>) {
        self.replace(self.selection.clone(), "\n", cx);
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        if self.enter_sends {
            cx.propagate();
            return;
        }
        self.replace(self.selection.clone(), "  ", cx);
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if self.enter_sends {
            cx.propagate();
            return;
        }
        cx.emit(AreaEvent::Save);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(AreaEvent::Cancel);
    }

    fn content_point(&self, position: Point<Pixels>) -> Option<Point<Pixels>> {
        let shaped = self.shaped.as_ref()?;
        Some(point(
            position.x - shaped.bounds.left(),
            position.y - shaped.bounds.top() + self.scroll,
        ))
    }

    fn offset_for(&self, position: Point<Pixels>) -> usize {
        self.place_for(position).0
    }

    fn place_for(&self, position: Point<Pixels>) -> (usize, bool) {
        match (self.content_point(position), &self.shaped) {
            (Some(at), Some(shaped)) => shaped.place(&self.value, at),
            _ => (self.value.len(), false),
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.dragging = true;
        self.goal_x = None;
        let (at, upstream) = self.place_for(event.position);
        self.press = (!event.modifiers.shift).then_some(event.position);
        if event.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
        self.upstream = upstream;
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
        self.open_preview_link(event.position, cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            let (at, upstream) = self.place_for(event.position);
            self.select_to(at, cx);
            self.upstream = upstream;
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        let max = self.shaped.as_ref().map_or(px(0.), |s| (s.height - s.bounds.size.height).max(px(0.)));
        let scroll = (self.scroll - delta.y).clamp(px(0.), max);
        if self.scroll != scroll {
            self.scroll = scroll;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn range16(&self, range: &Range<usize>) -> Range<usize> {
        to_utf16(&self.value, range.start)..to_utf16(&self.value, range.end)
    }

    fn range8(&self, range: &Range<usize>) -> Range<usize> {
        from_utf16(&self.value, range.start)..from_utf16(&self.value, range.end)
    }
}

impl EntityInputHandler for TextArea {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range8(&range);
        actual.replace(self.range16(&range));
        Some(self.value[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        insert: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.range8(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.replace(range, insert, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        insert: &str,
        new_selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.range8(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        let (value, at) = splice(&self.value, range.clone(), insert);
        self.value = value;
        self.marked = (!insert.is_empty()).then(|| range.start..range.start + insert.len());
        self.selection = match new_selection {
            Some(sel) => {
                let inner = from_utf16(insert, sel.start)..from_utf16(insert, sel.end);
                range.start + inner.start..range.start + inner.end
            }
            None => at..at,
        };
        self.reversed = false;
        self.reveal = true;
        self.upstream = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let shaped = self.shaped.as_ref()?;
        let range = self.range8(&range);
        let (start, end) = if range.is_empty() {
            let at = shaped.caret(&self.value, range.start, self.upstream);
            (at, at)
        } else {
            (shaped.position_down(&self.value, range.start), shaped.position(&self.value, range.end))
        };
        let top = bounds.top() - self.scroll;
        let right = if end.y > start.y { bounds.size.width } else { end.x.max(start.x) };
        Some(Bounds::from_corners(
            point(bounds.left() + start.x, top + start.y),
            point(bounds.left() + right, top + start.y + shaped.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.shaped.as_ref()?;
        Some(to_utf16(&self.value, self.offset_for(position)))
    }
}

struct AreaBody {
    area: Entity<TextArea>,
}

struct BodyPaint {
    rows: Vec<Row>,
    scroll: Pixels,
    quads: Vec<gpui::PaintQuad>,
    cursor: Option<gpui::PaintQuad>,
    height: Pixels,
}

impl IntoElement for AreaBody {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

fn shape(
    window: &mut Window,
    area: &TextArea,
    width: Pixels,
    style: &TextStyle,
    theme: &nixa_ui::Theme,
) -> (Vec<Row>, Pixels) {
    let color = style.color;
    let empty = area.value.is_empty();
    let marked = area.marked.clone();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = px(LINE_HEIGHT);
    let shown = if empty {
        area.placeholder.to_string()
    } else {
        area.value.clone()
    };
    let mut rows = Vec::new();
    let mut top = px(0.);
    let mut start = 0;
    for text in shown.split('\n') {
        let end = start + text.len();
        let base = TextRun {
            len: text.len(),
            font: style.font(),
            color: if empty { theme.dim } else { color },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match (&area.preview, marked.clone().filter(|_| !empty)) {
            (Some(spans), _) if !empty => preview::runs(spans, start..end, &base, theme.blue),
            (_, Some(m)) if m.start < end && m.end > start => {
                let a = m.start.max(start) - start;
                let b = m.end.min(end) - start;
                vec![
                    TextRun {
                        len: a,
                        ..base.clone()
                    },
                    TextRun {
                        len: b - a,
                        underline: Some(UnderlineStyle {
                            color: Some(color),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        ..base.clone()
                    },
                    TextRun {
                        len: text.len() - b,
                        ..base
                    },
                ]
                .into_iter()
                .filter(|r| r.len > 0)
                .collect()
            }
            _ => vec![base],
        };
        let line = window
            .text_system()
            .shape_text(
                SharedString::from(text.to_string()),
                font_size,
                &runs,
                Some(width),
                None,
            )
            .ok()
            .and_then(|lines| lines.into_iter().next())
            .unwrap_or_default();
        let rows_in = (line.wrap_boundaries().len() + 1) as f32;
        rows.push(Row {
            start: if empty { 0 } else { start },
            top,
            line,
        });
        top += line_height * rows_in;
        start = end + 1;
    }
    (rows, top)
}

fn selection_quads(
    shaped: &Shaped,
    value: &str,
    range: Range<usize>,
    origin: Point<Pixels>,
    color: gpui::Hsla,
) -> Vec<gpui::PaintQuad> {
    let mut out = Vec::new();
    if range.is_empty() {
        return out;
    }
    let lh = shaped.line_height;
    let width = shaped.bounds.size.width;
    let a = shaped.position_down(value, range.start);
    let b = shaped.position(value, range.end);
    let mut y = a.y;
    while y <= b.y {
        let left = if y == a.y { a.x } else { px(0.) };
        let right = if y == b.y { b.x } else { width };
        let right = if right <= left { left + px(4.) } else { right };
        out.push(fill(
            Bounds::from_corners(
                point(origin.x + left, origin.y + y),
                point(origin.x + right, origin.y + y + lh),
            ),
            color,
        ));
        y += lh;
    }
    out
}

impl Element for AreaBody {
    type RequestLayoutState = ();
    type PrepaintState = BodyPaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if let Some((min, max)) = self.area.read(cx).auto_rows {
            let layout = preview::measured_layout(self.area.clone(), style, min, max, window);
            (layout, ())
        } else {
            style.size.height = relative(1.).into();
            (window.request_layout(style, [], cx), ())
        }
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> BodyPaint {
        let t = theme(cx).clone();
        let area = self.area.read(cx);
        let empty = area.value.is_empty();
        let style = window.text_style();
        let (rows, height) = shape(window, area, bounds.size.width, &style, &t);
        let lh = px(LINE_HEIGHT);
        let probe = Shaped {
            rows,
            bounds,
            line_height: lh,
            height: height.max(lh),
        };
        let cursor_at = if empty {
            point(px(0.), px(0.))
        } else {
            probe.caret(&area.value, area.cursor(), area.upstream)
        };
        let view = bounds.size.height;
        let mut scroll = area.scroll;
        let resized = area.preview.is_none()
            && area.focus.is_focused(window)
            && area.shaped.as_ref().is_some_and(|s| s.bounds.size.width != bounds.size.width);
        if area.reveal || resized {
            if cursor_at.y < scroll {
                scroll = cursor_at.y;
            } else if cursor_at.y + lh > scroll + view {
                scroll = cursor_at.y + lh - view;
            }
        }
        let max = (probe.height - view).max(px(0.));
        scroll = scroll.clamp(px(0.), max);
        let origin = point(bounds.left(), bounds.top() - scroll);
        let quads = if empty {
            Vec::new()
        } else {
            selection_quads(
                &probe,
                &area.value,
                area.selection.clone(),
                origin,
                t.accent.opacity(0.3),
            )
        };
        let cursor = (area.preview.is_none() && area.selection.is_empty()).then(|| {
            fill(
                Bounds::new(
                    point(origin.x + cursor_at.x, origin.y + cursor_at.y),
                    size(px(1.5), lh),
                ),
                t.accent,
            )
        });
        BodyPaint {
            rows: probe.rows,
            scroll,
            quads,
            cursor,
            height: probe.height,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut BodyPaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let area = self.area.read(cx);
        let focus = area.focus.clone();
        if area.preview.is_none() {
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.area.clone()),
                cx,
            );
        }
        let lh = px(LINE_HEIGHT);
        let scroll = state.scroll;
        let rows = std::mem::take(&mut state.rows);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in state.quads.drain(..) {
                window.paint_quad(quad);
            }
            for row in &rows {
                let y = bounds.top() + row.top - scroll;
                let rows_in = (row.line.wrap_boundaries().len() + 1) as f32;
                if y + lh * rows_in < bounds.top() || y > bounds.bottom() {
                    continue;
                }
                row.line
                    .paint(
                        point(bounds.left(), y),
                        lh,
                        gpui::TextAlign::Left,
                        Some(bounds),
                        window,
                        cx,
                    )
                    .ok();
            }
            if focus.is_focused(window)
                && let Some(cursor) = state.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        let height = state.height;
        self.area.update(cx, |area, _| {
            area.scroll = scroll;
            area.reveal = false;
            area.shaped = Some(Shaped {
                rows,
                bounds,
                line_height: lh,
                height,
            });
        });
    }
}

impl Render for TextArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let focused = self.focus.is_focused(window);
        let mono = self.mono;
        let fixed = self.auto_rows.is_none();
        let pad_y = if self.enter_sends { 5. } else { 8. };
        div()
            .w_full()
            .when(fixed, |el| el.h_full())
            .flex()
            .px(px(10.))
            .py(px(pad_y))
            .rounded(px(7.))
            .bg(t.input)
            .border_1()
            .border_color(if focused { t.accent } else { t.border })
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::line_break))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(
                div()
                    .w_full()
                    .when(fixed, |el| el.h_full())
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_size(px(BASE))
                    .line_height(px(LINE_HEIGHT))
                    .text_color(t.foreground)
                    .when(mono, |el| el.font_family(MONO_FONT))
                    .child(AreaBody { area: cx.entity() }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(area: &Entity<TextArea>, vcx: &mut gpui::VisualTestContext) -> (Bounds<Pixels>, Vec<usize>) {
        vcx.update(|_, cx| {
            let shaped = area.read(cx).shaped.as_ref().expect("shaped");
            (shaped.bounds, wrap_starts(&shaped.rows[0].line))
        })
    }

    fn caret_y(area: &Entity<TextArea>, vcx: &mut gpui::VisualTestContext) -> Pixels {
        vcx.update(|_, cx| {
            let a = area.read(cx);
            a.shaped.as_ref().expect("shaped").caret(&a.value, a.cursor(), a.upstream).y
        })
    }

    #[gpui::test]
    fn the_caret_stays_on_the_row_it_was_put_on(cx: &mut gpui::TestAppContext) {
        cx.update(nixa_ui::init);
        let (area, vcx) = cx.add_window_view(|_, cx| TextArea::new(cx, "type"));
        vcx.update(|window, cx| window.focus(&area.read(cx).focus_handle(cx)));
        let mut text = "short ".to_string();
        text.push_str(&"x".repeat(400));
        text.push_str(&" words that wrap onto more rows".repeat(20));
        vcx.simulate_input(&text);
        vcx.simulate_keystrokes("cmd-up");
        vcx.run_until_parked();
        let (bounds, starts) = rows_of(&area, vcx);
        assert!(starts.len() >= 3, "{starts:?}");
        let lh = px(LINE_HEIGHT);
        vcx.simulate_click(point(bounds.right() - px(2.), bounds.top() + lh / 2.), gpui::Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(vcx.update(|_, cx| area.read(cx).cursor()), starts[0]);
        assert_eq!(caret_y(&area, vcx), px(0.));
        vcx.simulate_click(point(bounds.left() + px(1.), bounds.top() + lh * 1.5), gpui::Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(caret_y(&area, vcx), lh);
        vcx.simulate_click(point(bounds.right() - px(2.), bounds.top() + lh / 2.), gpui::Modifiers::none());
        vcx.run_until_parked();
        let mut ys = Vec::new();
        for _ in 0..3 {
            vcx.simulate_keystrokes("down");
            vcx.run_until_parked();
            ys.push(caret_y(&area, vcx));
        }
        assert_eq!(ys, vec![lh, lh * 2., lh * 3.]);
        for _ in 0..3 {
            vcx.simulate_keystrokes("up");
            vcx.run_until_parked();
            ys.push(caret_y(&area, vcx));
        }
        assert_eq!(ys[3..], [lh * 2., lh, px(0.)]);
    }

    #[test]
    fn lines_and_offsets() {
        let text = "ab\n\ncd";
        let starts = line_starts(text);
        assert_eq!(starts, vec![0, 3, 4]);
        assert_eq!(line_of(&starts, 0), 0);
        assert_eq!(line_of(&starts, 2), 0);
        assert_eq!(line_of(&starts, 3), 1);
        assert_eq!(line_of(&starts, 6), 2);
        assert_eq!(line_end(text, &starts, 0), 2);
        assert_eq!(line_end(text, &starts, 1), 3);
        assert_eq!(line_end(text, &starts, 2), 6);
        assert_eq!(clean_paste("a\r\nb\rc"), "a\nb\nc");
    }
}
