use crate::icons::{Icon, icon};
use crate::text;
use crate::theme::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point, Render,
    ShapedLine, SharedString, Style, Styled, TextRun, UTF16Selection, UnderlineStyle, Window,
    actions, div, fill, point, prelude::FluentBuilder, px, relative, size,
};
use std::ops::Range;

actions!(
    nixa_text_input,
    [
        Backspace, Delete, Left, Right, SelectLeft, SelectRight, SelectAll, Home, End, Paste, Copy,
        Cut, Submit, Cancel
    ]
);

pub const KEY_CONTEXT: &str = "NixaTextInput";
pub const MASK: char = '•';

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("cmd-left", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("cmd-right", End, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("cmd-x", Cut, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("escape", Cancel, ctx),
    ]);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed(String),
    Submit(String),
    Cancel,
}

pub fn prev_char(value: &str, offset: usize) -> usize {
    value[..offset.min(value.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(i, _)| i)
}

pub fn next_char(value: &str, offset: usize) -> usize {
    let offset = offset.min(value.len());
    value[offset..]
        .chars()
        .next()
        .map_or(value.len(), |c| offset + c.len_utf8())
}

pub fn to_utf16(value: &str, offset: usize) -> usize {
    value[..offset.min(value.len())].encode_utf16().count()
}

pub fn from_utf16(value: &str, offset16: usize) -> usize {
    let mut count = 0;
    for (i, c) in value.char_indices() {
        if count >= offset16 {
            return i;
        }
        count += c.len_utf16();
    }
    value.len()
}

pub fn mask_index(value: &str, offset: usize) -> usize {
    value[..offset.min(value.len())].chars().count() * MASK.len_utf8()
}

pub fn unmask_index(value: &str, shown: usize) -> usize {
    let n = shown / MASK.len_utf8();
    value.char_indices().nth(n).map_or(value.len(), |(i, _)| i)
}

pub fn splice(value: &str, range: Range<usize>, insert: &str) -> (String, usize) {
    let start = range.start.min(value.len());
    let end = range.end.clamp(start, value.len());
    let mut out = String::with_capacity(value.len() + insert.len());
    out.push_str(&value[..start]);
    out.push_str(insert);
    out.push_str(&value[end..]);
    (out, start + insert.len())
}

pub struct TextInput {
    focus: FocusHandle,
    value: String,
    placeholder: SharedString,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    line: Option<ShapedLine>,
    line_bounds: Option<Bounds<Pixels>>,
    dragging: bool,
    leading: Option<Icon>,
    masked: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        TextInput {
            focus: cx.focus_handle(),
            value: String::new(),
            placeholder: placeholder.into(),
            selection: 0..0,
            reversed: false,
            marked: None,
            line: None,
            line_bounds: None,
            dragging: false,
            leading: None,
            masked: false,
        }
    }

    pub fn with_icon(mut self, which: Icon) -> Self {
        self.leading = Some(which);
        self
    }

    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    fn shown_at(&self, offset: usize) -> usize {
        if self.masked { mask_index(&self.value, offset) } else { offset }
    }

    fn value_at(&self, shown: usize) -> usize {
        if self.masked { unmask_index(&self.value, shown) } else { shown }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        self.value = value.into();
        let end = self.value.len();
        self.selection = end..end;
        self.marked = None;
        cx.notify();
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, insert: &str, cx: &mut Context<Self>) {
        let (value, at) = splice(&self.value, range, insert);
        self.value = value;
        self.selection = at..at;
        self.reversed = false;
        self.marked = None;
        cx.emit(InputEvent::Changed(self.value.clone()));
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selection.is_empty() { prev_char(&self.value, self.cursor()) } else { self.selection.start };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selection.is_empty() { next_char(&self.value, self.cursor()) } else { self.selection.end };
        self.move_to(to, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_char(&self.value, self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_char(&self.value, self.cursor()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_value(cx);
    }

    pub fn select_value(&mut self, cx: &mut Context<Self>) {
        self.selection = 0..self.value.len();
        self.reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
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
            let clean = clip.replace(['\n', '\r'], " ");
            self.replace(self.selection.clone(), &clean, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.value[self.selection.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.value[self.selection.clone()].to_string()));
            self.replace(self.selection.clone(), "", cx);
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit(self.value.clone()));
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    fn offset_at(&self, position: Point<Pixels>) -> usize {
        match (&self.line, &self.line_bounds) {
            (Some(line), Some(bounds)) if !self.value.is_empty() => {
                if position.y < bounds.top() {
                    0
                } else if position.y > bounds.bottom() {
                    self.value.len()
                } else {
                    self.value_at(line.closest_index_for_x(position.x - bounds.left()))
                }
            }
            _ => 0,
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.dragging = true;
        let at = self.offset_at(event.position);
        if event.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.dragging = false;
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            self.select_to(self.offset_at(event.position), cx);
        }
    }

    fn range16(&self, range: &Range<usize>) -> Range<usize> {
        to_utf16(&self.value, range.start)..to_utf16(&self.value, range.end)
    }

    fn range8(&self, range: &Range<usize>) -> Range<usize> {
        from_utf16(&self.value, range.start)..from_utf16(&self.value, range.end)
    }
}

impl EntityInputHandler for TextInput {
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
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.line.as_ref()?;
        let range = self.range8(&range);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(self.shown_at(range.start)), bounds.top()),
            point(bounds.left() + line.x_for_index(self.shown_at(range.end)), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.line_bounds?;
        let line = self.line.as_ref()?;
        let index = self.value_at(line.index_for_x(position.x - bounds.left())?);
        Some(to_utf16(&self.value, index))
    }
}

struct InputLine {
    input: Entity<TextInput>,
}

struct LinePaint {
    line: ShapedLine,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for InputLine {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for InputLine {
    type RequestLayoutState = ();
    type PrepaintState = LinePaint;

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
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> LinePaint {
        let t = theme(cx).clone();
        let input = self.input.read(cx);
        let style = window.text_style();
        let empty = input.value.is_empty();
        let shown: SharedString = if empty {
            input.placeholder.clone()
        } else if input.masked {
            MASK.to_string().repeat(input.value.chars().count()).into()
        } else {
            input.value.clone().into()
        };
        let color = if empty { t.dim } else { style.color };
        let base = TextRun {
            len: shown.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked.clone() {
            Some(marked) if !empty => {
                let (start, end) = (input.shown_at(marked.start), input.shown_at(marked.end));
                vec![
                    TextRun { len: start, ..base.clone() },
                    TextRun {
                        len: end - start,
                        underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.0), wavy: false }),
                        ..base.clone()
                    },
                    TextRun { len: shown.len() - end, ..base },
                ]
                .into_iter()
                .filter(|run| run.len > 0)
                .collect()
            }
            _ => vec![base],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(shown, font_size, &runs, None);
        let cursor_at = if empty { 0 } else { input.shown_at(input.cursor()) };
        let cursor_x = line.x_for_index(cursor_at);
        let (selection, cursor) = if input.selection.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(point(bounds.left() + cursor_x, bounds.top()), size(px(1.5), bounds.size.height)),
                    t.accent,
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(bounds.left() + line.x_for_index(input.shown_at(input.selection.start)), bounds.top()),
                        point(bounds.left() + line.x_for_index(input.shown_at(input.selection.end)), bounds.bottom()),
                    ),
                    t.accent.opacity(0.3),
                )),
                None,
            )
        };
        LinePaint { line, cursor, selection }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut LinePaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        if let Some(selection) = state.selection.take() {
            window.paint_quad(selection);
        }
        let line = std::mem::take(&mut state.line);
        line.paint(bounds.origin, window.line_height(), window, cx).ok();
        if focus.is_focused(window)
            && let Some(cursor) = state.cursor.take()
        {
            window.paint_quad(cursor);
        }
        self.input.update(cx, |input, _| {
            input.line = Some(line);
            input.line_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let focused = self.focus.is_focused(window);
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(30.0))
            .px(px(10.0))
            .w_full()
            .rounded(px(7.0))
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
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .when_some(self.leading, |el, which| el.child(icon(which).size(px(14.0)).text_color(t.dim)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(px(text::BASE))
                    .line_height(px(18.0))
                    .text_color(t.foreground)
                    .child(InputLine { input: cx.entity() }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_indexes_follow_chars() {
        let s = "aé😀b";
        assert_eq!(mask_index(s, 0), 0);
        assert_eq!(mask_index(s, 3), 2 * MASK.len_utf8());
        assert_eq!(mask_index(s, s.len()), 4 * MASK.len_utf8());
        assert_eq!(unmask_index(s, 2 * MASK.len_utf8()), 3);
        assert_eq!(unmask_index(s, 99), s.len());
    }

    #[test]
    fn char_steps_respect_multibyte() {
        let s = "aé😀b";
        assert_eq!(next_char(s, 0), 1);
        assert_eq!(next_char(s, 1), 3);
        assert_eq!(next_char(s, 3), 7);
        assert_eq!(prev_char(s, 7), 3);
        assert_eq!(prev_char(s, 0), 0);
        assert_eq!(next_char(s, s.len()), s.len());
    }

    #[test]
    fn utf16_round_trip() {
        let s = "aé😀b";
        assert_eq!(to_utf16(s, 7), 4);
        assert_eq!(from_utf16(s, 4), 7);
        assert_eq!(from_utf16(s, 99), s.len());
        for i in [0, 1, 3, 7, 8] {
            assert_eq!(from_utf16(s, to_utf16(s, i)), i);
        }
    }

    #[test]
    fn splice_replaces_and_moves_cursor() {
        assert_eq!(splice("hello", 1..3, "EE"), ("hEElo".to_string(), 3));
        assert_eq!(splice("hi", 2..2, "!"), ("hi!".to_string(), 3));
        assert_eq!(splice("hi", 5..9, "x"), ("hix".to_string(), 3));
    }
}
