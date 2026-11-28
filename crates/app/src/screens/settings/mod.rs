pub mod parts;

use crate::actions::ClosePane;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, Bounds, Context, Entity, EntityId, FocusHandle,
    FontWeight, Global, KeyDownEvent, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, div, point, px, size,
};
use nixa_ui::metrics::{RADIUS_POPOVER, TRAFFIC_LIGHTS_X, TRAFFIC_LIGHTS_Y};
use nixa_ui::section::group_title;
use nixa_ui::{Icon, icon, theme};
use parts::drones::DronesView;
use parts::git_accounts::GitAccounts;
use parts::app::AppView;
use parts::look::LookView;
use parts::memory::doc::DocEditor;
use parts::rule_trees::RuleTreesEvent;
use parts::rules::RulesView;
use parts::repeats::Repeats;
use parts::schedules::Schedules;
use parts::spaces::SpacesPart;
use parts::{
    agents, app, chat_brain, connectors, drones, general, git_accounts, knowledge, look, memory, mic,
    decide, heavy, nixa_screen, phone, privacy, repeats, rules, schedules, spaces, voices, web_search, workflows,
};
use serde_json::Value;

pub const MEMORY: &str = "Memory";
pub const RULES: &str = rules::PART;
pub const TITLE: &str = "Nixa settings";

pub const GROUPS: &[(&str, &[&str])] = &[
    (
        "Nixa",
        &[
            general::PART,
            chat_brain::PART,
            repeats::PART,
            MEMORY,
            knowledge::PART,
            mic::PART,
            voices::PART,
            phone::PART,
            privacy::PART,
            nixa_screen::PART,
        ],
    ),
    (
        "Work",
        &[
            spaces::PART,
            agents::PART,
            workflows::PART,
            RULES,
            decide::PART,
            drones::PART,
            heavy::PART,
            schedules::PART,
            git_accounts::PART,
            web_search::PART,
            connectors::PART,
        ],
    ),
    ("App", &[look::PART, app::PART]),
];

pub const LATER: &[(&str, &str)] = &[
];

pub fn group_of(part: &str) -> Option<&'static str> {
    GROUPS
        .iter()
        .find(|(_, parts)| parts.contains(&part))
        .map(|(group, _)| *group)
}

pub fn known(part: &str) -> &'static str {
    if rules::is_part(part) {
        return RULES;
    }
    all()
        .find(|p| *p == part)
        .unwrap_or(general::PART)
}

pub fn all() -> impl Iterator<Item = &'static str> {
    GROUPS.iter().flat_map(|(_, parts)| parts.iter().copied())
}

pub fn later(part: &str) -> Option<&'static str> {
    LATER.iter().find(|(p, _)| *p == part).map(|(_, text)| *text)
}

#[derive(Default)]
pub struct Kept {
    pub memory: Option<Entity<DocEditor>>,
    pub rules: Option<Entity<RulesView>>,
    pub drones: Option<Entity<DronesView>>,
    pub workflows: Option<Entity<crate::screens::workflows::WorkflowsView>>,
    pub spaces: Option<Entity<SpacesPart>>,
    pub schedules: Option<Entity<Schedules>>,
    pub repeats: Option<Entity<Repeats>>,
    pub look: Option<Entity<LookView>>,
    pub app: Option<Entity<AppView>>,
    pub git_accounts: Option<Entity<GitAccounts>>,
}

pub struct SettingsView {
    pub part: &'static str,
    pub kept: Kept,
    focus: FocusHandle,
    parts_scroll: ScrollHandle,
    reveal_part: bool,
    watched: Vec<EntityId>,
    subs: Vec<Subscription>,
}

struct SettingsWindow(AnyWindowHandle);

impl Global for SettingsWindow {}

fn handle(cx: &App) -> Option<WindowHandle<SettingsView>> {
    cx.try_global::<SettingsWindow>()
        .and_then(|g| g.0.downcast::<SettingsView>())
}

fn workspace(cx: &App) -> Option<WindowHandle<Workspace>> {
    cx.windows().iter().find_map(|w| w.downcast::<Workspace>())
}

pub fn active_folder(cx: &App) -> Option<String> {
    let ws = workspace(cx)?;
    let ws = ws.read(cx).ok()?;
    ws.active_space()
        .map(|s| s.folder.clone())
        .filter(|f| !f.is_empty())
}

