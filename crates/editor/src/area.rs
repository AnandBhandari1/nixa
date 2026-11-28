use crate::syntax::{self, Tok};
use crate::view::Editor;
use gpui::{
    App, Bounds, ContentMask, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, TextRun,
    Window, fill, point, px, relative, size,
};
use nixa_ui::{Theme, theme};

pub const FONT_SIZE: f32 = 13.0;
pub const LINE_H: f32 = 19.0;
pub const TAB: usize = 4;
pub const MAX_SHOWN: usize = 20_000;
const SCROLLBAR_W: f32 = 9.0;

pub struct LineLayout {
    pub line: usize,
    pub shaped: ShapedLine,
    pub map: Vec<usize>,
    pub shown: usize,
    pub y: Pixels,
}

pub struct Layout {
    pub bounds: Bounds<Pixels>,
    pub scrollbar: Option<Scrollbar>,
    pub text_left: Pixels,
    pub line_h: Pixels,
    pub cw: Pixels,
    pub lines: Vec<LineLayout>,
}

#[derive(Clone, Copy)]
pub struct Scrollbar {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    max: Pixels,
}

impl Scrollbar {
    fn new(bounds: Bounds<Pixels>, max: Pixels, y: Pixels) -> Option<Self> {
        let (top, height) = nixa_ui::scroll::thumb(bounds.size.height.into(), max.into(), (-y).into())?;
        if px(height) >= bounds.size.height {
            return None;
        }
        let track = Bounds::new(point(bounds.right() - px(SCROLLBAR_W), bounds.top()), size(px(SCROLLBAR_W), bounds.size.height));
        let thumb = Bounds::new(point(track.left() + px(2.0), track.top() + px(top)), size(px(5.0), px(height)));
        Some(Self { track, thumb, max })
    }

    pub fn offset(self, y: Pixels, grab: Pixels) -> Pixels {
        let travel = self.track.size.height - self.thumb.size.height;
        self.max * ((y - self.track.top() - grab) / travel).clamp(0.0, 1.0)
    }
}

pub struct TextArea {
    pub editor: Entity<Editor>,
}

pub struct Paint {
    quads: Vec<PaintQuad>,
    lines: Vec<LineLayout>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    cursor: Option<PaintQuad>,
    gutter: Bounds<Pixels>,
    text: Bounds<Pixels>,
    text_left: Pixels,
    line_h: Pixels,
    cw: Pixels,
    background: Hsla,
    scrollbar: Option<Scrollbar>,
    scroll_color: Hsla,
}

