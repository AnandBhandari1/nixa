use super::{LINE_HEIGHT, TextArea, shape};
use gpui::{
    AvailableSpace, Context, Entity, FontStyle, FontWeight, LayoutId, Pixels, Point, Style,
    TextRun, UnderlineStyle, Window, point, px, size,
};
use nixa_ui::text::{BASE, MONO_FONT};
use nixa_ui::theme;
use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewSpan {
    pub range: Range<usize>,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: Option<String>,
}

impl TextArea {
    pub fn auto_rows(mut self, min: u16, max: u16) -> Self {
        self.auto_rows = Some((min, max));
        self
    }

    pub fn preview(mut self) -> Self {
        self.preview = Some(Vec::new());
        self
    }

    pub fn set_preview(&mut self, text: String, spans: Vec<PreviewSpan>, cx: &mut Context<Self>) {
        if self.value != text || self.preview.as_ref() != Some(&spans) {
            self.preview = Some(spans);
            self.set_value(text, cx);
        }
    }

    pub(super) fn open_preview_link(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(start) = self.press.take() else {
            return;
        };
        if (start.x - at.x).abs() > px(3.) || (start.y - at.y).abs() > px(3.) {
            return;
        }
        if self.preview.is_none() {
            return;
        }
        let Some(shaped) = &self.shaped else { return };
        if !shaped.bounds.contains(&at) {
            return;
        }
        let Some(at) = self.content_point(at) else {
            return;
        };
        let Some(row) = shaped.rows.get(shaped.row_at(at.y)) else {
            return;
        };
        let Ok(index) = row
            .line
            .index_for_position(point(at.x, at.y - row.top), shaped.line_height)
        else {
            return;
        };
        let offset = row.start + index;
        if let Some(url) = self.preview.as_ref().and_then(|spans| {
            spans
                .iter()
                .find(|span| span.range.contains(&offset))
                .and_then(|span| span.link.as_ref())
        }) {
            cx.open_url(url);
        }
    }
}

pub(super) fn runs(
    spans: &[PreviewSpan],
    line: Range<usize>,
    base: &TextRun,
    link_color: gpui::Hsla,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut start = line.start;
    let first = spans.partition_point(|span| span.range.end <= start);
    for span in spans[first..]
        .iter()
        .take_while(|span| span.range.start < line.end)
    {
        let a = span.range.start.max(start);
        let b = span.range.end.min(line.end);
        if a > start {
            runs.push(TextRun {
                len: a - start,
                ..base.clone()
            });
        }
        let mut run = TextRun {
            len: b - a,
            ..base.clone()
        };
        if span.bold {
            run.font.weight = FontWeight::SEMIBOLD;
        }
        if span.italic {
            run.font.style = FontStyle::Italic;
        }
        if span.code {
            run.font.family = MONO_FONT.into();
        }
        if span.link.is_some() {
            run.color = link_color;
            run.underline = Some(UnderlineStyle {
                color: Some(link_color),
                thickness: px(1.),
                wavy: false,
            });
        }
        runs.push(run);
        start = b;
    }
    if start < line.end || runs.is_empty() {
        runs.push(TextRun {
            len: line.end - start,
            ..base.clone()
        });
    }
    runs
}

pub(super) fn measured_layout(
    area: Entity<TextArea>,
    style: Style,
    min: u16,
    max: u16,
    window: &mut Window,
) -> LayoutId {
    let mut text_style = window.text_style();
    text_style.font_size = px(BASE).into();
    window.request_measured_layout(style, move |known, available, window, cx| {
        let width = known.width.unwrap_or_else(|| match available.width {
            AvailableSpace::Definite(width) => width,
            _ => px(600.),
        });
        let area = area.read(cx);
        let (_, content) = shape(window, area, width.max(px(1.)), &text_style, theme(cx));
        size(
            width,
            content.clamp(
                px(LINE_HEIGHT * f32::from(min)),
                px(LINE_HEIGHT * f32::from(max)),
            ),
        )
    })
}
