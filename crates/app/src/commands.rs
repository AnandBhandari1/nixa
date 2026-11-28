mod navigation;

use crate::layout::Axis;
use crate::links::procs::Cmd as ProcCmd;
use crate::screens::{agents, chat, ctxmenu, favourites, picker, question, spaces, tabbar as bar};
use crate::tabbar::QUICK_AGENTS;
use crate::workspace::{PanelKind, Workspace};
use crate::{hub, panel};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, Focusable, KeyDownEvent, MouseButton, ScrollHandle, SharedString, Subscription,
    Window, div, px,
};
use nixa_ui::metrics::RADIUS_POPOVER;
use nixa_ui::popover::shadow;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, theme};

const MAX_ROWS: usize = 14;
const ROW_PITCH: f32 = 31.0;

pub struct Palette {
    pub input: Entity<TextInput>,
    pub query: String,
    pub selected: usize,
    pub quick: Option<Vec<usize>>,
    scroll: ScrollHandle,
    _events: Subscription,
}

pub fn open_with(quick: Option<Vec<usize>>, window: &mut Window, cx: &mut Context<Workspace>) -> Palette {
    navigation::bind(cx);
    let input = cx.new(|cx| TextInput::new(cx, "Type a command").with_icon(Icon::Search));
    let events = cx.subscribe_in(&input, window, on_input);
    window.focus(&input.focus_handle(cx));
    Palette {
        input,
        query: String::new(),
        selected: 0,
        quick,
        scroll: ScrollHandle::new(),
        _events: events,
    }
}

fn on_input(
    ws: &mut Workspace,
    _: &Entity<TextInput>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    match event {
        InputEvent::Changed(query) => {
            if let Some(palette) = ws.palette.as_mut() {
                palette.query = query.clone();
                palette.selected = 0;
                palette.scroll.scroll_to_item(0);
            }
            cx.notify();
        }
        InputEvent::Submit(_) => choose(ws, window, cx),
        InputEvent::Cancel => ws.close_palette(window, cx),
    }
}