pub fn display(line: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(line.len());
    let mut map = Vec::with_capacity(line.len() + 1);
    let mut col = 0usize;
    for c in line.chars() {
        map.push(out.len());
        if c == '\t' {
            let n = TAB - col % TAB;
            out.push_str(&" ".repeat(n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
        for _ in 1..c.len_utf8() {
            map.push(out.len());
        }
    }
    map.push(out.len());
    (out, map)
}

pub fn display_col(line: &str, upto: usize) -> usize {
    let mut col = 0usize;
    for c in line[..upto.min(line.len())].chars() {
        col += if c == '\t' { TAB - col % TAB } else { 1 };
    }
    col
}

pub fn to_src(map: &[usize], d: usize) -> usize {
    map.partition_point(|m| *m <= d).saturating_sub(1)
}

pub fn cut(line: &str) -> &str {
    if line.len() <= MAX_SHOWN {
        return line;
    }
    let mut end = MAX_SHOWN;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

pub fn tok_color(t: &Theme, tok: Tok) -> Hsla {
    match tok {
        Tok::Keyword => t.magenta,
        Tok::Str => t.green,
        Tok::Comment => t.dim,
        Tok::Number => t.orange,
        Tok::Type => t.yellow,
        Tok::Func => t.blue,
        Tok::Tag => t.red,
        Tok::Attr => t.cyan,
        Tok::Heading => t.blue,
        Tok::Added => t.green,
        Tok::Removed => t.red,
    }
}

fn runs(len: usize, toks: &syntax::Toks, map: &[usize], t: &Theme, font: &gpui::Font) -> Vec<TextRun> {
    let run = |len: usize, color: Hsla| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut out = Vec::new();
    let mut at = 0usize;
    for (r, tok) in toks {
        let s = map[r.start.min(map.len() - 1)].max(at);
        let e = map[r.end.min(map.len() - 1)];
        if e <= s {
            continue;
        }
        if s > at {
            out.push(run(s - at, t.foreground));
        }
        out.push(run(e - s, tok_color(t, *tok)));
        at = e;
    }
    if at < len {
        out.push(run(len - at, t.foreground));
    }
    out
}

impl IntoElement for TextArea {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextArea {
    type RequestLayoutState = ();
    type PrepaintState = Paint;

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
        style.size.height = relative(1.).into();
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
    ) -> Paint {
        let t = theme(cx).clone();
        let font = gpui::font(nixa_ui::text::MONO_FONT);
        let font_size = px(FONT_SIZE);
        let line_h = px(LINE_H);
        let probe = TextRun { len: 1, font: font.clone(), color: t.foreground, background_color: None, underline: None, strikethrough: None };
        let cw = window.text_system().shape_line(SharedString::from("0"), font_size, &[probe], None).width;
        let cw = if cw > px(0.0) { cw } else { px(8.0) };
        let ed = self.editor.read(cx);
        let buf = &ed.buf;
        let count = buf.line_count();
        let digits = count.to_string().len().max(3);
        let gutter_w = cw * digits as f32 + px(28.0);
        let text_left_base = bounds.left() + gutter_w + px(8.0);
        let h = bounds.size.height;
        let max_y = (line_h * (count + 2) as f32 - h).max(px(0.0));
        let inset = if Scrollbar::new(bounds, max_y, px(0.0)).is_some() { px(SCROLLBAR_W) } else { px(0.0) };
        let text_w = (bounds.size.width - gutter_w - px(8.0) - inset).max(px(10.0));
        let mut scroll = ed.scroll;
        let head = buf.sel.head;
        let caret_line = buf.line_of(head);
        if ed.autoscroll {
            let y = line_h * caret_line as f32;
            if y < scroll.y {
                scroll.y = y;
            } else if y + line_h > scroll.y + h {
                scroll.y = y + line_h - h;
            }
            let ls = buf.line_start(caret_line);
            let x = cw * display_col(buf.line_text(caret_line), head - ls) as f32;
            if x < scroll.x {
                scroll.x = (x - cw * 4.0).max(px(0.0));
            } else if x + cw * 2.0 > scroll.x + text_w {
                scroll.x = x + cw * 2.0 - text_w;
            }
        }
        scroll.y = scroll.y.clamp(px(0.0), max_y);
        scroll.x = scroll.x.max(px(0.0));
        let first = (f32::from(scroll.y) / LINE_H).floor() as usize;
        let last = (first + (f32::from(h) / LINE_H).ceil() as usize + 1).min(count);
        let sel = buf.sel.range();
        let focused = ed.focus.is_focused(window);
        let sel_color = if focused { t.accent.opacity(0.28) } else { t.selected };
        let blocks = ed.blocks.as_ref().map(|(_, b)| b.clone());
        let mut quads = Vec::new();
        let mut lines = Vec::new();
        let mut numbers = Vec::new();
        let mut cursor = None;
        let mut widest = px(0.0);
        let text_left = text_left_base - scroll.x;
        for line in first..last {
            let y = bounds.top() + line_h * line as f32 - scroll.y;
            let ls = buf.line_start(line);
            let le = buf.line_end(line);
            let full = buf.line_text(line);
            let shown_text = cut(full);
            let (disp, map) = display(shown_text);
            let state = blocks.as_ref().and_then(|b| b.get(line).copied()).unwrap_or(false);
            let toks = syntax::line(shown_text, &ed.lang, state).0;
            let disp_len = disp.len();
            let r = runs(disp_len, &toks, &map, &t, &font);
            let shaped = window.text_system().shape_line(SharedString::from(disp), font_size, &r, None);
            widest = widest.max(shaped.width);
            let x_of = |off: usize| text_left + shaped.x_for_index(map[(off.saturating_sub(ls)).min(shown_text.len())]);
            if line == caret_line && sel.is_empty() {
                quads.push(fill(Bounds::new(point(bounds.left(), y), size(bounds.size.width, line_h)), t.surface.opacity(0.6)));
            }
            for m in ed.found.iter().filter(|m| m.start <= le && m.end >= ls && m.start >= ls) {
                let current = ed.found_at.and_then(|i| ed.found.get(i)) == Some(m);
                let color = if current { t.accent.opacity(0.45) } else { t.yellow.opacity(0.25) };
                quads.push(fill(Bounds::from_corners(point(x_of(m.start), y), point(x_of(m.end.min(le)), y + line_h)), color));
            }
            if !sel.is_empty() && sel.start < le + 1 && sel.end > ls {
                let s = sel.start.max(ls);
                let e = sel.end.min(le);
                let extra = if sel.end > le { cw } else { px(0.0) };
                quads.push(fill(Bounds::from_corners(point(x_of(s), y), point(x_of(e) + extra, y + line_h)), sel_color));
            }
            if line == caret_line && focused {
                cursor = Some(fill(Bounds::new(point(x_of(head), y), size(px(2.0), line_h)), t.accent));
            }
            let num = (line + 1).to_string();
            let color = if line == caret_line { t.muted } else { t.dim };
            let nrun = TextRun { len: num.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
            let nshape = window.text_system().shape_line(SharedString::from(num), font_size, &[nrun], None);
            let nx = bounds.left() + gutter_w - px(12.0) - nshape.width;
            numbers.push((nshape, point(nx, y)));
            lines.push(LineLayout { line, shaped, map, shown: shown_text.len(), y });
        }
        let max_x = (widest + cw * 4.0 - text_w).max(px(0.0));
        if scroll.x > max_x && !ed.autoscroll {
            scroll.x = max_x;
        }
        let page = ((f32::from(h) / LINE_H).floor() as usize).max(1);
        let changed = scroll != ed.scroll || ed.autoscroll || ed.page != page;
        if changed {
            self.editor.update(cx, |ed, _| {
                ed.scroll = scroll;
                ed.autoscroll = false;
                ed.page = page;
            });
        }
        let gutter = Bounds::new(bounds.origin, size(gutter_w, bounds.size.height));
        let text = Bounds::new(point(bounds.left() + gutter_w, bounds.top()), size((bounds.size.width - gutter_w - inset).max(px(0.0)), bounds.size.height));
        let scrollbar = Scrollbar::new(bounds, max_y, scroll.y);
        Paint { quads, lines, numbers, cursor, gutter, text, text_left, line_h, cw, background: t.background, scrollbar, scroll_color: t.border_strong }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut Paint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let line_h = state.line_h;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, state.background));
            window.with_content_mask(Some(ContentMask { bounds: state.text }), |window| {
                for q in state.quads.drain(..) {
                    window.paint_quad(q);
                }
                for l in &state.lines {
                    l.shaped.paint(point(state.text_left, l.y), line_h, window, cx).ok();
                }
                if focus.is_focused(window) && let Some(c) = state.cursor.take() {
                    window.paint_quad(c);
                }
            });
            window.paint_quad(fill(state.gutter, state.background));
            for (n, at) in &state.numbers {
                n.paint(*at, line_h, window, cx).ok();
            }
            if let Some(scrollbar) = state.scrollbar {
                window.paint_quad(fill(scrollbar.track, state.scroll_color.opacity(0.15)));
                window.paint_quad(fill(scrollbar.thumb, state.scroll_color).corner_radii(px(2.5)));
            }
        });
        let layout = Layout {
            bounds,
            scrollbar: state.scrollbar,
            text_left: state.text_left,
            line_h,
            cw: state.cw,
            lines: std::mem::take(&mut state.lines),
        };
        self.editor.update(cx, |ed, _| ed.layout = Some(layout));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_and_map_back() {
        let (d, map) = display("a\tb");
        assert_eq!(d, "a   b");
        assert_eq!(map, vec![0, 1, 4, 5]);
        assert_eq!(to_src(&map, 2), 1);
        assert_eq!(to_src(&map, 4), 2);
        let (d, map) = display("é\t");
        assert_eq!(d, "é   ");
        assert_eq!(map, vec![0, 2, 2, 5]);
        assert_eq!(to_src(&map, 0), 0);
        assert_eq!(display_col("\tx", 2), 5);
    }

    #[test]
    fn long_lines_are_cut_on_a_char() {
        let s = "é".repeat(MAX_SHOWN);
        let c = cut(&s);
        assert!(c.len() <= MAX_SHOWN);
        assert!(s.is_char_boundary(c.len()));
    }
}
