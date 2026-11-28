use super::motion;
use gpui::prelude::*;
use gpui::{AnyElement, Div, HighlightStyle, StyledText, div, hsla, px, rgb};

const INK: u32 = 0x1d2129;
const PAPER: u32 = 0xffffff;
pub const LEAD_MS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pop {
    pub scale: f32,
    pub alpha: f32,
    pub up: f32,
}

pub fn shown_chars(n: usize, tb: u64, say_ms: u64) -> usize {
    if tb < LEAD_MS {
        return 0;
    }
    let reveal = say_ms.saturating_sub(900).max(400);
    let done = (n as u64 * (tb - LEAD_MS)).div_ceil(reveal);
    (done as usize).min(n)
}

pub fn pop_in(tb: u64) -> Pop {
    let k = (tb as f32 / 220.0).min(1.0);
    Pop {
        scale: 0.6 + 0.4 * motion::out_back(k, 1.7),
        alpha: (tb as f32 / 120.0).min(1.0),
        up: 0.0,
    }
}

pub fn dimmed(p: Pop, td: u64) -> Pop {
    let k = motion::out_cubic(td as f32 / 180.0);
    Pop {
        scale: p.scale * (1.0 - 0.08 * k),
        alpha: p.alpha * (1.0 - 0.55 * k),
        up: p.up + 6.0 * k,
    }
}

pub fn popped_out(p: Pop, tw: u64) -> Pop {
    let k = motion::in_cubic(tw as f32 / 160.0);
    Pop {
        scale: p.scale * (1.0 - 0.15 * k),
        alpha: p.alpha * (1.0 - k),
        up: p.up,
    }
}

fn tail(s: f32) -> Div {
    let row = |w: f32, last: bool| {
        div()
            .w(px((w * s).max(2.0)))
            .h(px((2.0 * s).max(1.0)))
            .bg(rgb(if last { INK } else { PAPER }))
            .border_l_1()
            .border_r_1()
            .border_color(rgb(INK))
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .mt(px(-2.0))
        .child(row(9.0, false))
        .child(row(6.0, false))
        .child(row(3.0, true))
}

fn frame((sx, sy): (f32, f32), h: f32, pop: Pop, width: f32) -> Div {
    div()
        .absolute()
        .left(px(sx - width / 2.0))
        .bottom(px(h - sy + pop.up))
        .w(px(width))
        .flex()
        .flex_col()
        .items_center()
        .opacity(pop.alpha.clamp(0.0, 1.0))
}

pub fn guess_width(text: &str, big: bool) -> f32 {
    let size = if big { 13.0 } else { 11.0 };
    let max = if big { 240.0 } else { 180.0 };
    (text.chars().count() as f32 * size * 0.55 + 20.0).min(max)
}

pub fn bubble(text: &str, shown: usize, at: (f32, f32), h: f32, pop: Pop, big: bool, side: f32) -> AnyElement {
    let s = pop.scale.max(0.1);
    let max_w = if big { 240.0 } else { 180.0 } * s;
    let size = if big { 13.0 } else { 11.0 } * s;
    let shift = side * (guess_width(text, big) * s / 2.0 - 14.0 * s).max(0.0) * 0.6;
    let cut = text
        .char_indices()
        .nth(shown)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let clear = HighlightStyle {
        color: Some(hsla(0.0, 0.0, 0.0, 0.0)),
        ..Default::default()
    };
    let hidden = (cut < text.len()).then_some((cut..text.len(), clear));
    let words = StyledText::new(text.to_string()).with_highlights(hidden);
    let card = div()
        .relative()
        .left(px(shift))
        .max_w(px(max_w))
        .px(px(8.0 * s))
        .py(px(5.0 * s))
        .rounded(px(4.0 * s))
        .bg(rgb(PAPER))
        .border_2()
        .border_color(rgb(INK))
        .text_size(px(size))
        .text_color(rgb(INK))
        .when(!big, |el| el.line_clamp(2).text_ellipsis())
        .child(words);
    frame(at, h, pop, max_w + 2.0 * shift.abs())
        .child(card)
        .child(tail(s))
        .into_any_element()
}

pub fn dots(at: (f32, f32), h: f32, now: u64, alpha: f32) -> AnyElement {
    let lit = (now / 200) % 3;
    let mut row = div()
        .flex()
        .gap(px(3.0))
        .px(px(6.0))
        .py(px(5.0))
        .rounded(px(4.0))
        .bg(rgb(PAPER))
        .border_2()
        .border_color(rgb(INK));
    for k in 0..3u64 {
        let a = if k == lit { 1.0 } else { 0.3 };
        row = row.child(div().size(px(4.0)).bg(rgb(INK)).opacity(a));
    }
    let pop = Pop {
        scale: 1.0,
        alpha,
        up: 0.0,
    };
    frame(at, h, pop, 60.0)
        .child(row)
        .child(tail(0.8))
        .into_any_element()
}
