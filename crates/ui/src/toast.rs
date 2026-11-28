use crate::icon_button::{IconSize, icon_button};
use crate::icons::{Icon, icon};
use crate::popover::shadow;
use crate::text;
use crate::theme::Theme;
use gpui::{
    App, Deferred, Div, ElementId, FontWeight, Hsla, InteractiveElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, deferred, div, prelude::FluentBuilder, px,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub title: SharedString,
    pub body: Option<SharedString>,
}

#[derive(Clone, Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

pub const MAX_TOASTS: usize = 4;

impl Toasts {
    pub fn push(&mut self, kind: ToastKind, title: impl Into<SharedString>, body: Option<SharedString>) -> u64 {
        self.next += 1;
        self.items.push(Toast {
            id: self.next,
            kind,
            title: title.into(),
            body,
        });
        if self.items.len() > MAX_TOASTS {
            self.items.remove(0);
        }
        self.next
    }

    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        before != self.items.len()
    }

    pub fn items(&self) -> &[Toast] {
        &self.items
    }
}

pub fn kind_color(t: &Theme, kind: ToastKind) -> Hsla {
    match kind {
        ToastKind::Info => t.accent,
        ToastKind::Success => t.green,
        ToastKind::Warning => t.yellow,
        ToastKind::Error => t.red,
    }
}

pub fn kind_icon(kind: ToastKind) -> Icon {
    match kind {
        ToastKind::Info => Icon::Info,
        ToastKind::Success => Icon::CircleCheck,
        ToastKind::Warning => Icon::CircleAlert,
        ToastKind::Error => Icon::CircleX,
    }
}

pub fn toast_card(
    t: &Theme,
    toast: &Toast,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let color = kind_color(t, toast.kind);
    let id = ElementId::Name(format!("nixa-toast-{}", toast.id).into());
    div()
        .id(id.clone())
        .w(px(320.0))
        .relative()
        .flex()
        .items_start()
        .gap(px(10.0))
        .pl(px(14.0))
        .pr(px(6.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.border)
        .shadow(shadow(t))
        .overflow_hidden()
        .child(div().absolute().left(px(0.0)).top(px(0.0)).bottom(px(0.0)).w(px(3.0)).bg(color))
        .child(icon(kind_icon(toast.kind)).size(px(16.0)).mt(px(1.0)).text_color(color))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(px(text::BASE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(toast.title.clone()),
                )
                .when_some(toast.body.clone(), |el, body| {
                    el.child(div().text_size(px(12.0)).text_color(t.muted).child(body))
                }),
        )
        .child(
            icon_button(t, crate::child_id(&id, "close"), Icon::X, IconSize::Small, false)
                .on_click(move |_, window, cx| on_close(window, cx)),
        )
}

pub fn toast_stack() -> Div {
    div()
        .absolute()
        .bottom(px(40.0))
        .right(px(56.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
}

pub fn toast_layer(stack: Div) -> Deferred {
    deferred(stack).with_priority(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_dismiss() {
        let mut list = Toasts::default();
        let a = list.push(ToastKind::Info, "a", None);
        let b = list.push(ToastKind::Error, "b", Some("why".into()));
        assert_ne!(a, b);
        assert_eq!(list.items().len(), 2);
        assert!(list.dismiss(a));
        assert!(!list.dismiss(a));
        assert_eq!(list.items()[0].id, b);
    }

    #[test]
    fn keeps_only_newest() {
        let mut list = Toasts::default();
        for n in 0..(MAX_TOASTS + 2) {
            list.push(ToastKind::Success, format!("t{n}"), None);
        }
        assert_eq!(list.items().len(), MAX_TOASTS);
        assert_eq!(list.items()[0].title.as_ref(), "t2");
    }
}