pub fn open(part: &str, cx: &mut App) {
    let part = known(part);
    if let Some(handle) = handle(cx) {
        let shown = handle.update(cx, |view, window, cx| {
            select(view, part, cx);
            window.activate_window();
            cx.notify();
        });
        if shown.is_ok() {
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(1000.), px(740.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(TRAFFIC_LIGHTS_X), px(TRAFFIC_LIGHTS_Y))),
        }),
        window_min_size: Some(size(px(640.), px(420.))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        window.on_window_should_close(cx, |_, cx| {
            leave_voices(cx);
            true
        });
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus);
            let mut view = SettingsView {
                part,
                kept: Kept::default(),
                focus,
                parts_scroll: ScrollHandle::new(),
                reveal_part: true,
                watched: Vec::new(),
                subs: Vec::new(),
            };
            prepare(&mut view, cx);
            view
        })
    });
    if let Ok(handle) = opened {
        cx.set_global(SettingsWindow(handle.into()));
    }
}

fn watch<T: 'static>(view: &mut SettingsView, part: &Entity<T>, cx: &mut Context<SettingsView>) {
    let id = part.entity_id();
    if !view.watched.contains(&id) {
        view.watched.push(id);
        view.subs.push(cx.observe(part, |_, _, cx| cx.notify()));
    }
}

pub struct SharedBrainLink(pub crate::links::brain::Link);

impl Global for SharedBrainLink {}

pub fn share_brain_link(link: crate::links::brain::Link, cx: &mut App) {
    cx.set_global(SharedBrainLink(link));
}

fn brain_link(cx: &App) -> Option<crate::links::brain::Link> {
    if let Some(shared) = cx.try_global::<SharedBrainLink>() {
        return Some(shared.0.clone());
    }
    workspace(cx)?.read(cx).ok()?.hub.link.clone()
}

fn brain_addr() -> String {
    nixa_protocol::paths::brain_addr()
}

fn select(view: &mut SettingsView, part: &'static str, cx: &mut Context<SettingsView>) {
    if view.part == voices::PART && part != voices::PART {
        leave_voices(cx);
    }
    view.part = part;
    prepare(view, cx);
}

fn part_row(part: &str) -> usize {
    GROUPS.iter().flat_map(|(_, parts)| std::iter::once(None).chain(parts.iter().copied().map(Some)))
        .position(|row| row == Some(part))
        .unwrap_or(1)
}

pub fn prepare(view: &mut SettingsView, cx: &mut Context<SettingsView>) {
    view.reveal_part = true;
    match view.part {
        general::PART => {
            if let Some(part) = general::shared(cx) {
                watch(view, &part, cx);
            }
        }
        chat_brain::PART => {
            let part = chat_brain::show(cx);
            watch(view, &part, cx);
        }
        MEMORY => {
            let part = view
                .kept
                .memory
                .get_or_insert_with(|| memory::new(brain_addr(), cx))
                .clone();
            watch(view, &part, cx);
        }
        mic::PART => {
            let part = mic::show(cx);
            if !view.watched.contains(&part.entity_id()) {
                view.subs.push(cx.subscribe(&part, |view, _, event: &mic::MicEvent, cx| {
                    let mic::MicEvent::OpenPart(part) = event;
                    select(view, known(part), cx);
                    cx.notify();
                }));
            }
            watch(view, &part, cx);
        }
        voices::PART => {
            let part = voices::show(cx);
            watch(view, &part, cx);
        }
        phone::PART => {
            let part = phone::show(cx);
            watch(view, &part, cx);
        }
        privacy::PART => {
            let part = privacy::show(cx);
            watch(view, &part, cx);
        }
        knowledge::PART => {
            let part = knowledge::show(cx);
            watch(view, &part, cx);
        }
        web_search::PART => {
            let part = web_search::show(cx);
            watch(view, &part, cx);
        }
        decide::PART => {
            let part = decide::show(cx);
            watch(view, &part, cx);
        }
        heavy::PART => {
            let part = heavy::show(cx);
            watch(view, &part, cx);
        }
        connectors::PART => {
            let part = connectors::show(cx);
            watch(view, &part, cx);
        }
        nixa_screen::PART => {
            if let Some(part) = crate::screens::nixa_screen::shared(cx) {
                watch(view, &part, cx);
                part.update(cx, |s, cx| s.ask_soap_models(cx));
            }
        }
        spaces::PART => {
            if view.kept.spaces.is_none() {
                let part = spaces::new(cx);
                view.subs.push(cx.observe(&part, |_, _, cx| reload_spaces(cx)));
                view.kept.spaces = Some(part);
            }
            if let Some(part) = view.kept.spaces.clone() {
                watch(view, &part, cx);
            }
        }
        agents::PART => {
            let part = agents::shared(cx);
            watch(view, &part, cx);
        }
        workflows::PART => {
            let part = view
                .kept
                .workflows
                .get_or_insert_with(|| workflows::new(cx))
                .clone();
            watch(view, &part, cx);
        }
        RULES => {
            if view.kept.rules.is_none() {
                let folder = active_folder(cx);
                let part = rules::new(brain_addr(), folder, cx);
                let trees = part.read(cx).trees.clone();
                view.subs.push(cx.subscribe(&trees, |_, _, event: &RuleTreesEvent, cx| {
                    on_rule_trees(event, cx)
                }));
                view.kept.rules = Some(part);
            }
            if let Some(part) = view.kept.rules.clone() {
                watch(view, &part, cx);
            }
        }
        drones::PART => {
            let part = view
                .kept
                .drones
                .get_or_insert_with(|| drones::new(brain_addr(), cx))
                .clone();
            watch(view, &part, cx);
        }
        schedules::PART => {
            let part = view
                .kept
                .schedules
                .get_or_insert_with(|| schedules::new(brain_link(cx), cx))
                .clone();
            watch(view, &part, cx);
        }
        repeats::PART => {
            let part = view
                .kept
                .repeats
                .get_or_insert_with(|| repeats::new(brain_link(cx), cx))
                .clone();
            watch(view, &part, cx);
        }
        look::PART => {
            let part = view.kept.look.get_or_insert_with(|| look::new(cx)).clone();
            watch(view, &part, cx);
        }
        app::PART => {
            let part = view.kept.app.get_or_insert_with(|| app::new(cx)).clone();
            watch(view, &part, cx);
        }
        git_accounts::PART => {
            let folder = active_folder(cx);
            let part = view
                .kept
                .git_accounts
                .get_or_insert_with(|| git_accounts::new(folder, cx))
                .clone();
            watch(view, &part, cx);
        }
        _ => {}
    }
}

