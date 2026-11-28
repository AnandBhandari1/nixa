use super::{
    Agent, Custom, MenuEvent, NewMenu, SETTINGS_PART, WIDTH, ask_remove, cancel_remove, choices,
    close_form, emit, height, open_form, pick_icon, profile_text, remove, save, start,
    start_custom,
};
use crate::programs;
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, FontWeight, SharedString, div, px};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::popover::{popover, tooltip};
use nixa_ui::{Icon, Theme, icon, theme};

pub fn mono_of(icon_name: &str) -> String {
    match icon_name {
        "claude" => "CC".into(),
        "codex" => "CX".into(),
        "cursor" => "CU".into(),
        "gemini" => "GE".into(),
        "pi" => "PI".into(),
        "antigravity" => "AG".into(),
        "opencode" => "OC".into(),
        "omo" => "OM".into(),
        "crush" => "CR".into(),
        "jcode" => "JC".into(),
        "hermes" => "HE".into(),
        "dim" => "DI".into(),
        other => other
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_uppercase(),
    }
}

pub fn glyph(t: &Theme, icon_file: Option<&str>, size: f32) -> AnyElement {
    let name = icon_file.map(programs::icon_name).unwrap_or_default();
    if let Some(mark) = nixa_ui::agent_icons::glyph(&name, t.foreground, size) {
        return mark;
    }
    let mono = mono_of(&name);
    if name.is_empty() || name == "agent" || mono.is_empty() {
        return icon(Icon::Hexagon)
            .size(px(size - 2.0))
            .text_color(t.foreground)
            .into_any_element();
    }
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(t.raised)
        .text_size(px(size * 0.5))
        .font_weight(FontWeight::BOLD)
        .text_color(t.bright)
        .child(SharedString::from(mono))
        .into_any_element()
}

fn caption(t: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .px(px(9.0))
        .pt(px(8.0))
        .pb(px(4.0))
        .text_size(px(11.0))
        .text_color(t.dim)
        .child(text.into())
}

fn section(t: &Theme, text: &'static str, top: f32) -> gpui::Div {
    div()
        .px(px(9.0))
        .pt(px(top))
        .pb(px(2.0))
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(text)
}

fn row(
    t: &Theme,
    id: &'static str,
    glyph: AnyElement,
    name: &str,
    hint: &str,
    tip: &str,
) -> gpui::Stateful<gpui::Div> {
    let hover = t.selected;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(31.0))
        .px(px(9.0))
        .gap(px(10.0))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(7.0))
        .text_color(t.foreground)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip.to_string()))
        .child(glyph)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(name.to_string())),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(11.0))
                .text_color(t.dim)
                .child(SharedString::from(hint.to_string())),
        )
}

fn tool(t: &Theme, id: String, which: Icon, tip: String) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let bright = t.foreground;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .size(px(22.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .text_color(t.dim)
        .cursor_pointer()
        .tooltip(tooltip(tip))
        .hover(move |s| s.bg(hover).text_color(bright))
        .child(icon(which).size(px(12.0)).text_color(t.dim))
}

fn agent_row(t: &Theme, menu: &NewMenu, agent: &Agent, cx: &mut Context<NewMenu>) -> AnyElement {
    let key = agent.program.key.clone();
    let mine = menu.mine.contains(&key);
    let place = super::installed(menu)
        .iter()
        .position(|a| a.program.key == key);
    let hint = if agent.installed {
        place
            .filter(|i| *i < 9)
            .map(|i| (i + 1).to_string())
            .unwrap_or_default()
    } else {
        "not installed".into()
    };
    let tip = if agent.installed {
        format!(
            "Start {}: {}",
            agent.program.name,
            programs::join_command(&agent.program.run)
        )
    } else {
        let word = programs::program_word(&agent.program.run).unwrap_or(&agent.program.name);
        format!("{word} is not installed")
    };
    let hover = t.selected;
    let on = agent.installed;
    let go_key = key.clone();
    let go_id = format!("go-{key}");
    div()
        .id(SharedString::from(format!("row-{key}")))
        .h(px(31.0))
        .pl(px(9.0))
        .pr(px(if mine { 4.0 } else { 9.0 }))
        .gap(px(6.0))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(7.0))
        .text_color(if on { t.foreground } else { t.dim })
        .when(on, |el| el.hover(move |s| s.bg(hover)))
        .child(
            div()
                .id(SharedString::from(go_id.clone()))
                .debug_selector(move || go_id.clone())
                .flex_1()
                .min_w_0()
                .h_full()
                .gap(px(10.0))
                .flex()
                .items_center()
                .tooltip(tooltip(tip))
                .when(on, |el| {
                    el.cursor_pointer()
                        .on_click(cx.listener(move |menu, _: &ClickEvent, _, cx| {
                            start(menu, &go_key, cx);
                        }))
                })
                .child(glyph(t, agent.program.icon.as_deref(), 14.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(agent.program.name.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.0))
                        .text_color(t.dim)
                        .child(SharedString::from(hint)),
                ),
        )
        .when(mine, |el| {
            let edit_key = key.clone();
            let drop_key = key.clone();
            el.child(
                tool(
                    t,
                    format!("edit-{key}"),
                    Icon::SquarePen,
                    format!("Edit {}", agent.program.name),
                )
                .on_click(cx.listener(move |menu, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    open_form(menu, Some(&edit_key), window, cx);
                })),
            )
            .child(
                tool(
                    t,
                    format!("remove-{key}"),
                    Icon::X,
                    format!("Remove {} from programs.json", agent.program.name),
                )
                .on_click(cx.listener(move |menu, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    ask_remove(menu, &drop_key, cx);
                })),
            )
        })
        .into_any_element()
}

