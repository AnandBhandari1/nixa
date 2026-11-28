use crate::colors::{Palette, cell_background, cell_foreground};
use crate::metrics::{Metrics, PaintStats, cell_origin, note_paint};
use crate::selection::Selection;
use gpui::{
    App, Bounds, ContentMask, Font, FontStyle, FontWeight, Hsla, Pixels, SharedString,
    StrikethroughStyle, TextRun, UnderlineStyle, Window, fill, outline, point, px, size,
};
use nixa_protocol::{Cell, CursorShape, Snapshot, flags};
use std::sync::Arc;
use std::time::Instant;

pub struct Frame {
    pub snap: Option<Arc<Snapshot>>,
    pub palette: Palette,
    pub font: Font,
    pub metrics: Metrics,
    pub selection: Option<Selection>,
    pub focused: bool,
    pub stats: Arc<PaintStats>,
    pub link: Option<(u16, u16, u16)>,
}

struct Segment {
    col: u16,
    text: String,
    runs: Vec<TextRun>,
    forced: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: u16,
    strike: bool,
}

const UNDERLINES: u16 = flags::UNDERLINE
    | flags::DOUBLE_UNDERLINE
    | flags::UNDERCURL
    | flags::DOTTED_UNDERLINE
    | flags::DASHED_UNDERLINE;

fn cell_style(cell: &Cell, fg: Hsla) -> Style {
    Style {
        fg,
        bold: cell.has(flags::BOLD),
        italic: cell.has(flags::ITALIC),
        underline: cell.flags & UNDERLINES,
        strike: cell.has(flags::STRIKE),
    }
}

fn style_run(base: &Font, s: Style, len: usize) -> TextRun {
    let mut font = base.clone();
    if s.bold {
        font.weight = FontWeight::BOLD;
    }
    if s.italic {
        font.style = FontStyle::Italic;
    }
    let underline = (s.underline != 0).then(|| UnderlineStyle {
        thickness: px(1.0),
        color: Some(s.fg),
        wavy: s.underline & flags::UNDERCURL != 0,
    });
    let strikethrough = s.strike.then(|| StrikethroughStyle {
        thickness: px(1.0),
        color: Some(s.fg),
    });
    TextRun {
        len,
        font,
        color: s.fg,
        background_color: None,
        underline,
        strikethrough,
    }
}

struct Open {
    col: u16,
    text: String,
    styles: Vec<(Style, usize)>,
    forced: bool,
}

fn open(col: u16, forced: bool) -> Open {
    Open {
        col,
        text: String::new(),
        styles: Vec::new(),
        forced,
    }
}

fn push_cell(seg: &mut Open, cell: &Cell, style: Style) {
    let before = seg.text.len();
    seg.text.push(if cell.ch == '\0' { ' ' } else { cell.ch });
    if let Some(z) = &cell.zerowidth {
        seg.text.push_str(z);
    }
    let len = seg.text.len() - before;
    match seg.styles.last_mut() {
        Some((last, n)) if *last == style => *n += len,
        _ => seg.styles.push((style, len)),
    }
}

fn close(seg: Open, base: &Font) -> Segment {
    Segment {
        col: seg.col,
        runs: seg
            .styles
            .iter()
            .map(|(s, len)| style_run(base, *s, *len))
            .collect(),
        text: seg.text,
        forced: seg.forced,
    }
}

fn is_blank(cell: &Cell) -> bool {
    (cell.ch == ' ' || cell.ch == '\0')
        && cell.flags & (UNDERLINES | flags::STRIKE) == 0
        && cell.zerowidth.is_none()
}