fn reload_spaces(cx: &mut App) {
    let Some(ws) = workspace(cx) else {
        return;
    };
    cx.defer(move |cx| {
        ws.update(cx, |ws, _, cx| {
            crate::screens::spaces::handle(ws, &crate::screens::spaces::SpacesEvent::Reload, cx)
        })
        .ok();
    });
}

fn on_rule_trees(event: &RuleTreesEvent, cx: &mut App) {
    let RuleTreesEvent::EditJson { name, argv } = event;
    let Some(ws) = workspace(cx) else {
        return;
    };
    let argv = argv.clone();
    let title = format!("{name}.json");
    cx.defer(move |cx| {
        ws.update(cx, |ws, window, cx| {
            ws.new_tab(argv, Some(title), cx);
            window.activate_window();
        })
        .ok();
    });
}

pub fn feed(msg: &Value, cx: &mut App) {
    if let Some(part) = web_search::shared(cx) {
        web_search::apply(&part, msg, cx);
    }
    if let Some(part) = decide::shared(cx) {
        decide::apply(&part, msg, cx);
    }
    if let Some(part) = connectors::shared(cx) {
        connectors::apply(&part, msg, cx);
    }
    let Some(handle) = handle(cx) else {
        return;
    };
    let Ok(view) = handle.read(cx) else {
        return;
    };
    let memory = view.kept.memory.clone();
    let rules_part = view.kept.rules.clone();
    let drones_part = view.kept.drones.clone();
    if let Some(part) = memory {
        memory::feed(&part, msg, cx);
    }
    if let Some(part) = rules_part {
        rules::feed(&part, msg, cx);
    }
    if let Some(part) = drones_part {
        drones::feed(&part, msg, cx);
    }
}

