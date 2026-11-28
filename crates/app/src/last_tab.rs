use crate::tabs::{self, Tabs};
use crate::workspace::{EngineState, Workspace};
use gpui::Context;
use nixa_protocol::TermId;

pub fn space_of(tabs: &Tabs, id: &TermId) -> Option<String> {
    tabs::tab_of(tabs, id).map(|t| t.space.clone())
}

pub fn needs_tab(tabs: &Tabs, space: &str, active: Option<&str>, known: bool) -> bool {
    known
        && active == Some(space)
        && tabs.pending.is_empty()
        && !tabs.list.iter().any(|t| t.space == space)
}

pub fn refill(ws: &mut Workspace, space: Option<String>, cx: &mut Context<Workspace>) {
    let Some(space) = space else {
        return;
    };
    if ws.engine != EngineState::Up {
        return;
    }
    let known = crate::spaces::find(&ws.spaces, &space).is_some();
    if needs_tab(&ws.tabs, &space, ws.spaces.active.as_deref(), known) {
        ws.new_tab(Vec::new(), None, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Node;
    use crate::tabs::Tab;

    fn tab(id: &str, space: &str, n: u64) -> Tab {
        Tab {
            id: id.into(),
            space: space.into(),
            root: Node::Leaf(TermId::from_number(n)),
            focus: TermId::from_number(n),
            zoom: None,
        }
    }

    #[test]
    fn an_empty_active_space_gets_a_new_tab() {
        let tabs = Tabs::default();
        assert!(needs_tab(&tabs, "s1", Some("s1"), true));
    }

    #[test]
    fn no_new_tab_when_tabs_are_left_or_coming() {
        let mut tabs = Tabs::default();
        tabs.list.push(tab("t1", "s1", 1));
        assert!(!needs_tab(&tabs, "s1", Some("s1"), true));
        assert!(needs_tab(&tabs, "s2", Some("s2"), true));
        tabs.pending.push("t2".into());
        assert!(!needs_tab(&tabs, "s2", Some("s2"), true));
    }

    #[test]
    fn no_new_tab_for_a_closed_or_hidden_space() {
        let tabs = Tabs::default();
        assert!(!needs_tab(&tabs, "s1", Some("s1"), false));
        assert!(!needs_tab(&tabs, "s1", Some("s2"), true));
        assert!(!needs_tab(&tabs, "s1", None, true));
    }

    #[test]
    fn the_space_comes_from_the_tab_that_holds_the_pane() {
        let mut tabs = Tabs::default();
        tabs.list.push(tab("t1", "s1", 1));
        assert_eq!(space_of(&tabs, &TermId::from_number(1)).as_deref(), Some("s1"));
        assert_eq!(space_of(&tabs, &TermId::from_number(2)), None);
    }
}
