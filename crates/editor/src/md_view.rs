use crate::markdown::{Block, Inline, Link, Mark, Style};
use crate::mermaid::{self, Previews};
use crate::view::Editor;
use gpui::{
    AnyElement, Context, Div, FontStyle, FontWeight, HighlightStyle, InteractiveElement, InteractiveText,
    IntoElement, ObjectFit, ParentElement, SharedString, StrikethroughStyle, Styled, StyledImage,
    UnderlineStyle, WeakEntity, div, img, prelude::FluentBuilder, px,
};
use nixa_ui::Theme;
use std::path::PathBuf;

mod selection;
pub(super) use selection::Selection;
use selection::Fragments;

type Styles = Vec<(std::ops::Range<usize>, HighlightStyle)>;
type Links = Vec<(std::ops::Range<usize>, String)>;

fn flatten(inline: &Inline, t: &Theme) -> (Styles, Links) {
    let mut cuts: Vec<usize> = vec![0, inline.text.len()];
    for s in &inline.spans {
        cuts.push(s.range.start);
        cuts.push(s.range.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut styles = Vec::new();
    let mut links: Links = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let mut h = HighlightStyle::default();
        let mut any = false;
        for s in inline.spans.iter().filter(|s| s.range.start <= a && s.range.end >= b) {
            any = true;
            match &s.style {
                Style::Bold => h.font_weight = Some(FontWeight::BOLD),
                Style::Italic => h.font_style = Some(FontStyle::Italic),
                Style::Code => {
                    h.background_color = Some(t.raised);
                    h.color = Some(t.orange);
                }
                Style::Strike => h.strikethrough = Some(StrikethroughStyle { thickness: px(1.0), color: None }),
                Style::Link(url) => {
                    h.color = Some(t.accent);
                    h.underline = Some(UnderlineStyle { thickness: px(1.0), color: Some(t.accent), wavy: false });
                    match links.last_mut() {
                        Some((r, u)) if r.end == a && u == url => r.end = b,
                        _ => links.push((a..b, url.clone())),
                    }
                }
            }
        }
        if any {
            styles.push((a..b, h));
        }
    }
    (styles, links)
}

struct PreviewText<'a> {
    fragments: Fragments,
    range: std::ops::Range<usize>,
    theme: &'a Theme,
    editor: WeakEntity<Editor>,
}

impl PreviewText<'_> {
    fn styled(&mut self, text: &str, styles: Styles) -> gpui::StyledText {
        self.fragments.styled(text, styles, &self.range, self.theme.accent.opacity(0.36))
    }

    fn rich(&mut self, id: SharedString, inline: &Inline) -> AnyElement {
        let (styles, links) = flatten(inline, self.theme);
        let text = self.styled(&inline.text, styles);
        if links.is_empty() {
            return text.into_any_element();
        }
        let ranges = links.iter().map(|(r, _)| r.clone()).collect();
        let me = self.editor.clone();
        InteractiveText::new(id, text)
            .on_click(ranges, move |ix, window, cx| {
                if let Some((_, url)) = links.get(ix) {
                    me.update(cx, |ed, cx| {
                        if ed.md_selection.link_click(window.mouse_position()) {
                            ed.follow_link(url, cx);
                        }
                    }).ok();
                }
            })
            .into_any_element()
    }
}

pub fn plain_text(blocks: &[Block]) -> String {
    let row = |cells: &[Inline]| cells.iter().map(|cell| cell.text.as_str()).collect::<Vec<_>>().join("\t");
    blocks.iter().map(|block| match block {
        Block::Heading(_, text) | Block::Para(text) | Block::Quote(text) => text.text.clone(),
        Block::Item { text, .. } => text.text.clone(),
        Block::Code { text, .. } => text.clone(),
        Block::Image { alt, .. } => alt.clone(),
        Block::Rule => String::new(),
        Block::Table { head, rows } => std::iter::once(row(head)).chain(rows.iter().map(|cells| row(cells))).collect::<Vec<_>>().join("\n"),
    }).collect::<Vec<_>>().join("\n\n")
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 22.0,
        2 => 18.0,
        3 => 15.5,
        _ => 13.5,
    }
}

fn code_block(t: &Theme, caption: &str, body: impl IntoElement) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(10.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .when(!caption.is_empty(), |el| el.child(div().text_size(px(11.0)).text_color(t.dim).child(caption.to_owned())))
        .child(
            div()
                .font_family(nixa_ui::text::MONO_FONT)
                .text_size(px(12.0))
                .text_color(t.foreground)
                .child(body),
        )
}

