use super::items::{TABS, Tab};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Menu,
    Projects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nav {
    pub tab: Tab,
    pub at: usize,
    pub view: View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Moved,
    Choose(usize),
    Close,
    OpenSettings,
    Ignore,
}

impl Nav {
    pub fn new(tab: Tab) -> Nav {
        Nav {
            tab,
            at: 0,
            view: View::Menu,
        }
    }

    pub fn pick_tab(&mut self, tab: Tab) -> Outcome {
        if tab == Tab::Settings {
            return Outcome::OpenSettings;
        }
        self.tab = tab;
        self.view = View::Menu;
        self.at = 0;
        Outcome::Moved
    }
}

pub fn key(nav: &mut Nav, key: &str, shift: bool, count: usize) -> Outcome {
    match key {
        "escape" => {
            if nav.view != View::Menu {
                nav.view = View::Menu;
                nav.at = 0;
                Outcome::Moved
            } else {
                Outcome::Close
            }
        }
        "tab" | "left" | "right" => {
            let back = key == "left" || (key == "tab" && shift);
            let n = TABS.len();
            let now = nav.tab.index();
            let next = if back {
                (now + n - 1) % n
            } else {
                (now + 1) % n
            };
            nav.pick_tab(TABS[next])
        }
        "down" | "up" | "enter" if count == 0 => Outcome::Ignore,
        "down" => {
            nav.at = (nav.at + 1) % count;
            Outcome::Moved
        }
        "up" => {
            nav.at = (nav.at + count - 1) % count;
            Outcome::Moved
        }
        "enter" => Outcome::Choose(nav.at.min(count - 1)),
        _ => Outcome::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_wrap_round_the_list() {
        let mut nav = Nav::new(Tab::Quick);
        assert_eq!(key(&mut nav, "up", false, 4), Outcome::Moved);
        assert_eq!(nav.at, 3);
        key(&mut nav, "down", false, 4);
        assert_eq!(nav.at, 0);
        key(&mut nav, "down", false, 4);
        assert_eq!(key(&mut nav, "enter", false, 4), Outcome::Choose(1));
        assert_eq!(key(&mut nav, "enter", false, 0), Outcome::Ignore);
        assert_eq!(key(&mut nav, "x", false, 4), Outcome::Ignore);
    }

    #[test]
    fn tab_and_arrows_switch_tabs_and_settings_opens_the_window() {
        let mut nav = Nav::new(Tab::Quick);
        nav.at = 5;
        assert_eq!(key(&mut nav, "tab", false, 9), Outcome::Moved);
        assert_eq!((nav.tab, nav.at), (Tab::Create, 0));
        key(&mut nav, "right", false, 9);
        key(&mut nav, "right", false, 9);
        assert_eq!(nav.tab, Tab::Workspace);
        assert_eq!(key(&mut nav, "right", false, 9), Outcome::OpenSettings);
        assert_eq!(nav.tab, Tab::Workspace);
        assert_eq!(key(&mut nav, "tab", true, 9), Outcome::Moved);
        assert_eq!(nav.tab, Tab::Layout);
        let mut first = Nav::new(Tab::Quick);
        assert_eq!(key(&mut first, "left", false, 9), Outcome::OpenSettings);
    }

    #[test]
    fn escape_leaves_projects_first_then_closes() {
        let mut nav = Nav::new(Tab::Quick);
        nav.view = View::Projects;
        nav.at = 2;
        assert_eq!(key(&mut nav, "escape", false, 3), Outcome::Moved);
        assert_eq!((nav.view, nav.at), (View::Menu, 0));
        assert_eq!(key(&mut nav, "escape", false, 3), Outcome::Close);
    }
}