fn member_row(t: &Theme, custom: &Custom, cx: &mut Context<NewMenu>) -> AnyElement {
    let open = !custom.state.is_empty() && custom.state != "closed";
    let tip = format!(
        "{} in the shown space as {}. {}",
        if open {
            format!("{} is open: start one more tab of it", custom.title)
        } else {
            format!("Start {}", custom.title)
        },
        profile_text(custom),
        custom.purpose
    );
    let hover = t.selected;
    let id = custom.id.clone();
    let selector = format!("custom-{}", custom.id);
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .h(px(36.0))
        .px(px(9.0))
        .flex_none()
        .flex()
        .flex_col()
        .justify_center()
        .rounded(px(7.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip.trim().to_string()))
        .on_click(cx.listener(move |menu, _: &ClickEvent, _, cx| start_custom(menu, &id, cx)))
        .child(
            div()
                .flex()
                .gap(px(8.0))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(t.foreground)
                        .child(SharedString::from(custom.title.clone())),
                )
                .when(open, |el| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(px(10.5))
                            .text_color(t.dim)
                            .child(SharedString::from(custom.state.clone())),
                    )
                }),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .text_size(px(10.5))
                .text_color(t.dim)
                .child(SharedString::from(profile_text(custom))),
        )
        .into_any_element()
}

fn team_rows(t: &Theme, menu: &NewMenu, cx: &mut Context<NewMenu>) -> Vec<AnyElement> {
    if let Some(error) = &menu.team_error {
        return vec![
            div()
                .px(px(9.0))
                .py(px(4.0))
                .text_size(px(11.0))
                .text_color(t.red)
                .debug_selector(|| "team-error".into())
                .child(SharedString::from(format!("Custom agents: {error}")))
                .into_any_element(),
        ];
    }
    if menu.customs.is_empty() {
        return Vec::new();
    }
    let mut out = vec![section(t, "Custom agents", 2.0).into_any_element()];
    out.extend(menu.customs.iter().map(|c| member_row(t, c, cx)));
    out.push(section(t, "Agents", 8.0).into_any_element());
    out
}

