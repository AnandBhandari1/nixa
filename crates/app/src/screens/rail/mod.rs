pub mod host;
#[cfg(test)]
mod tests;

use crate::workspace::PanelKind;
use gpui::prelude::*;
use gpui::{App, Div, Stateful, Window, div, px};
use nixa_store::Connection;
use nixa_ui::icon_button::{IconSize, count_badge, icon_button, tinted_icon_button};
use nixa_ui::metrics::RAIL_WIDTH;
use nixa_ui::popover::tooltip;
use nixa_ui::section::short_rule;
use nixa_ui::{Icon, Theme};
use std::rc::Rc;

pub const COMMAND_MENU: &str = r#"if command -v cmdmenu >/dev/null 2>&1; then
  cmdmenu --root "$PWD"
elif [ -x "$HOME/.local/bin/cmdmenu" ]; then
  "$HOME/.local/bin/cmdmenu" --root "$PWD"
else
  printf '%s\n' 'cmdmenu is not installed on this machine. Install cmdmenu and put it in PATH or ~/.local/bin/cmdmenu.'
fi
exec "${SHELL:-/bin/sh}" -l"#;

pub const REPEATS_PART: &str = "Repeat messages";
pub const AGENTS_PART: &str = "Agents and team";
pub const GENERAL_PART: &str = "General";
pub const SCHEDULES_PART: &str = "Schedules";
pub const RULES_PART: &str = "Rules";
pub const MEMORY_PART: &str = "Memory";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailAction {
    Commands,
    Tasks,
    Panel(PanelKind),
    Repeats,
    DeepBrain,
    Agents,
    Workflows,
    Automate,
    Rules,
    Memory,
    Settings,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RailState {
    pub panel: Option<PanelKind>,
    pub panels: Vec<PanelKind>,
    pub tasks_waiting: usize,
    pub folder: Option<String>,
    pub space: Option<String>,
}

pub type OnRail = Rc<dyn Fn(RailAction, &mut Window, &mut App)>;

pub const PANELS: [PanelKind; 2] = [PanelKind::Files, PanelKind::Git];

pub fn state(panel: Option<PanelKind>, tasks_waiting: usize, folder: Option<String>) -> RailState {
    RailState {
        panel,
        panels: PANELS.to_vec(),
        tasks_waiting,
        folder,
        space: None,
    }
}

pub fn command_menu_argv() -> Vec<String> {
    vec!["sh".into(), "-lc".into(), COMMAND_MENU.into()]
}

pub fn tasks_waiting(conn: &Connection) -> usize {
    let Ok(tasks) = nixa_store::tasks::waiting_tasks(conn) else {
        return 0;
    };
    let locked: Vec<String> = nixa_store::spaces::list_spaces(conn)
        .map(|list| {
            list.into_iter()
                .filter(|s| s.locked)
                .map(|s| s.space_id)
                .collect()
        })
        .unwrap_or_default();
    tasks
        .iter()
        .filter(|t| t.space_id.is_empty() || !locked.contains(&t.space_id))
        .count()
}

pub fn panel_icon(kind: PanelKind) -> Icon {
    match kind {
        PanelKind::Files => Icon::Files,
        PanelKind::Git => Icon::GitBranch,
        PanelKind::Nixa => Icon::Chat,
    }
}

pub fn panel_tip(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Files => "Files panel: open or close it",
        PanelKind::Git => {
            "Git panel: changes, graph, branches and stash. Click to open or close it."
        }
        PanelKind::Nixa => "Nixa chat panel",
    }
}

pub fn panel_id(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Files => "nixa-rail-panel-files",
        PanelKind::Git => "nixa-rail-panel-changes",
        PanelKind::Nixa => "nixa-rail-panel-nixa",
    }
}

pub fn tasks_tip(waiting: usize) -> String {
    let head = if waiting > 0 {
        format!("Tasks: {waiting} waiting for you.")
    } else {
        "Tasks.".to_string()
    };
    format!("{head} Opens in its own window. Drag it to another screen, or make it full screen.")
}

pub fn deep_tip(space: Option<&str>) -> String {
    match space {
        Some(name) => format!("Deep brain of space {name}: starts it when it is not running, or tells Nixa it is already up"),
        None => "Deep brain: open a space first".into(),
    }
}

pub fn commands_tip(folder: Option<&str>) -> String {
    match folder {
        Some(f) => format!("Commands: open only this project's cmdmenu in {f}."),
        None => "Commands: select a project with a folder first.".to_string(),
    }
}

pub fn visible_panels(s: &RailState) -> Vec<PanelKind> {
    s.panels
        .iter()
        .copied()
        .filter(|k| *k != PanelKind::Nixa)
        .collect()
}

fn hook(el: Stateful<Div>, on: &OnRail, action: RailAction) -> Stateful<Div> {
    let on = on.clone();
    el.tab_index(0)
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on(action, window, cx)
        })
}