fn choose(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(palette) = ws.palette.as_ref() else {
        return;
    };
    let pick = palette.selected;
    let list = filtered(ws, &palette.query, cx);
    if let Some(chosen) = list.get(pick) {
        let cmd = chosen.cmd.clone();
        ws.close_palette(window, cx);
        run(ws, cmd, window, cx);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    NewTab,
    SplitRight,
    SplitDown,
    ClosePane,
    CloseTab,
    RenameTab,
    StepTab(isize),
    StepSpace(isize),
    Keys,
    Layouts,
    Spaces(spaces::Action),
    ToggleSidebar,
    Panel(PanelKind),
    Settings(&'static str),
    Space(String),
    Tab(String),
    Agent(usize),
    ZoomPane,
    Chat(chat::Action),
    Agents(agents::Action),
    Question(question::QuestionCommand),
    Events(crate::screens::events::EventsCommand),
    CustomAgents(crate::screens::custom_agents::CustomAgentsCommand),
    GitAi(crate::screens::git::ai::GitAiCommand),
    Tasks(crate::screens::tasks::TasksCommand),
    Models(crate::screens::models::ModelsCommand),
    Workflows(crate::screens::workflows::WorkflowsCommand),
    Assistant(crate::screens::assistant::AssistantCommand),
    Desk(crate::screens::desk_panels::Panel),
    FirstRun(crate::screens::first_run::Action),
    History(crate::screens::git::history::HistoryCommand),
    Bar(bar::Action),
    Brain(ProcCmd),
    Menu(crate::screens::menu::Command),
    Files(crate::screens::files::Command),
    Editor(nixa_editor::view::EditorCommand),
    Changes(crate::screens::git::changes::Command),
    Rail(crate::screens::rail::RailAction),
    Picker,
    Favourites,
    Actions,
    Theme(isize),
    NixaScreen,
    Screen(crate::screens::nixa_screen::Command),
    Git(crate::screens::git::accounts::GitCommand),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub hint: &'static str,
    pub cmd: Cmd,
}

fn item(label: impl Into<String>, hint: &'static str, cmd: Cmd) -> Item {
    Item {
        label: label.into(),
        hint,
        cmd,
    }
}

pub fn items(ws: &Workspace) -> Vec<Item> {
    let mut list = vec![
        item("New tab", "⌘T", Cmd::NewTab),
        item("Split right", "⌘D", Cmd::SplitRight),
        item("Split down", "⇧⌘D", Cmd::SplitDown),
        item("Close pane", "⇧⌘W", Cmd::ClosePane),
        item("Close tab", "⌥W", Cmd::CloseTab),
        item("Rename tab", "F2", Cmd::RenameTab),
        item("Next tab", "⌃⇥", Cmd::StepTab(1)),
        item("Previous tab", "⌃⇧⇥", Cmd::StepTab(-1)),
        item("Next space", "⌥⌘]", Cmd::StepSpace(1)),
        item("Previous space", "⌥⌘[", Cmd::StepSpace(-1)),
        item("Show the keys", "⌘/", Cmd::Keys),
        item("Layouts", "", Cmd::Layouts),
        item("Zoom pane", "⇧⌘↩", Cmd::ZoomPane),
        item("Show or hide the sidebar", "⌘B", Cmd::ToggleSidebar),
        item(picker::view::commands()[0].0, "⌘P", Cmd::Picker),
        item("Open the Nixa screen", "⌥⇧M", Cmd::NixaScreen),
        item("Next theme", "", Cmd::Theme(1)),
        item("Previous theme", "", Cmd::Theme(-1)),
    ];
    for (label, command) in crate::screens::nixa_screen::commands().into_iter().skip(1) {
        list.push(item(label, "", Cmd::Screen(command)));
    }
    for (label, action) in spaces::commands() {
        let hint = if action == spaces::Action::Rename { "⇧F2" } else { "" };
        list.push(item(format!("Space: {label}"), hint, Cmd::Spaces(action)));
    }
    for (label, _) in favourites::view::commands() {
        list.push(item(label, "⌥F", Cmd::Favourites));
    }
    for (label, _, _) in ctxmenu::view::commands() {
        list.push(item(label, "", Cmd::Actions));
    }
    for kind in [
        PanelKind::Files,
        PanelKind::Git,
        PanelKind::Nixa,
    ] {
        list.push(item(
            format!("Panel: {}", panel::title(kind)),
            "",
            Cmd::Panel(kind),
        ));
    }
    for (label, action) in chat::commands() {
        list.push(item(format!("Nixa: {label}"), "", Cmd::Chat(action)));
    }
    for (label, action) in bar::commands() {
        if action == bar::Action::NixaScreen {
            continue;
        }
        let hint = match action {
            bar::Action::NewMenu => "⌥A",
            bar::Action::ToggleNixa => "⌥⇧V",
            _ => "",
        };
        list.push(item(label, hint, Cmd::Bar(action)));
    }
    for (label, action) in agents::commands() {
        let hint = match action {
            agents::Action::NextAgent => "⌘]",
            agents::Action::PrevAgent => "⌘[",
            _ => "",
        };
        list.push(item(format!("Agents: {label}"), hint, Cmd::Agents(action)));
    }
    for (label, _, command) in question::commands() {
        list.push(item(label, "", Cmd::Question(command)));
    }
    for (label, _, command) in crate::screens::custom_agents::commands() {
        list.push(item(label, "", Cmd::CustomAgents(command)));
    }
    for (label, _, command) in crate::screens::tasks::commands() {
        list.push(item(label, "", Cmd::Tasks(command)));
    }
    for (label, _, command) in crate::screens::models::commands() {
        list.push(item(label, "", Cmd::Models(command)));
    }
    for (label, _, command) in crate::screens::workflows::commands() {
        list.push(item(label, "", Cmd::Workflows(command)));
    }
    for (label, _, command) in crate::screens::assistant::commands() {
        list.push(item(label, "", Cmd::Assistant(command)));
    }
    for (label, _, panel) in crate::screens::desk_panels::commands() {
        list.push(item(label, "", Cmd::Desk(panel)));
    }
    for (label, _, action) in crate::screens::first_run::commands() {
        list.push(item(label, "", Cmd::FirstRun(action)));
    }
    for (label, _, command) in crate::screens::git::history::commands() {
        list.push(item(label, "", Cmd::History(command)));
    }
    for (label, _, command) in crate::screens::git::ai::commands() {
        list.push(item(label, "", Cmd::GitAi(command)));
    }
    for (label, _, command) in crate::screens::events::commands() {
        use crate::screens::events::EventsCommand::{AgentVoiceOff, AgentVoiceOn};
        if !matches!(command, AgentVoiceOn | AgentVoiceOff) {
            list.push(item(label, "", Cmd::Events(command)));
        }
    }
    for (label, command) in crate::screens::files::commands() {
        list.push(item(label, "", Cmd::Files(command)));
    }
    for (label, command) in crate::screens::git::changes::commands() {
        list.push(item(label, "", Cmd::Changes(command)));
    }
    for (label, action) in crate::screens::rail::commands() {
        list.push(item(label, "", Cmd::Rail(action)));
    }
    list.push(item(
        "Restart the Nixa brain",
        "",
        Cmd::Brain(ProcCmd::RestartBrain),
    ));
    list.push(item(
        "Stop the Nixa brain",
        "",
        Cmd::Brain(ProcCmd::StopBrain),
    ));
    for (label, command) in crate::screens::menu::commands() {
        use crate::screens::menu::{Action, Command, Tab};
        let hint = match command {
            Command::Open(Tab::Quick) => "⌥M",
            Command::Run(Action::Tasks) => "⌥T",
            Command::Run(Action::ProjectCommands) => "⌥P",
            _ => "",
        };
        list.push(item(label, hint, Cmd::Menu(command)));
    }
    for (label, _, command) in crate::screens::git::accounts::commands() {
        list.push(item(label, "", Cmd::Git(command)));
    }
    list.push(item("Settings", "⌘,", Cmd::Settings("General")));
    for part in crate::screens::settings::all().skip(1) {
        list.push(item(format!("Settings: {part}"), "", Cmd::Settings(part)));
    }
    for i in quick_installed(ws) {
        if let Some(agent) = QUICK_AGENTS.get(i) {
            list.push(item(format!("Start {}", agent.name), "", Cmd::Agent(i)));
        }
    }
    for space in &ws.spaces.spaces {
        list.push(item(
            format!("Space: {}", space.name),
            "",
            Cmd::Space(space.id.clone()),
        ));
    }
    for tab in &ws.tabs.list {
        list.push(item(
            format!("Tab: {}", ws.tab_name(tab)),
            "",
            Cmd::Tab(tab.id.clone()),
        ));
    }
    list
}

pub fn quick_installed(ws: &Workspace) -> Vec<usize> {
    match ws.palette.as_ref().and_then(|p| p.quick.clone()) {
        Some(list) => list,
        None => installed_now(),
    }
}

pub fn installed_now() -> Vec<usize> {
    bar::quick::buttons_in(
        &crate::programs::default_path(),
        &crate::programs::default_dirs(),
    )
    .into_iter()
    .map(|b| b.index)
    .collect()
}

pub fn matches(label: &str, query: &str) -> bool {
    let label = label.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| label.contains(word))
}

struct PaletteEditor(gpui::WindowHandle<nixa_editor::Editor>);

impl gpui::Global for PaletteEditor {}

pub fn target_editor(handle: gpui::WindowHandle<nixa_editor::Editor>, cx: &mut gpui::App) {
    cx.set_global(PaletteEditor(handle));
}

fn editor_window(cx: &gpui::App) -> Option<gpui::WindowHandle<nixa_editor::Editor>> {
    let target = cx.try_global::<PaletteEditor>().map(|target| target.0);
    let windows = cx.windows();
    target.filter(|target| windows.iter().any(|window| window.window_id() == target.window_id()))
        .or_else(|| windows.into_iter().find_map(|window| window.downcast::<nixa_editor::Editor>()))
}

pub fn filtered(ws: &Workspace, query: &str, cx: &gpui::App) -> Vec<Item> {
    let editors = editor_window(cx).into_iter().flat_map(|_| nixa_editor::view::commands())
        .map(|(label, hint, command)| item(label, hint, Cmd::Editor(command)));
    items(ws)
        .into_iter().chain(editors)
        .filter(|i| matches(&i.label, query))
        .collect()
}

pub fn key(
    ws: &mut Workspace,
    event: &KeyDownEvent,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    move_selection(ws, event.keystroke.key.as_str(), cx);
}

fn move_selection(ws: &mut Workspace, key: &str, cx: &mut Context<Workspace>) {
    let Some(query) = ws.palette.as_ref().map(|p| p.query.clone()) else {
        return;
    };
    let count = filtered(ws, &query, cx).len();
    let Some(palette) = ws.palette.as_mut() else {
        return;
    };
    let viewport = f32::from(palette.scroll.bounds().size.height);
    let page = ((viewport / ROW_PITCH).floor() as usize).max(1);
    let Some(selected) = navigation::selected(key, palette.selected, count, page) else {
        return;
    };
    palette.selected = selected;
    palette.scroll.scroll_to_item(selected);
    cx.stop_propagation();
    cx.notify();
}

pub fn run(ws: &mut Workspace, cmd: Cmd, window: &mut Window, cx: &mut Context<Workspace>) {
    match cmd {
        Cmd::NewTab => ws.new_tab(Vec::new(), None, cx),
        Cmd::SplitRight => ws.split(Axis::Row, cx),
        Cmd::SplitDown => ws.split(Axis::Column, cx),
        Cmd::ClosePane => ws.close_pane(cx),
        Cmd::CloseTab => crate::actions::close_active_tab(ws, cx),
        Cmd::RenameTab => crate::actions::rename_tab(ws, cx),
        Cmd::StepTab(step) => ws.step_tab(step, cx),
        Cmd::StepSpace(step) => crate::actions::step_space(ws, step, cx),
        Cmd::Keys => crate::keys::help::open(cx),
        Cmd::Layouts => crate::named_layouts::open(cx),
        Cmd::Spaces(action) => {
            let view = ws.spaces_view.clone();
            let space = ws.spaces.active.clone();
            spaces::run(&view, action, space.as_deref(), window, cx);
        }
        Cmd::Editor(command) => {
            if let Some(editor) = editor_window(cx) {
                cx.defer(move |cx| {
                    editor.update(cx, |editor, window, cx| {
                        window.activate_window();
                        nixa_editor::view::run_command(editor, command, window, cx);
                    }).ok();
                });
            }
        }
        Cmd::Files(command) => {
            ws.set_panel(Some(PanelKind::Files), cx);
            let view = ws.files.clone();
            crate::screens::files::run(&view, command, window, cx);
        }
        Cmd::Changes(command) => {
            ws.set_panel(Some(PanelKind::Git), cx);
            let view = ws.changes.clone();
            crate::screens::git::changes::run(&view, command, window, cx);
        }
        Cmd::Rail(action) => crate::screens::rail::host::act(ws, action, cx),
        Cmd::CustomAgents(command) => crate::screens::custom_agents::run(command, cx),
        Cmd::GitAi(command) => {
            let repo = ws.active_space().map(|s| std::path::PathBuf::from(&s.folder));
            crate::screens::git::ai::run(command, repo, cx);
        }
        Cmd::Tasks(command) => crate::screens::tasks::run(command, cx),
        Cmd::Models(command) => crate::screens::models::run(command, cx),
        Cmd::Workflows(command) => crate::screens::workflows::run(command, cx),
        Cmd::Assistant(command) => crate::screens::assistant::run(command, cx),
        Cmd::Desk(panel) => crate::screens::desk_panels::run(panel, cx),
        Cmd::FirstRun(action) => crate::screens::first_run::host::run(action, cx),
        Cmd::History(command) => {
            let folder = ws.active_space().map(|s| s.folder.clone());
            crate::screens::git::history::run(command, folder.as_deref(), cx);
        }
        Cmd::Picker => ws.open_picker(window, cx),
        Cmd::Favourites => ws.open_favourites(window, cx),
        Cmd::Actions => ws.open_ctx(ctxmenu::Target::Default, window.mouse_position(), window, cx),
        Cmd::Theme(step) => ws.step_theme(step, cx),
        Cmd::Git(command) => {
            let folder = ws.active_space().map(|s| s.folder.clone()).filter(|f| !f.is_empty());
            crate::screens::git::accounts::run(command, folder, cx)
        }
        Cmd::NixaScreen => crate::screens::nixa_screen::host::open(ws, cx),
        Cmd::Screen(command) => crate::screens::nixa_screen::host::run(ws, command, cx),
        Cmd::ToggleSidebar => ws.toggle_sidebar(cx),
        Cmd::Panel(kind) => ws.toggle_panel(kind, cx),
        Cmd::Settings(part) => ws.open_settings(part, cx),
        Cmd::Space(id) => ws.select_space(&id, cx),
        Cmd::Tab(id) => ws.select_tab(&id, cx),
        Cmd::ZoomPane => ws.zoom_focused(cx),
        Cmd::Chat(action) => {
            ws.set_panel(Some(PanelKind::Nixa), cx);
            chat::run(&ws.hub.chat, action, window, cx);
            cx.notify();
        }
        Cmd::Agents(action) => agents::run(&ws.hub.agents, action, cx),
        Cmd::Question(command) => {
            ws.set_panel(Some(PanelKind::Nixa), cx);
            ws.hub.question.update(cx, |view, cx| view.run(command, cx));
            cx.notify();
        }
        Cmd::Bar(action) => bar::run(ws, action, window, cx),
        Cmd::Menu(command) => ws.menu_command(command, window, cx),
        Cmd::Brain(command) => {
            hub::command(ws, command);
            cx.notify();
        }
        Cmd::Events(command) => {
            let view = ws.hub.events.events.clone();
            view.update(cx, |v, cx| v.run(command, cx));
            if command == crate::screens::events::EventsCommand::Show {
                window.focus(&view.focus_handle(cx));
            }
            cx.notify();
        }
        Cmd::Agent(i) => {
            use crate::screens::tabbar::quick::{button_in, start};
            if let Some((name, run)) = button_in(&crate::programs::default_path(), i)
                .and_then(|button| start(button, None))
            {
                ws.new_tab(run, Some(name), cx);
            }
        }
    }
}

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(palette) = ws.palette.as_ref() else {
        return div().into_any_element();
    };
    let t = theme(cx).clone();
    let list = filtered(ws, &palette.query, cx);
    let selected = palette.selected.min(list.len().saturating_sub(1));
    let hover = t.hover;
    let rows: Vec<AnyElement> = list
        .into_iter()
        .enumerate()
        .map(|(i, it)| {
            let cmd = it.cmd.clone();
            let on = i == selected;
            div()
                .id(("palette-row", i))
                .h(px(30.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .when(on, |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |ws, _, window, cx| {
                    ws.close_palette(window, cx);
                    run(ws, cmd.clone(), window, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(if on { t.bright } else { t.muted })
                        .child(SharedString::from(it.label)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(t.dim)
                        .child(it.hint),
                )
                .into_any_element()
        })
        .collect();
    let empty = rows.is_empty();
    let list_height = px(12.0 + (rows.len().clamp(1, MAX_ROWS) as f32) * ROW_PITCH);
    let rows = div()
        .id("palette-results")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&palette.scroll)
        .p(px(6.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .children(rows)
        .when(empty, |el| el.child(div().px(px(10.)).py(px(8.)).text_color(t.dim).child("No command matches.")));
    div()
        .id("palette-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(80.))
        .bg(t.scrim)
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|ws, _, window, cx| ws.close_palette(window, cx)),
        )
        .child(
            div()
                .id("palette")
                .key_context(navigation::CONTEXT)
                .w(px(560.))
                .h(px(44.) + list_height)
                .max_h_full()
                .flex()
                .flex_col()
                .rounded(px(RADIUS_POPOVER))
                .border_1()
                .border_color(t.border)
                .bg(t.chrome)
                .shadow(shadow(&t))
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .capture_key_down(cx.listener(key))
                .on_action(cx.listener(|ws, _: &navigation::FirstCommand, _, cx| move_selection(ws, "home", cx)))
                .on_action(cx.listener(|ws, _: &navigation::LastCommand, _, cx| move_selection(ws, "end", cx)))
                .child(
                    div()
                        .p(px(6.))
                        .border_b_1()
                        .border_color(t.border)
                        .child(palette.input.clone()),
                )
                .child(
                    div().relative().flex().flex_col().flex_1().min_h_0()
                        .child(rows)
                        .child(nixa_ui::scroll::scrollbar(&t, &palette.scroll)),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_match() {
        assert!(matches("Split right", ""));
        assert!(matches("Split right", "sp ri"));
        assert!(matches("Settings: Chat brain", "chat set"));
        assert!(!matches("New tab", "split"));
    }
}
