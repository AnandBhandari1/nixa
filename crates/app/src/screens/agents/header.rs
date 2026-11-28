use super::model::{self, Scope, Sort};
use super::{AgentsView, bring_back, focus, set_scope, set_sort};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Corner, FontWeight, MouseDownEvent, SharedString, div, px,
};
use nixa_ui::menu::{MenuItem, menu_row};
use nixa_ui::popover::popover;
use nixa_ui::popover::{float_at, tooltip};
use nixa_ui::{Icon, Theme, icon};
use std::rc::Rc;

type Pick = Rc<dyn Fn(usize, &mut gpui::App)>;

pub fn render(v: &AgentsView, t: &Theme, cx: &mut Context<AgentsView>) -> AnyElement {
    div()
        .w_full()
        .h(px(41.))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px(px(16.))
        .pt(px(14.))
        .pb(px(5.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.dim)
                        .child("Tabs"),
                )
                .child(scope_switch(v, t, cx)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(next_needy(v, t, cx))
                .child(sort_button(v, t, cx)),
        )
        .into_any_element()
}

fn scope_switch(
    v: &AgentsView,
    t: &Theme,
    cx: &mut Context<AgentsView>,
) -> impl IntoElement + use<> {
    let hover = t.hover;
    let option = |id: &'static str, label: &'static str, tip: &'static str, scope: Scope| {
        let on = v.scope == scope;
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .h(px(18.))
            .px(px(5.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .cursor_pointer()
            .text_size(px(10.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if on { t.bright } else { t.dim })
            .when(on, |el| el.bg(t.raised))
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |v, _, _, cx| set_scope(v, scope, cx)))
            .child(label)
    };
    let space = option(
        "scope-space",
        "This space",
        "Show agents in this space",
        Scope::Space,
    );
    let all = option(
        "scope-all",
        "All spaces",
        "Show agents in every space",
        Scope::All,
    );
    div()
        .p(px(1.))
        .flex()
        .items_center()
        .gap(px(1.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .child(space)
        .child(all)
}

fn next_needy(v: &AgentsView, t: &Theme, cx: &mut Context<AgentsView>) -> impl IntoElement + use<> {
    let any = model::next_needy(&v.input).is_some();
    let hover = t.hover;
    div()
        .id("ag-next-needy")
        .debug_selector(|| "ag-next-needy".into())
        .size(px(20.))
        .flex_none()
        .rounded(px(5.))
        .bg(t.red.opacity(0.15))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(t.red)
        .opacity(if any { 1. } else { 0.35 })
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(if any {
            "Focus the next agent that needs you, in any space"
        } else {
            "No agent needs you right now"
        }))
        .on_click(cx.listener(|v, _, _, cx| focus(model::next_needy(&v.input), cx)))
        .child("!")
}

fn sort_button(
    v: &AgentsView,
    t: &Theme,
    cx: &mut Context<AgentsView>,
) -> impl IntoElement + use<> {
    let hover = t.hover;
    let which = match v.sort {
        Sort::Priority => Icon::Clock,
        Sort::Grouped => Icon::Layers,
    };
    div()
        .id("ag-sort")
        .debug_selector(|| "ag-sort".into())
        .p(px(3.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(format!("Sort agents: {}", v.sort.label())))
        .on_click(cx.listener(|v, e: &ClickEvent, _, cx| {
            v.sort_menu = match v.sort_menu {
                Some(_) => None,
                None => Some(e.position()),
            };
            cx.notify();
        }))
        .child(icon(which).size(px(14.)).text_color(t.dim))
}

pub fn menus(v: &AgentsView, t: &Theme, cx: &mut Context<AgentsView>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let this = cx.entity();
    if let Some(at) = v.sort_menu {
        let items = vec![
            MenuItem::new(Sort::Priority.label())
                .icon(Icon::Clock)
                .checked(v.sort == Sort::Priority),
            MenuItem::new(Sort::Grouped.label())
                .icon(Icon::Layers)
                .checked(v.sort == Sort::Grouped),
            MenuItem::new("Close"),
        ];
        let target = this.clone();
        let pick = Rc::new(move |i: usize, cx: &mut gpui::App| {
            target.update(cx, |v, cx| match i {
                0 => set_sort(v, Sort::Priority, cx),
                1 => set_sort(v, Sort::Grouped, cx),
                _ => { v.sort_menu = None; cx.notify(); }
            });
        });
        out.push(
            float_at(
                at,
                Corner::TopRight,
                div()
                    .id("agents-sort-holder")
                    .on_mouse_down_out(cx.listener(|v, _: &MouseDownEvent, _, cx| {
                        v.sort_menu = None;
                        cx.notify();
                    }))
                    .child(menu(t, "agents-sort-menu", "Sort agents", &items, pick)),
            )
            .into_any_element(),
        );
    }
    if let Some(at) = v.away_menu {
        let away = v.input.away.clone();
        let items: Vec<MenuItem> = away
            .iter()
            .map(|a| MenuItem::new(SharedString::from(a.name.clone())).icon(Icon::Undo))
            .collect();
        let target = this.clone();
        let pick = Rc::new(move |i: usize, cx: &mut gpui::App| {
            if let Some(tab) = away.get(i).map(|a| a.tab.clone()) {
                target.update(cx, |v, cx| bring_back(v, &tab, cx));
            }
        });
        out.push(
            float_at(
                at,
                Corner::BottomLeft,
                div()
                    .id("agents-away-holder")
                    .on_mouse_down_out(cx.listener(|v, _: &MouseDownEvent, _, cx| {
                        v.away_menu = None;
                        cx.notify();
                    }))
                    .child(menu(
                        t,
                        "agents-away-menu",
                        "Put away in this space: click to bring back",
                        &items,
                        pick,
                    )),
            )
            .into_any_element(),
        );
    }
    out
}

fn menu(
    t: &Theme,
    id: &'static str,
    title: &'static str,
    items: &[MenuItem],
    pick: Pick,
) -> gpui::Stateful<gpui::Div> {
    popover(t)
        .id(id)
        .min_w(px(200.))
        .child(
            div()
                .px(px(8.))
                .pt(px(2.))
                .pb(px(4.))
                .text_size(px(11.))
                .text_color(t.dim)
                .truncate()
                .child(title),
        )
        .children(items.iter().enumerate().map(|(i, item)| {
            let pick = pick.clone();
            let selector = format!("{id}-{i}");
            menu_row(t, SharedString::from(format!("{id}-{i}")), item, false)
                .debug_selector(move || selector)
                .on_click(move |_, _, cx| pick(i, cx))
        }))
}