fn plain(
    t: &Theme,
    id: &'static str,
    which: Icon,
    on_state: bool,
    tip: String,
    on: &OnRail,
    action: RailAction,
) -> Stateful<Div> {
    let el = icon_button(t, id, which, IconSize::Rail, on_state)
        .debug_selector(move || id.to_string())
        .tooltip(tooltip(tip));
    hook(el, on, action)
}

pub fn render(s: &RailState, t: &Theme, on: OnRail) -> Stateful<Div> {
    let waiting = s.tasks_waiting;
    let tasks = if waiting > 0 {
        tinted_icon_button(
            t,
            "nixa-rail-tasks",
            Icon::Tasks,
            IconSize::Rail,
            false,
            t.yellow,
        )
        .child(count_badge(t, waiting, t.yellow).debug_selector(|| "nixa-rail-tasks-badge".into()))
    } else {
        icon_button(t, "nixa-rail-tasks", Icon::Tasks, IconSize::Rail, false)
    };
    let tasks = hook(
        tasks
            .debug_selector(|| "nixa-rail-tasks".into())
            .tooltip(tooltip(tasks_tip(waiting))),
        &on,
        RailAction::Tasks,
    );
    let commands = plain(
        t,
        "nixa-rail-commands",
        Icon::Commands,
        false,
        commands_tip(s.folder.as_deref()),
        &on,
        RailAction::Commands,
    )
    .when(s.folder.is_none(), |el| el.opacity(0.5));
    let panels: Vec<Stateful<Div>> = visible_panels(s)
        .into_iter()
        .map(|kind| {
            plain(
                t,
                panel_id(kind),
                panel_icon(kind),
                s.panel == Some(kind),
                panel_tip(kind).into(),
                &on,
                RailAction::Panel(kind),
            )
        })
        .collect();
    div()
        .id("nixa-rail")
        .tab_group()
        .w(px(RAIL_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .py(px(8.))
        .bg(t.chrome)
        .child(commands)
        .child(tasks)
        .child(short_rule(t))
        .children(panels)
        .child(plain(
            t,
            "nixa-rail-repeats",
            Icon::Clock,
            false,
            "Repeat messages: messages sent to Nixa chat by themselves every few minutes or hours. Opens Nixa settings at Repeat messages.".into(),
            &on,
            RailAction::Repeats,
        ))
        .child(
            plain(
                t,
                "nixa-rail-deep",
                Icon::Core,
                false,
                deep_tip(s.space.as_deref()),
                &on,
                RailAction::DeepBrain,
            )
            .when(s.space.is_none(), |el| el.opacity(0.5)),
        )
        .child(div().flex_1().min_h_0())
        .child(plain(
            t,
            "nixa-rail-agents",
            Icon::Users,
            false,
            "Agents: your custom agents and teams, in Nixa settings".into(),
            &on,
            RailAction::Agents,
        ))
        .child(plain(
            t,
            "nixa-rail-workflows",
            Icon::Play,
            false,
            "Workflows: build and edit step by step agent workflows on a canvas".into(),
            &on,
            RailAction::Workflows,
        ))
        .child(plain(
            t,
            "nixa-rail-automate",
            Icon::Zap,
            false,
            "Automate: schedules that open an agent tab with your instructions at set times. Opens Nixa settings at Schedules.".into(),
            &on,
            RailAction::Automate,
        ))
        .child(plain(
            t,
            "nixa-rail-rules",
            Icon::Check,
            false,
            "Rules: standing rules, answer rules and rule trees in one place. Opens Nixa settings at Rules.".into(),
            &on,
            RailAction::Rules,
        ))
        .child(plain(
            t,
            "nixa-rail-memory",
            Icon::Notebook,
            false,
            "Memory: write or edit Nixa's memory notes, then Save. Opens Nixa settings at Memory.".into(),
            &on,
            RailAction::Memory,
        ))
        .child(plain(
            t,
            "nixa-rail-settings",
            Icon::Settings,
            false,
            "Nixa settings: every Nixa setting in one place".into(),
            &on,
            RailAction::Settings,
        ))
}

pub fn commands() -> Vec<(&'static str, RailAction)> {
    vec![
        ("Open the project command menu", RailAction::Commands),
        ("Open the task list", RailAction::Tasks),
        (
            "Open or close the Files panel",
            RailAction::Panel(PanelKind::Files),
        ),
        (
            "Open or close the Git panel",
            RailAction::Panel(PanelKind::Git),
        ),
        ("Open repeat messages", RailAction::Repeats),
        ("Start or check the deep brain of this space", RailAction::DeepBrain),
        ("Open Agents and team settings", RailAction::Agents),
        ("Open Workflows", RailAction::Workflows),
        ("Open Automate schedules", RailAction::Automate),
        ("Open Rules", RailAction::Rules),
        ("Open Memory", RailAction::Memory),
        ("Open Nixa settings", RailAction::Settings),
    ]
}
