use super::Styles;
use gpui::{HighlightStyle, Hsla, Pixels, Point, SharedString, StyledText, TextLayout, px};
use std::ops::Range;

#[derive(Default)]
pub(crate) struct Selection {
    pub(super) fragments: Fragments,
    anchor: usize,
    head: usize,
    press: Option<Point<Pixels>>,
    moved: bool,
}

impl Selection {
    pub(super) fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub(crate) fn begin(&mut self, at: Point<Pixels>, extend: bool) {
        let Some(index) = self.fragments.at(at) else { return };
        if !extend {
            self.anchor = index;
        }
        self.head = index;
        self.press = Some(at);
        self.moved = extend;
    }

    pub(crate) fn extend(&mut self, at: Point<Pixels>) -> bool {
        let Some(press) = self.press else { return false };
        self.moved |= apart(press, at);
        if self.moved && let Some(index) = self.fragments.at(at) && index != self.head {
            self.head = index;
            return true;
        }
        false
    }

    pub(crate) fn finish(&mut self, at: Point<Pixels>) -> bool {
        let changed = self.extend(at);
        self.press = None;
        changed
    }

    pub(crate) fn select_all(&mut self) {
        let all = self.fragments.all();
        self.anchor = all.start;
        self.head = all.end;
    }

    pub(crate) fn copied(&self) -> Option<String> {
        self.fragments.copy(self.range())
    }

    #[cfg(test)]
    pub(crate) fn highlighted(&self) -> &[String] {
        &self.fragments.highlighted
    }

    #[cfg(test)]
    pub(crate) fn fragment_point(&self, fragment: usize, index: usize) -> Option<Point<Pixels>> {
        let (_, layout) = self.fragments.layouts.get(fragment)?;
        let at = layout.position_for_index(index)?;
        Some(at + gpui::point(px(0.25), layout.line_height() / 2.0))
    }

    pub(super) fn link_click(&self, at: Point<Pixels>) -> bool {
        !self.moved && self.press.is_none_or(|press| !apart(press, at))
    }
}

fn apart(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    (a.x - b.x).abs() > px(3.0) || (a.y - b.y).abs() > px(3.0)
}

#[derive(Default)]
pub(in crate::md_view) struct Fragments {
    text: String,
    layouts: Vec<(usize, TextLayout)>,
    #[cfg(test)]
    highlighted: Vec<String>,
}

impl Fragments {
    pub(in crate::md_view) fn separate(&mut self, separator: &str) {
        self.text.push_str(separator);
    }

    pub(in crate::md_view) fn styled(
        &mut self,
        text: &str,
        styles: Styles,
        selection: &Range<usize>,
        color: Hsla,
    ) -> StyledText {
        let start = self.text.len();
        self.text.push_str(text);
        let from = text.floor_char_boundary(selection.start.saturating_sub(start));
        let to = text.floor_char_boundary(selection.end.saturating_sub(start));
        let styles = if from < to { selected(styles, from..to, color) } else { styles };
        #[cfg(test)]
        {
            let shown: String = styles.iter().filter(|(_, style)| style.background_color == Some(color))
                .map(|(range, _)| &text[range.clone()]).collect();
            if !shown.is_empty() {
                self.highlighted.push(shown);
            }
        }
        let styled = StyledText::new(SharedString::from(text.to_owned())).with_highlights(styles);
        if !text.is_empty() {
            self.layouts.push((start, styled.layout().clone()));
        }
        styled
    }

    pub(in crate::md_view) fn at(&self, position: Point<Pixels>) -> Option<usize> {
        let (start, layout) = self.layouts.iter().min_by_key(|(_, layout)| {
            let bounds = layout.bounds();
            let dy = (bounds.top() - position.y).max(position.y - bounds.bottom()).max(px(0.0));
            let dx = (bounds.left() - position.x).max(position.x - bounds.right()).max(px(0.0));
            (dy, dx)
        })?;
        let local = layout.index_for_position(position).unwrap_or_else(|index| index).min(layout.len());
        Some(self.text.floor_char_boundary(start + local))
    }

    pub(in crate::md_view) fn all(&self) -> Range<usize> {
        0..self.text.len()
    }

    pub(in crate::md_view) fn copy(&self, range: Range<usize>) -> Option<String> {
        if range.is_empty() { None } else { self.text.get(range).map(str::to_owned) }
    }
}

fn selected(styles: Styles, range: Range<usize>, color: Hsla) -> Styles {
    let mut pieces = Vec::with_capacity(styles.len() + 3);
    let highlight = HighlightStyle { background_color: Some(color), ..HighlightStyle::default() };
    let mut at = range.start;
    for (part, style) in styles {
        if part.end <= range.start || part.start >= range.end {
            if part.start >= range.end && at < range.end {
                pieces.push((at..range.end, highlight));
                at = range.end;
            }
            pieces.push((part, style));
            continue;
        }
        let from = part.start.max(range.start);
        let to = part.end.min(range.end);
        if part.start < from {
            pieces.push((part.start..from, style));
        }
        if at < from {
            pieces.push((at..from, highlight));
        }
        pieces.push((from..to, HighlightStyle { background_color: Some(color), ..style }));
        at = to;
        if to < part.end {
            pieces.push((to..part.end, style));
        }
    }
    if at < range.end {
        pieces.push((at..range.end, highlight));
    }
    pieces
}