fn header(t: &Theme, menu: &NewMenu, cx: &mut Context<NewMenu>) -> AnyElement {
    let hover = t.hover;
    let bright = t.foreground;
    let title = menu
        .form
        .as_ref()
        .map(|f| f.title.clone())
        .unwrap_or_else(|| "New agent".into());
    div()
        .h(px(24.0))
        .pl(px(9.0))
        .pr(px(2.0))
        .flex_none()
        .flex()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.0))
                .text_color(t.dim)
                .child(SharedString::from(title)),
        )
        .child(
            div()
                .id("new-menu-close")
                .debug_selector(|| "new-menu-close".into())
                .h(px(20.0))
                .px(px(6.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .text_size(px(11.5))
                .text_color(t.dim)
                .cursor_pointer()
                .tooltip(tooltip("Close this menu"))
                .hover(move |s| s.bg(hover).text_color(bright))
                .on_click(
                    cx.listener(|menu, _: &ClickEvent, _, cx| emit(menu, MenuEvent::Close, cx)),
                )
                .child("× close"),
        )
        .into_any_element()
}

fn icon_tile(t: &Theme, path: &str, picked: bool, cx: &mut Context<NewMenu>) -> AnyElement {
    let name = programs::icon_name(path);
    let hover = t.hover;
    let file = path.to_string();
    let selector = format!("icon-{name}");
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(if picked { t.accent } else { t.border })
        .when(picked, |el| el.bg(t.selected))
        .cursor_pointer()
        .tooltip(tooltip(name.clone()))
        .hover(move |s| s.bg(hover))
        .on_click(cx.listener(move |menu, _: &ClickEvent, _, cx| pick_icon(menu, file.clone(), cx)))
        .child(glyph(t, Some(path), 14.0))
        .into_any_element()
}

fn field(t: &Theme, input: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .h(px(30.0))
        .px(px(8.0))
        .justify_center()
        .rounded(px(7.0))
        .border_1()
        .border_color(t.input)
        .bg(t.background)
        .text_size(px(12.5))
        .child(input)
        .into_any_element()
}

fn form_view(t: &Theme, menu: &NewMenu, cx: &mut Context<NewMenu>) -> AnyElement {
    let Some(form) = menu.form.as_ref() else {
        return div().into_any_element();
    };
    let label = |text: String| {
        div()
            .pt(px(10.0))
            .pb(px(5.0))
            .text_size(px(11.0))
            .text_color(t.dim)
            .child(SharedString::from(text))
    };
    let icon_label = format!(
        "Icon: {}",
        form.icon
            .as_deref()
            .map(programs::icon_name)
            .unwrap_or_else(|| "none".into())
    );
    let tiles: Vec<AnyElement> = form
        .icons
        .iter()
        .map(|p| icon_tile(t, p, form.icon.as_deref() == Some(p.as_str()), cx))
        .collect();
    let error = menu.error.clone();
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .px(px(9.0))
        .pb(px(4.0))
        .child(label("Name".into()))
        .child(field(t, menu.name_input.clone().into_any_element()))
        .child(label("Command".into()))
        .child(field(t, menu.cmd_input.clone().into_any_element()))
        .child(label(icon_label))
        .child(div().flex().flex_wrap().gap(px(4.0)).children(tiles))
        .child(div().flex_1())
        .when(!error.is_empty(), |el| {
            el.child(
                div()
                    .pt(px(8.0))
                    .text_size(px(11.5))
                    .text_color(t.red)
                    .debug_selector(|| "form-error".into())
                    .child(SharedString::from(error)),
            )
        })
        .child(
            div()
                .pt(px(10.0))
                .flex()
                .gap(px(6.0))
                .justify_end()
                .child(
                    button(t, "form-cancel", "Cancel", ButtonKind::Secondary)
                        .debug_selector(|| "form-cancel".into())
                        .tooltip(tooltip("Back to the menu"))
                        .on_click(cx.listener(|menu, _: &ClickEvent, window, cx| {
                            close_form(menu, cx);
                            window.focus(&menu.focus);
                        })),
                )
                .child(
                    button(
                        t,
                        "form-save",
                        if menu.busy { "Saving…" } else { "Save" },
                        ButtonKind::Primary,
                    )
                    .debug_selector(|| "form-save".into())
                    .tooltip(tooltip("Save to programs.json"))
                    .on_click(cx.listener(|menu, _: &ClickEvent, _, cx| save(menu, cx))),
                ),
        )
        .into_any_element()
}

fn confirm_view(t: &Theme, menu: &NewMenu, key: &str, cx: &mut Context<NewMenu>) -> AnyElement {
    let name = menu
        .agents
        .iter()
        .find(|a| a.program.key == key)
        .map(|a| a.program.name.clone())
        .unwrap_or_else(|| key.to_string());
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(9.0))
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child(SharedString::from(format!("Remove {name}?"))),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.muted)
                .child(SharedString::from(format!(
                    "This deletes the \"{key}\" entry from programs.json. Other agents stay as they are."
                ))),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .justify_end()
                .child(
                    button(t, "remove-cancel", "Cancel", ButtonKind::Secondary)
                        .debug_selector(|| "remove-cancel".into())
                        .on_click(cx.listener(|menu, _: &ClickEvent, _, cx| cancel_remove(menu, cx))),
                )
                .child(
                    button(t, "remove-yes", "Remove", ButtonKind::Danger)
                        .debug_selector(|| "remove-yes".into())
                        .on_click(cx.listener(|menu, _: &ClickEvent, _, cx| remove(menu, cx))),
                ),
        )
        .into_any_element()
}