fn row_segments(f: &Frame, cells: &[Cell], cursor_col: Option<u16>) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut cur: Option<Open> = None;
    let last = cells
        .iter()
        .rposition(|c| !is_blank(c))
        .map(|i| i + 1)
        .unwrap_or(0);
    for (i, cell) in cells.iter().enumerate().take(last) {
        let col = i as u16;
        if cell.has(flags::WIDE_SPACER) {
            continue;
        }
        let fg = if cursor_col == Some(col) {
            f.palette.background
        } else {
            cell_foreground(&f.palette, cell)
        };
        let style = cell_style(cell, fg);
        if cell.has(flags::WIDE) || cell.zerowidth.is_some() {
            if let Some(s) = cur.take() {
                out.push(close(s, &f.font));
            }
            let mut alone = open(col, false);
            push_cell(&mut alone, cell, style);
            out.push(close(alone, &f.font));
            continue;
        }
        push_cell(cur.get_or_insert_with(|| open(col, true)), cell, style);
    }
    if let Some(s) = cur.take() {
        out.push(close(s, &f.font));
    }
    out
}

fn paint_backgrounds(
    bounds: &Bounds<Pixels>,
    f: &Frame,
    snap: &Snapshot,
    row: u16,
    window: &mut Window,
) {
    let m = &f.metrics;
    let cells = snap.row(row);
    let mut span: Option<(u16, u16, Hsla)> = None;
    let mut prev_bg: Option<Hsla> = None;
    let flush = |span: Option<(u16, u16, Hsla)>, window: &mut Window| {
        if let Some((from, to, color)) = span {
            let o = cell_origin(bounds, m, row, from);
            window.paint_quad(fill(
                Bounds::new(o, size(m.cell_w * (to - from) as f32, m.line_h)),
                color,
            ));
        }
    };
    for (i, cell) in cells.iter().enumerate() {
        let col = i as u16;
        let bg = if cell.has(flags::WIDE_SPACER) {
            prev_bg
        } else {
            cell_background(&f.palette, cell)
        };
        prev_bg = bg;
        match (span, bg) {
            (Some((from, to, c)), Some(b)) if to == col && c == b => {
                span = Some((from, col + 1, c))
            }
            (_, Some(b)) => {
                flush(span.take(), window);
                span = Some((col, col + 1, b));
            }
            (_, None) => flush(span.take(), window),
        }
    }
    flush(span, window);
    if let Some(sel) = &f.selection
        && let Some((from, to)) = sel.row_span(snap, row)
    {
        let o = cell_origin(bounds, m, row, from);
        window.paint_quad(fill(
            Bounds::new(o, size(m.cell_w * (to - from + 1) as f32, m.line_h)),
            f.palette.selection.opacity(0.9),
        ));
    }
}

fn cursor_spot(snap: &Snapshot) -> Option<(u16, u16)> {
    let c = &snap.cursor;
    if !c.visible || c.shape == CursorShape::Hidden {
        return None;
    }
    let row = c.row as usize;
    if row >= snap.rows as usize || c.col >= snap.cols {
        return None;
    }
    Some((row as u16, c.col))
}

fn paint_cursor(
    bounds: &Bounds<Pixels>,
    f: &Frame,
    snap: &Snapshot,
    at: (u16, u16),
    window: &mut Window,
) {
    let m = &f.metrics;
    let o = cell_origin(bounds, m, at.0, at.1);
    let wide = snap
        .row(at.0)
        .get(at.1 as usize)
        .is_some_and(|c| c.has(flags::WIDE));
    let w = if wide { m.cell_w * 2.0 } else { m.cell_w };
    let color = f.palette.cursor;
    let shape = if f.focused {
        snap.cursor.shape
    } else {
        CursorShape::HollowBlock
    };
    let quad = match shape {
        CursorShape::Block => fill(Bounds::new(o, size(w, m.line_h)), color),
        CursorShape::Underline => fill(
            Bounds::new(point(o.x, o.y + m.line_h - px(2.0)), size(w, px(2.0))),
            color,
        ),
        CursorShape::Bar => fill(Bounds::new(o, size(px(2.0), m.line_h)), color),
        CursorShape::HollowBlock => {
            outline(Bounds::new(o, size(w, m.line_h)), color, Default::default())
        }
        CursorShape::Hidden => return,
    };
    window.paint_quad(quad);
}

pub fn paint_frame(bounds: Bounds<Pixels>, f: &Frame, window: &mut Window, cx: &mut App) {
    let began = Instant::now();
    paint_cells(bounds, f, window, cx);
    note_paint(&f.stats, began.elapsed());
}