pub(super) fn diagram(t: &Theme, source: &str, index: usize, previews: Option<&Previews>) -> Div {
    let status = match previews.and_then(|p| p.get(&index)) {
        Some(Ok(picture)) => {
            let (w, h) = picture.size;
            return div().w_full().flex().justify_center().overflow_hidden().rounded(px(6.0)).child(
                img(picture.image.clone())
                    .w(px(w as f32 / 1.5)).max_w_full()
                    .h(px(h as f32 / 1.5))
                    .object_fit(ObjectFit::Contain),
            );
        }
        Some(Err(message)) => message,
        None if previews.is_some() => mermaid::TOO_MANY,
        None => return div().text_size(px(12.0)).text_color(t.dim).child("Drawing Mermaid diagram…"),
    };
    let sample = &source[..source.floor_char_boundary(2048)];
    let mut lines = sample.lines();
    let mut excerpt = lines.by_ref().take(12).collect::<Vec<_>>().join("\n");
    if sample.len() < source.len() || lines.next().is_some() {
        excerpt.push_str("\n… (full source in Edit)");
    }
    code_block(t, status, SharedString::from(excerpt.replace('\t', "    ")))
}

pub(super) fn render_with_diagrams(blocks: &[Block], folder: &str, t: &Theme, previews: Option<&Previews>, selection: &mut Selection, cx: &mut Context<Editor>) -> Vec<AnyElement> {
    let mut text_view = PreviewText { fragments: Fragments::default(), range: selection.range(), theme: t, editor: cx.weak_entity() };
    let mut out = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        if i != 0 {
            text_view.fragments.separate("\n\n");
        }
        let id = SharedString::from(format!("md-{i}"));
        let el: AnyElement = match b {
            Block::Heading(level, text) => div()
                .w_full()
                .pt(px(if *level <= 2 { 8.0 } else { 4.0 }))
                .pb(px(4.0))
                .when(*level <= 2, |el| el.border_b_1().border_color(t.border))
                .text_size(px(heading_size(*level)))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child(text_view.rich(id, text))
                .into_any_element(),
            Block::Para(text) => div().debug_selector(|| format!("md-paragraph-{i}")).w_full().line_height(px(20.0)).child(text_view.rich(id, text)).into_any_element(),
            Block::Quote(text) => div()
                .w_full()
                .pl(px(12.0))
                .border_l_2()
                .border_color(t.accent.opacity(0.6))
                .text_color(t.muted)
                .child(text_view.rich(id, text))
                .into_any_element(),
            Block::Item { depth, mark, text } => {
                let marker = match mark {
                    Mark::Bullet => "•".to_string(),
                    Mark::Number(n) => format!("{n}."),
                    Mark::Task(true) => "☑".to_string(),
                    Mark::Task(false) => "☐".to_string(),
                };
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.0))
                    .pl(px(6.0 + 18.0 * *depth as f32))
                    .child(div().flex_none().min_w(px(14.0)).text_color(t.dim).child(marker))
                    .child(div().flex_1().min_w(px(0.0)).child(text_view.rich(id, text)))
                    .into_any_element()
            }
            Block::Code { info, text } if mermaid::is_fence(info) => diagram(t, text, i, previews).into_any_element(),
            Block::Code { info, text } => code_block(t, info, text_view.styled(&text.replace('\t', "    "), Vec::new())).into_any_element(),
            Block::Rule => div().w_full().h(px(1.0)).my(px(6.0)).bg(t.border).into_any_element(),
            Block::Table { head, rows } => {
                let cols = head.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0)).max(1);
                let mut row = |cells: &[Inline], r: usize, bold: bool| {
                    if !bold {
                        text_view.fragments.separate("\n");
                    }
                    div()
                        .w_full()
                        .flex()
                        .border_b_1()
                        .border_color(t.border)
                        .when(bold, |el| el.bg(t.surface).font_weight(FontWeight::SEMIBOLD))
                        .children((0..cols).map(|c| {
                            if c > 0 {
                                text_view.fragments.separate("\t");
                            }
                            let cell = cells.get(c).cloned().unwrap_or_default();
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .px(px(8.0))
                                .py(px(4.0))
                                .child(text_view.rich(SharedString::from(format!("md-{i}-{r}-{c}")), &cell))
                        }))
                };
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(t.border)
                    .overflow_hidden()
                    .child(row(head, 0, true))
                    .children(rows.iter().enumerate().map(|(r, cells)| row(cells, r + 1, false)))
                    .into_any_element()
            }
            Block::Image { alt, src } => match crate::markdown::link_target(src, folder) {
                Link::File(path) if std::path::Path::new(&path).is_file() => div()
                    .id(id)
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(img(PathBuf::from(path)).max_w_full())
                    .into_any_element(),
                _ => {
                    let label = Inline {
                        text: if alt.is_empty() { src.clone() } else { format!("Picture: {alt}") },
                        spans: vec![],
                    };
                    let label = Inline {
                        spans: vec![crate::markdown::Span { range: 0..label.text.len(), style: Style::Link(src.clone()) }],
                        ..label
                    };
                    div().w_full().child(text_view.rich(id, &label)).into_any_element()
                }
            },
        };
        out.push(el);
    }
    selection.fragments = text_view.fragments;
    out
}

pub fn scroller(id: &'static str, t: &Theme, children: Vec<AnyElement>, handle: &gpui::ScrollHandle) -> Div {
    nixa_ui::scroll::scroll_area(t, id, handle,
            div()
                .w_full()
                .max_w(px(860.0))
                .px(px(22.0))
                .py(px(16.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .text_size(px(13.5))
                .children(children),
        ).flex_1().w_full().bg(t.background)
}
