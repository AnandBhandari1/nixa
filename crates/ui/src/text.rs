use crate::theme::Theme;
use gpui::{Div, FontWeight, ParentElement, SharedString, Styled, div, px};

pub const UI_FONT: &str = ".SystemUIFont";
pub const MONO_FONT: &str = "Menlo";

pub const XS: f32 = 10.5;
pub const SM: f32 = 11.5;
pub const MENU: f32 = 12.8;
pub const BASE: f32 = 13.0;
pub const MD: f32 = 14.0;
pub const LG: f32 = 16.0;
pub const XL: f32 = 20.0;

pub fn label(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(px(BASE)).text_color(t.foreground).child(text.into())
}

pub fn strong(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(BASE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.bright)
        .child(text.into())
}

pub fn muted(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(px(SM)).text_color(t.muted).child(text.into())
}

pub fn caption(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(px(XS)).text_color(t.dim).child(text.into())
}

pub fn heading(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(LG))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.bright)
        .child(text.into())
}

pub fn title(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(XL))
        .font_weight(FontWeight::BOLD)
        .text_color(t.bright)
        .child(text.into())
}

pub fn mono(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .font_family(MONO_FONT)
        .text_size(px(12.0))
        .text_color(t.foreground)
        .child(text.into())
}

pub fn shortcut(t: &Theme, keys: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .text_size(px(11.0))
        .text_color(t.dim)
        .child(keys.into())
}

pub fn kbd(t: &Theme, keys: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(18.0))
        .px(px(5.0))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(t.border)
        .bg(t.raised)
        .text_size(px(XS))
        .text_color(t.muted)
        .child(keys.into())
}