fn list_view(t: &Theme, menu: &NewMenu, cx: &mut Context<NewMenu>) -> Vec<AnyElement> {
    let rows: Vec<AnyElement> = choices(menu)
        .into_iter()
        .map(|a| agent_row(t, menu, a, cx))
        .collect();
    let empty = rows.is_empty();
    let waiting = if menu.failed {
        "Could not load agents"
    } else if menu.loaded {
        "No agents found"
    } else {
        "Checking installed agents…"
    };
    let team = team_rows(t, menu, cx);
    vec![
        row(
            t,
            "terminal",
            icon(Icon::Terminal)
                .size(px(14.0))
                .text_color(t.foreground)
                .into_any_element(),
            "Terminal",
            "⌘T",
            "Open a terminal in a new tab",
        )
        .on_click(cx.listener(|menu, _: &ClickEvent, _, cx| emit(menu, MenuEvent::Terminal, cx)))
        .into_any_element(),
        caption(t, "Choose an agent").into_any_element(),
        nixa_ui::scroll::scroll_area(t, "new-menu-list", &menu.scroll, div()
            .flex()
            .flex_col()
            .children(team)
            .children(rows)
            .when(empty, |el| {
                el.child(
                    div()
                        .px(px(9.0))
                        .py(px(6.0))
                        .text_color(t.dim)
                        .child(waiting),
                )
            }),
        ).flex_1().into_any_element(),
        div()
            .h(px(1.0))
            .mx(px(4.0))
            .my(px(5.0))
            .flex_none()
            .bg(t.border)
            .into_any_element(),
        row(
            t,
            "add-agent",
            icon(Icon::Plus)
                .size(px(14.0))
                .text_color(t.foreground)
                .into_any_element(),
            "Add agent…",
            "",
            "Add an agent to programs.json",
        )
        .on_click(cx.listener(|menu, _: &ClickEvent, window, cx| open_form(menu, None, window, cx)))
        .into_any_element(),
        row(
            t,
            "settings",
            icon(Icon::Settings)
                .size(px(14.0))
                .text_color(t.foreground)
                .into_any_element(),
            "Nixa settings…",
            "",
            "Open Nixa settings: every Nixa setting in one place",
        )
        .on_click(cx.listener(|menu, _: &ClickEvent, _, cx| {
            emit(menu, MenuEvent::Settings(SETTINGS_PART), cx)
        }))
        .into_any_element(),
        row(
            t,
            "leaderboard",
            icon(Icon::Star)
                .size(px(14.0))
                .text_color(t.foreground)
                .into_any_element(),
            "Model leaderboard",
            "",
            "Open the model leaderboard: which model is best for each job",
        )
        .on_click(cx.listener(|menu, _: &ClickEvent, _, cx| {
            crate::screens::models::open(cx);
            emit(menu, MenuEvent::Close, cx)
        }))
        .into_any_element(),
    ]
}

pub fn render(menu: &NewMenu, cx: &mut Context<NewMenu>) -> AnyElement {
    let t = theme(cx).clone();
    let body: Vec<AnyElement> = if menu.form.is_some() {
        vec![form_view(&t, menu, cx)]
    } else if let Some(key) = menu.confirm.clone() {
        vec![confirm_view(&t, menu, &key, cx)]
    } else {
        list_view(&t, menu, cx)
    };
    popover(&t)
        .id("new-menu")
        .debug_selector(|| "new-menu".into())
        .key_context("NewMenu")
        .track_focus(&menu.focus)
        .w(px(WIDTH))
        .h(px(height(menu)))
        .min_h_0()
        .occlude()
        .on_key_down(cx.listener(super::key))
        .child(header(&t, menu, cx))
        .children(body)
        .into_any_element()
}
