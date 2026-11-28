use nixa_ui::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Quick,
    Create,
    Layout,
    Workspace,
    Settings,
}

pub const TABS: [Tab; 5] = [
    Tab::Quick,
    Tab::Create,
    Tab::Layout,
    Tab::Workspace,
    Tab::Settings,
];

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Quick => "Quick actions",
            Tab::Create => "Create",
            Tab::Layout => "Layout",
            Tab::Workspace => "Workspace",
            Tab::Settings => "Settings",
        }
    }

    pub fn index(self) -> usize {
        TABS.iter().position(|t| *t == self).unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Tasks,
    QuickCapture,
    Board,
    Files,
    Events,
    Favourites,
    Commands,
    NewAgent,
    AgentSettings,
    CustomAgents,
    TasksWindow,
    ProjectCommands,
    Projects,
    Git,
    GitAccounts,
    GitClone,
    NewTab,
    NewSpace,
    NewSpaceInFolder,
    NewWorktree,
    OpenWorktree,
    SplitRight,
    SplitDown,
    ZoomPane,
    MyLayout,
    Layouts,
    ToggleSidebar,
    RenameSpace,
    RenameTab,
    CloseTab,
    CloseSpace,
    Back,
    OpenProject { name: String, path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: &'static str,
    pub icon: Icon,
    pub label: String,
    pub hint: String,
    pub tip: String,
    pub action: Action,
}

fn item(id: &'static str, icon: Icon, label: &str, hint: &str, tip: &str, action: Action) -> Item {
    Item {
        id,
        icon,
        label: label.to_string(),
        hint: hint.to_string(),
        tip: tip.to_string(),
        action,
    }
}

pub fn quick(unread: usize) -> Vec<Item> {
    let (hint, tip) = if unread > 0 {
        (
            format!("{unread} new"),
            format!("Event history: {unread} new"),
        )
    } else {
        (
            "history".to_string(),
            "Event history: agent alerts and notices".to_string(),
        )
    };
    vec![
        item(
            "tasks",
            Icon::Check,
            "Tasks",
            "nixa task",
            "Task list in a popup terminal",
            Action::Tasks,
        ),
        item(
            "quickcapture",
            Icon::Plus,
            "Quick capture",
            "nixa task",
            "Capture a task in a popup terminal",
            Action::QuickCapture,
        ),
        item(
            "board",
            Icon::PanelRight,
            "Board in a new tab",
            "nixa task",
            "Open the task board in a new tab that keeps itself up to date",
            Action::Board,
        ),
        item(
            "files",
            Icon::Files,
            "Files",
            "panel",
            "Open the Files panel",
            Action::Files,
        ),
        Item {
            id: "events",
            icon: Icon::Bell,
            label: "Events".into(),
            hint,
            tip,
            action: Action::Events,
        },
        item(
            "favourites",
            Icon::Pin,
            "Favourites",
            "links",
            "Favourite links and files of this project",
            Action::Favourites,
        ),
        item(
            "commands",
            Icon::Search,
            "Commands",
            "⌘K",
            "Open the command palette",
            Action::Commands,
        ),
        item(
            "agents",
            Icon::Layers,
            "Start an agent",
            "new",
            "Start a new agent",
            Action::NewAgent,
        ),
        item(
            "team",
            Icon::Users,
            "Agents and team settings",
            "settings",
            "Nixa settings at Agents: custom agents and teams",
            Action::AgentSettings,
        ),
        item(
            "custom-agents",
            Icon::Layers,
            "Custom agents",
            "window",
            "Saved agent sessions that Nixa reuses or resumes by purpose",
            Action::CustomAgents,
        ),
        item(
            "tasks-window",
            Icon::Tasks,
            "Tasks window",
            "window",
            "Open Tasks in its own window",
            Action::TasksWindow,
        ),
        item(
            "projectcommands",
            Icon::Terminal,
            "Project commands",
            "⌥P",
            "Project and global commands in a popup terminal",
            Action::ProjectCommands,
        ),
        item(
            "projects",
            Icon::Folder,
            "Projects",
            "new space",
            "Open a project folder as a new space",
            Action::Projects,
        ),
        item(
            "git",
            Icon::GitBranch,
            "Git",
            "panel",
            "Open the Git panel: changes, graph, branches and stash",
            Action::Git,
        ),
        item(
            "gitaccounts",
            Icon::Settings,
            "Git accounts and identity",
            "settings",
            "Open Nixa settings at Git accounts: GitHub accounts, profiles and AI settings for git",
            Action::GitAccounts,
        ),
        item(
            "gitclone",
            Icon::FolderPlus,
            "Clone a git repo",
            "git",
            "Copy a repo from GitHub to this Mac, then open it as a space",
            Action::GitClone,
        ),
    ]
}

pub fn create() -> Vec<Item> {
    vec![
        item(
            "tab",
            Icon::Plus,
            "New tab",
            "⌘T",
            "New tab in this space",
            Action::NewTab,
        ),
        item(
            "space",
            Icon::FolderPlus,
            "New space",
            "picker",
            "Pick a folder for a new space",
            Action::NewSpace,
        ),
        item(
            "space-folder",
            Icon::Folder,
            "New space in folder…",
            "picker",
            "Pick a folder, then open it as a new space",
            Action::NewSpaceInFolder,
        ),
        item(
            "worktree",
            Icon::GitBranch,
            "New worktree…",
            "git",
            "Make a new git worktree",
            Action::NewWorktree,
        ),
        item(
            "worktree-open",
            Icon::GitBranch,
            "Open worktree…",
            "git",
            "Open a git worktree",
            Action::OpenWorktree,
        ),
        item(
            "split-right",
            Icon::PanelRight,
            "Split right",
            "⌘D",
            "Split the focused pane to the right",
            Action::SplitRight,
        ),
        item(
            "split-down",
            Icon::ChevronDown,
            "Split down",
            "⇧⌘D",
            "Split the focused pane downward",
            Action::SplitDown,
        ),
    ]
}

pub fn layout() -> Vec<Item> {
    vec![
        item(
            "zoom",
            Icon::Square,
            "Zoom pane",
            "⇧⌘↩",
            "Zoom the focused pane (again to undo)",
            Action::ZoomPane,
        ),
        item(
            "my-layout",
            Icon::Monitor,
            "Use my layout",
            "nixa",
            "Go back to your layout: sidebar and panel at their usual width",
            Action::MyLayout,
        ),
        item(
            "layouts",
            Icon::Layers,
            "Layouts…",
            "saved",
            "Save the current layout under a name, or switch to a saved one",
            Action::Layouts,
        ),
        item(
            "sidebar",
            Icon::Sidebar,
            "Toggle sidebar",
            "⌘B",
            "Hide or show the sidebar",
            Action::ToggleSidebar,
        ),
    ]
}

pub fn workspace() -> Vec<Item> {
    vec![
        item(
            "rename-space",
            Icon::SquarePen,
            "Rename space",
            "asks",
            "Rename this space",
            Action::RenameSpace,
        ),
        item(
            "rename-tab",
            Icon::SquarePen,
            "Rename tab",
            "asks",
            "Rename this tab",
            Action::RenameTab,
        ),
        item(
            "close-tab",
            Icon::X,
            "Close tab",
            "⌘W",
            "Close this tab and end its terminals",
            Action::CloseTab,
        ),
        item(
            "close-space",
            Icon::X,
            "Close space",
            "space",
            "Close this space and end its terminals",
            Action::CloseSpace,
        ),
    ]
}

pub fn projects(list: Option<&[crate::screens::menu::projects::Project]>) -> Vec<Item> {
    let mut rows = vec![
        item(
            "back",
            Icon::ChevronDown,
            "← Back",
            "",
            "Back to Quick actions",
            Action::Back,
        ),
        item(
            "choose",
            Icon::FolderPlus,
            "Choose folder…",
            "picker",
            "Pick any folder and open it as a new space",
            Action::NewSpaceInFolder,
        ),
    ];
    match list {
        None => rows.push(item(
            "loading",
            Icon::Folder,
            "Loading…",
            "",
            "",
            Action::Back,
        )),
        Some(list) => rows.extend(list.iter().map(|p| Item {
            id: "project",
            icon: Icon::Folder,
            label: p.name.clone(),
            hint: "new space".into(),
            tip: format!("Open {} as a new space", p.path),
            action: Action::OpenProject {
                name: p.name.clone(),
                path: p.path.clone(),
            },
        })),
    }
    rows
}

pub fn for_tab(tab: Tab, unread: usize) -> Vec<Item> {
    match tab {
        Tab::Quick => quick(unread),
        Tab::Create => create(),
        Tab::Layout => layout(),
        Tab::Workspace => workspace(),
        Tab::Settings => Vec::new(),
    }
}

pub fn quick_action(id: &str) -> Option<Action> {
    let want: String = id
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase())
        .collect();
    quick(0)
        .into_iter()
        .find(|it| it.id.replace('-', "") == want)
        .map(|it| it.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_have_titles_and_places() {
        let titles: Vec<&str> = TABS.iter().map(|t| t.title()).collect();
        assert_eq!(
            titles,
            ["Quick actions", "Create", "Layout", "Workspace", "Settings"]
        );
        assert_eq!(Tab::Workspace.index(), 3);
    }

    #[test]
    fn quick_actions_match_today() {
        let ids: Vec<&str> = quick(0).iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            [
                "tasks",
                "quickcapture",
                "board",
                "files",
                "events",
                "favourites",
                "commands",
                "agents",
                "team",
                "custom-agents",
                "tasks-window",
                "projectcommands",
                "projects",
                "git",
                "gitaccounts",
                "gitclone"
            ]
        );
        assert!(!ids.contains(&"desk") && !ids.contains(&"issues"));
    }

    #[test]
    fn events_hint_counts_new_ones() {
        let unread = quick(3);
        let events = unread.iter().find(|i| i.id == "events").unwrap();
        assert_eq!(events.hint, "3 new");
        assert_eq!(events.tip, "Event history: 3 new");
        let none = quick(0);
        assert_eq!(
            none.iter().find(|i| i.id == "events").unwrap().hint,
            "history"
        );
    }

    #[test]
    fn key_actions_are_found_by_id() {
        assert_eq!(quick_action("tasks"), Some(Action::Tasks));
        assert_eq!(
            quick_action("projectcommands"),
            Some(Action::ProjectCommands)
        );
        assert_eq!(
            quick_action("Project-Commands"),
            Some(Action::ProjectCommands)
        );
        assert_eq!(quick_action("favourites"), Some(Action::Favourites));
        assert_eq!(quick_action("tasks-window"), Some(Action::TasksWindow));
        assert_eq!(quick_action("nope"), None);
    }

    #[test]
    fn every_tab_has_rows_except_settings() {
        for tab in TABS {
            assert_eq!(for_tab(tab, 0).is_empty(), tab == Tab::Settings);
        }
        assert_eq!(create().len(), 7);
        assert_eq!(layout().len(), 4);
        assert_eq!(workspace().len(), 4);
    }

    #[test]
    fn projects_view_starts_with_back_and_picker() {
        let loading = projects(None);
        assert_eq!(loading.len(), 3);
        assert_eq!(loading[0].action, Action::Back);
        assert_eq!(loading[2].label, "Loading…");
        let list = [crate::screens::menu::projects::Project {
            name: "nixa".into(),
            path: "/p/nixa".into(),
        }];
        let rows = projects(Some(&list));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[2].action,
            Action::OpenProject {
                name: "nixa".into(),
                path: "/p/nixa".into()
            }
        );
        assert_eq!(rows[2].tip, "Open /p/nixa as a new space");
    }
}