fn paint_cells(bounds: Bounds<Pixels>, f: &Frame, window: &mut Window, cx: &mut App) {
    window.paint_quad(fill(bounds, f.palette.background));
    let Some(snap) = f.snap.as_deref() else {
        return;
    };
    let m = f.metrics;
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let cursor = cursor_spot(snap);
        let block = cursor.filter(|_| f.focused && snap.cursor.shape == CursorShape::Block);
        for row in 0..snap.rows {
            paint_backgrounds(&bounds, f, snap, row, window);
        }
        if let Some(at) = cursor {
            paint_cursor(&bounds, f, snap, at, window);
        }
        for row in 0..snap.rows {
            let cursor_col = block.filter(|b| b.0 == row).map(|b| b.1);
            for seg in row_segments(f, snap.row(row), cursor_col) {
                let force = seg.forced.then_some(m.cell_w);
                let line = window.text_system().shape_line(
                    SharedString::from(seg.text),
                    m.font_size,
                    &seg.runs,
                    force,
                );
                let o = cell_origin(&bounds, &m, row, seg.col);
                line.paint(o, m.line_h, window, cx).ok();
            }
        }
        if let Some((row, from, to)) = f.link
            && row < snap.rows
            && to > from
        {
            let o = cell_origin(&bounds, &m, row, from);
            window.paint_quad(fill(
                Bounds::new(
                    point(o.x, o.y + m.line_h - px(1.0)),
                    size(m.cell_w * (to - from) as f32, px(1.0)),
                ),
                f.palette.foreground,
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;
    use gpui::font;
    use nixa_protocol::Color;

    fn frame() -> Frame {
        Frame {
            snap: None,
            palette: Palette::default(),
            font: font("Menlo"),
            metrics: Metrics {
                cell_w: px(8.0),
                line_h: px(18.0),
                font_size: px(14.0),
            },
            selection: None,
            focused: true,
            stats: Arc::new(PaintStats::default()),
            link: None,
        }
    }

    fn cell(ch: char, fg: Color, flags: u16) -> Cell {
        Cell {
            ch,
            fg,
            flags,
            ..Cell::default()
        }
    }

    #[test]
    fn same_style_cells_share_one_run() {
        let f = frame();
        let mut row: Vec<Cell> = "ab"
            .chars()
            .map(|c| cell(c, Color::Indexed(1), 0))
            .collect();
        row.extend(
            "cd".chars()
                .map(|c| cell(c, Color::Indexed(1), flags::BOLD)),
        );
        row.push(cell('e', Color::Default, 0));
        row.extend((0..5).map(|_| Cell::default()));
        let segs = row_segments(&f, &row, None);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "abcde");
        let lens: Vec<usize> = segs[0].runs.iter().map(|r| r.len).collect();
        assert_eq!(lens, vec![2, 2, 1]);
        assert_eq!(segs[0].runs[1].font.weight, FontWeight::BOLD);
        assert_eq!(segs[0].runs[0].font.weight, f.font.weight);
        assert!(segs[0].forced);
    }

    #[test]
    fn wide_cells_stand_alone_and_cursor_flips_the_color() {
        let f = frame();
        let row = vec![
            cell('a', Color::Default, 0),
            cell('日', Color::Default, flags::WIDE),
            cell(' ', Color::Default, flags::WIDE_SPACER),
            cell('b', Color::Default, flags::UNDERLINE),
        ];
        let segs = row_segments(&f, &row, Some(3));
        let cols: Vec<u16> = segs.iter().map(|s| s.col).collect();
        assert_eq!(cols, vec![0, 1, 3]);
        assert!(!segs[1].forced);
        assert_eq!(segs[2].runs[0].color, f.palette.background);
        assert!(segs[2].runs[0].underline.is_some());
    }

    #[test]
    fn a_blank_row_has_no_segments() {
        let f = frame();
        let row: Vec<Cell> = (0..10).map(|_| Cell::default()).collect();
        assert!(row_segments(&f, &row, None).is_empty());
    }
}
