use crate::icons::{Icon, icon};
use crate::metrics::{MENU_ROW_HEIGHT, RADIUS_ROW};
use crate::popover::popover;
use crate::section::separator;
use crate::text;
use crate::theme::Theme;
use gpui::{
    App, Div, ElementId, InteractiveElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    pub label: SharedString,
    pub icon: Option<Icon>,
    pub shortcut: Option<SharedString>,
    pub checked: bool,
    pub danger: bool,
    pub disabled: bool,
    pub separator_before: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        MenuItem {
            label: label.into(),
            ..MenuItem::default()
        }
    }

    pub fn icon(mut self, which: Icon) -> Self {
        self.icon = Some(which);
        self
    }

    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn disabled(mut self, off: bool) -> Self {
        self.disabled = off;
        self
    }

    pub fn separated(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Move(usize),
    Choose(usize),
    Close,
    Ignore,
}

pub fn menu_key(items: &[MenuItem], current: Option<usize>, key: &str) -> MenuKey {
    let usable: Vec<usize> = (0..items.len()).filter(|&i| !items[i].disabled).collect();
    if usable.is_empty() {
        return if key == "escape" { MenuKey::Close } else { MenuKey::Ignore };
    }
    let at = current.and_then(|c| usable.iter().position(|&i| i == c));
    match key {
        "down" => MenuKey::Move(usable[at.map_or(0, |p| (p + 1) % usable.len())]),
        "up" => MenuKey::Move(usable[at.map_or(usable.len() - 1, |p| (p + usable.len() - 1) % usable.len())]),
        "home" => MenuKey::Move(usable[0]),
        "end" => MenuKey::Move(usable[usable.len() - 1]),
        "enter" | "space" => at.map_or(MenuKey::Ignore, |p| MenuKey::Choose(usable[p])),
        "escape" => MenuKey::Close,
        _ => MenuKey::Ignore,
    }
}

pub type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub fn menu_row(t: &Theme, id: impl Into<ElementId>, item: &MenuItem, highlighted: bool) -> Stateful<Div> {
    let hover = t.hover;
    let fg = if item.danger { t.red } else { t.foreground };
    let icon_color = if item.danger { t.red } else { t.muted };
    div()
        .id(id)
        .flex_none()
        .h(px(MENU_ROW_HEIGHT))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(9.0))
        .rounded(px(RADIUS_ROW))
        .text_color(fg)
        .when(item.disabled, |el| el.opacity(0.45))
        .when(!item.disabled, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .when(highlighted && !item.disabled, |el| el.bg(t.selected))
        .child(
            div()
                .flex_none()
                .size(px(14.0))
                .when_some(item.icon, |el, which| el.child(icon(which).size(px(14.0)).text_color(icon_color))),
        )
        .child(div().flex_1().min_w(px(0.0)).truncate().child(item.label.clone()))
        .when(item.checked, |el| el.child(icon(Icon::Check).size(px(14.0)).text_color(t.foreground)))
        .when_some(item.shortcut.clone(), |el, keys| el.child(text::shortcut(t, keys)))
}

pub fn menu(
    t: &Theme,
    id: impl Into<ElementId>,
    title: Option<SharedString>,
    items: &[MenuItem],
    highlighted: Option<usize>,
    on_pick: OnPick,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let mut panel = popover(t).id(id.clone()).min_w(px(200.0));
    if let Some(title) = title {
        panel = panel.child(
            div()
                .px(px(8.0))
                .pt(px(2.0))
                .pb(px(4.0))
                .text_size(px(11.0))
                .text_color(t.dim)
                .truncate()
                .child(title),
        );
    }
    for (index, item) in items.iter().enumerate() {
        if item.separator_before && index > 0 {
            panel = panel.child(separator(t));
        }
        let pick = on_pick.clone();
        let row = menu_row(t, crate::child_id(&id, index.to_string()), item, highlighted == Some(index));
        let row = if item.disabled {
            row
        } else {
            row.on_click(move |_, window, cx| pick(index, window, cx))
        };
        panel = panel.child(row);
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::new("Split right"),
            MenuItem::new("Off").disabled(true),
            MenuItem::new("Zoom"),
            MenuItem::new("Close").separated().danger(),
        ]
    }

    #[test]
    fn down_skips_disabled_and_wraps() {
        let list = items();
        assert_eq!(menu_key(&list, None, "down"), MenuKey::Move(0));
        assert_eq!(menu_key(&list, Some(0), "down"), MenuKey::Move(2));
        assert_eq!(menu_key(&list, Some(3), "down"), MenuKey::Move(0));
    }

    #[test]
    fn up_wraps_to_last() {
        let list = items();
        assert_eq!(menu_key(&list, None, "up"), MenuKey::Move(3));
        assert_eq!(menu_key(&list, Some(0), "up"), MenuKey::Move(3));
        assert_eq!(menu_key(&list, Some(2), "up"), MenuKey::Move(0));
    }

    #[test]
    fn enter_and_escape() {
        let list = items();
        assert_eq!(menu_key(&list, Some(2), "enter"), MenuKey::Choose(2));
        assert_eq!(menu_key(&list, None, "enter"), MenuKey::Ignore);
        assert_eq!(menu_key(&list, Some(1), "enter"), MenuKey::Ignore);
        assert_eq!(menu_key(&list, Some(2), "escape"), MenuKey::Close);
        assert_eq!(menu_key(&list, Some(2), "x"), MenuKey::Ignore);
    }

    #[test]
    fn all_disabled_menu() {
        let list = vec![MenuItem::new("a").disabled(true)];
        assert_eq!(menu_key(&list, None, "down"), MenuKey::Ignore);
        assert_eq!(menu_key(&list, None, "escape"), MenuKey::Close);
    }
}
