use crate::actions::{
    ClosePane, CloseTab, FontBigger, FontReset, FontSmaller, GoPane, Hide, HideOthers, Listen,
    MenuFavourites, MenuProjectCommands, MenuTasks, NewAgentMenu, NewSpace, NewTab, NextAgent,
    NextSpace, NextTab, NixaScreen, OpenCommands, OpenFavourites, OpenHelp, OpenLayouts, OpenMenu,
    OpenPicker, OpenSettings, PickSpace, PickTab, PrevAgent, PrevSpace, PrevTab, Quit, RenameSpace,
    RenameTab, SplitDown, SplitRight, SwapPane, ToggleNixa, ToggleSidebar, ZoomPane,
};
use crate::pane_moves::{self, Dir};
use gpui::{Action, NoAction};

fn split(name: &str) -> (&str, Option<&str>) {
    match name.split_once(':') {
        Some((base, arg)) => (base, Some(arg)),
        None => (name, None),
    }
}

fn number(arg: Option<&str>) -> Option<usize> {
    arg.and_then(|a| a.parse::<usize>().ok())
        .filter(|n| (1..=9).contains(n))
}

fn dir(arg: Option<&str>) -> Option<Dir> {
    arg.and_then(pane_moves::from_word)
}

pub fn action(name: &str) -> Option<Box<dyn Action>> {
    if crate::keys::table::is_off(name) {
        return Some(Box::new(NoAction));
    }
    let (base, arg) = split(name);
    let made: Box<dyn Action> = match (base, arg) {
        ("app.quit", None) => Box::new(Quit),
        ("app.hide", None) => Box::new(Hide),
        ("app.hide_others", None) => Box::new(HideOthers),
        ("app.settings", None) => Box::new(OpenSettings),
        ("app.palette", None) => Box::new(OpenCommands),
        ("app.help", None) => Box::new(OpenHelp),
        ("app.layouts", None) => Box::new(OpenLayouts),
        ("app.zoom_in", None) => Box::new(FontBigger),
        ("app.zoom_out", None) => Box::new(FontSmaller),
        ("app.zoom_reset", None) => Box::new(FontReset),
        ("tab.new", None) => Box::new(NewTab),
        ("tab.close", None) => Box::new(CloseTab),
        ("tab.next", None) => Box::new(NextTab),
        ("tab.prev", None) => Box::new(PrevTab),
        ("tab.rename", None) => Box::new(RenameTab),
        ("tab.select", Some(_)) => Box::new(PickTab {
            number: number(arg)?,
        }),
        ("space.new", None) => Box::new(NewSpace),
        ("space.rename", None) => Box::new(RenameSpace),
        ("space.next", None) => Box::new(NextSpace),
        ("space.prev", None) => Box::new(PrevSpace),
        ("space.select", Some(_)) => Box::new(PickSpace {
            number: number(arg)?,
        }),
        ("agent.next", None) => Box::new(NextAgent),
        ("agent.prev", None) => Box::new(PrevAgent),
        ("agent.new_menu", None) => Box::new(NewAgentMenu),
        ("pane.split", Some("right")) => Box::new(SplitRight),
        ("pane.split", Some("down")) => Box::new(SplitDown),
        ("pane.close", None) => Box::new(ClosePane),
        ("pane.zoom", None) => Box::new(ZoomPane),
        ("pane.go", Some(_)) => Box::new(GoPane { dir: dir(arg)? }),
        ("pane.swap", Some(_)) => Box::new(SwapPane { dir: dir(arg)? }),
        ("layout.toggle", Some("sidebar")) => Box::new(ToggleSidebar),
        ("menu.open", None) => Box::new(OpenMenu),
        ("menu.tasks", None) => Box::new(MenuTasks),
        ("menu.project_commands", None) => Box::new(MenuProjectCommands),
        ("menu.favourites", None) => Box::new(MenuFavourites),
        ("picker.open", None) => Box::new(OpenPicker),
        ("favourites.open", None) => Box::new(OpenFavourites),
        ("nixa.screen", None) => Box::new(NixaScreen),
        ("nixa.listen", None) => Box::new(Listen),
        ("nixa.toggle", None) => Box::new(ToggleNixa),
        _ => return None,
    };
    Some(made)
}

fn side(arg: Option<&str>) -> &'static str {
    match dir(arg) {
        Some(Dir::Left) => "on the left",
        Some(Dir::Right) => "on the right",
        Some(Dir::Up) => "above",
        Some(Dir::Down) => "below",
        None => "",
    }
}

pub fn label(name: &str) -> String {
    if crate::keys::table::is_off(name) {
        return "Turned off".to_string();
    }
    let (base, arg) = split(name);
    let fixed = match (base, arg) {
        ("tab.select", Some(n)) => return format!("Go to tab {n}"),
        ("space.select", Some(n)) => return format!("Go to space {n}"),
        ("pane.go", Some(_)) => return format!("Move to the pane {}", side(arg)),
        ("pane.swap", Some(_)) => return format!("Swap with the pane {}", side(arg)),
        ("app.quit", _) => "Quit Nixa",
        ("app.hide", _) => "Hide Nixa",
        ("app.hide_others", _) => "Hide other apps",
        ("app.settings", _) => "Open settings",
        ("app.palette", _) => "Open the command list",
        ("app.help", _) => "Show this key list",
        ("app.layouts", _) => "Open the layouts window",
        ("app.zoom_in", _) => "Bigger terminal text",
        ("app.zoom_out", _) => "Smaller terminal text",
        ("app.zoom_reset", _) => "Normal terminal text",
        ("tab.new", _) => "New tab",
        ("tab.close", _) => "Close the tab",
        ("tab.next", _) => "Next tab",
        ("tab.prev", _) => "Previous tab",
        ("tab.rename", _) => "Rename the tab",
        ("space.new", _) => "New space",
        ("space.rename", _) => "Rename the space",
        ("space.next", _) => "Next space",
        ("space.prev", _) => "Previous space",
        ("agent.next", _) => "Next agent",
        ("agent.prev", _) => "Previous agent",
        ("agent.new_menu", _) => "Start an agent",
        ("pane.split", Some("right")) => "Split right",
        ("pane.split", Some("down")) => "Split down",
        ("pane.close", _) => "Close the pane",
        ("pane.zoom", _) => "Zoom the pane",
        ("layout.toggle", Some("sidebar")) => "Show or hide the sidebar",
        ("menu.open", _) => "Main menu",
        ("menu.tasks", _) => "Tasks",
        ("menu.project_commands", _) => "Project commands",
        ("menu.favourites", _) => "Favourites menu",
        ("picker.open", _) => "Go to a tab, space or agent",
        ("favourites.open", _) => "Favourites window",
        ("nixa.screen", _) => "Nixa screen",
        ("nixa.listen", _) => "Talk to Nixa",
        ("nixa.toggle", _) => "Switch Nixa on or off",
        _ => name,
    };
    fixed.to_string()
}