pub enum Body {
    View(AnyView),
    Element(AnyElement),
    Later(&'static str),
    Waiting,
}

pub fn body(view: &SettingsView, cx: &mut App) -> Body {
    let kept = &view.kept;
    let found: Option<AnyView> = match view.part {
        general::PART => general::shared(cx).map(AnyView::from),
        chat_brain::PART => chat_brain::shared(cx).map(AnyView::from),
        MEMORY => kept.memory.clone().map(AnyView::from),
        mic::PART => mic::shared(cx).map(AnyView::from),
        voices::PART => crate::screens::voices::shared(cx).map(AnyView::from),
        phone::PART => phone::shared(cx).map(AnyView::from),
        privacy::PART => privacy::shared(cx).map(AnyView::from),
        knowledge::PART => knowledge::shared(cx).map(AnyView::from),
        web_search::PART => web_search::shared(cx).map(AnyView::from),
        decide::PART => decide::shared(cx).map(AnyView::from),
        heavy::PART => heavy::shared(cx).map(AnyView::from),
        connectors::PART => connectors::shared(cx).map(AnyView::from),
        nixa_screen::PART => return Body::Element(nixa_screen::render(cx)),
        spaces::PART => {
            return match kept.spaces.as_ref() {
                Some(part) => Body::Element(spaces::render(part)),
                None => Body::Waiting,
            };
        }
        agents::PART => return Body::Element(agents::render(cx)),
        workflows::PART => {
            return match kept.workflows.as_ref() {
                Some(part) => Body::Element(workflows::render(part, cx)),
                None => Body::Waiting,
            };
        }
        RULES => kept.rules.clone().map(AnyView::from),
        drones::PART => kept.drones.clone().map(AnyView::from),
        schedules::PART => kept.schedules.clone().map(AnyView::from),
        repeats::PART => kept.repeats.clone().map(AnyView::from),
        look::PART => kept.look.clone().map(AnyView::from),
        app::PART => kept.app.clone().map(AnyView::from),
        git_accounts::PART => kept.git_accounts.clone().map(AnyView::from),
        other => return later(other).map(Body::Later).unwrap_or(Body::Waiting),
    };
    found.map(Body::View).unwrap_or(Body::Waiting)
}

fn leave_voices(cx: &mut App) {
    if let Some(view) = crate::screens::voices::shared(cx) {
        crate::screens::voices::stop_refresh(&view, cx);
    }
}

fn close(window: &mut Window, cx: &mut App) {
    leave_voices(cx);
    window.remove_window();
}

fn notice(t: &nixa_ui::Theme, text: &'static str) -> AnyElement {
    div()
        .mt(px(14.))
        .p(px(16.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(RADIUS_POPOVER))
        .border_1()
        .border_color(t.border)
        .bg(t.chrome)
        .text_color(t.muted)
        .child(icon(Icon::Settings).size(px(16.)).text_color(t.dim))
        .child(text)
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.part;
        if std::mem::take(&mut self.reveal_part) {
            let view = cx.weak_entity();
            cx.defer(move |cx| {
                view.update(cx, |view, cx| {
                    view.parts_scroll.scroll_to_item(part_row(view.part));
                    cx.notify();
                }).ok();
            });
        }
        let t = theme(cx).clone();
        let hover = t.hover;
        let mut list = div()
            .id("settings-parts")
            .w(px(220.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(1.))
            .pt(px(46.))
            .px(px(8.))
            .pb(px(12.))
            .bg(t.chrome)
            .border_r_1()
            .border_color(t.border)
            .overflow_y_scroll()
            .track_scroll(&self.parts_scroll);
        for (group, parts) in GROUPS {
            list = list.child(group_title(&t, *group).flex_none().pt(px(12.)).pb(px(4.)).px(px(10.)));
            for part in parts.iter().copied() {
                let on = part == current;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("settings-part-{part}")))
                        .debug_selector(move || format!("settings-part-{part}"))
                        .h(px(28.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px(px(10.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_color(if on { t.bright } else { t.muted })
                        .when(on, |el| el.bg(t.selected))
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            select(view, part, cx);
                            cx.notify();
                        }))
                        .child(part),
                );
            }
        }
        let content = match body(self, cx) {
            Body::View(view) => div()
                .mt(px(10.))
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .pb(px(16.))
                .child(view)
                .into_any_element(),
            Body::Element(el) => div()
                .mt(px(10.))
                .flex_1()
                .min_h_0()
                .pb(px(16.))
                .child(el)
                .into_any_element(),
            Body::Later(text) => notice(&t, text),
            Body::Waiting => notice(&t, "These settings are not ready yet."),
        };
        div()
            .id("settings-root")
            .debug_selector(|| "settings-root".into())
            .key_context("NixaSettings")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_row()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(13.))
            .on_action(cx.listener(|_, _: &ClosePane, window, cx| close(window, cx)))
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    close(window, cx);
                }
            }))
            .child(list)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pt(px(40.))
                    .px(px(32.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(t.dim)
                            .child(SharedString::from(format!(
                                "{} settings",
                                group_of(current).unwrap_or("Nixa")
                            ))),
                    )
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.bright)
                            .child(current),
                    )
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests;
