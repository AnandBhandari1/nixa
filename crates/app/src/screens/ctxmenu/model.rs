use nixa_protocol::TermId;
use nixa_ui::Icon;

pub const RIGHT_CLICK_LABEL: &str = "nixa.right-click";
pub const TO_PROGRAM: &str = "program";
pub const TO_MENU: &str = "menu";

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Link {
        path: String,
    },
    Pane {
        tab: String,
        terminal: TermId,
    },
    Tab {
        space: String,
        id: Option<String>,
        terminal: Option<TermId>,
    },
    Settled {
        space: String,
    },
    Space {
        id: String,
    },
    Default,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TermFact {
    pub id: TermId,
    pub name: String,
    pub to_program: bool,
    pub parent: Option<TermId>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabFact {
    pub id: String,
    pub name: String,
    pub named: bool,
    pub program: Option<String>,
    pub title: Option<String>,
    pub settled: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpaceFact {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub place: String,
    pub tabs: Vec<TabFact>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    pub spaces: Vec<SpaceFact>,
    pub terms: Vec<TermFact>,
    pub themes: Vec<String>,
    pub theme: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkout {
    Main,
    Linked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worktree {
    New,
    Open,
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    LinkOpen {
        path: String,
    },
    LinkReveal {
        path: String,
    },
    Split {
        tab: String,
        terminal: TermId,
        side: Side,
    },
    Zoom {
        tab: String,
        terminal: TermId,
    },
    RightClicks {
        terminal: TermId,
        to_program: bool,
    },
    SendBack {
        terminal: TermId,
    },
    ClosePane {
        tab: String,
        terminal: TermId,
    },
    RenameTab {
        id: String,
    },
    NewTab {
        space: String,
    },
    SaveAgent {
        terminal: TermId,
        title: String,
        space: String,
        space_name: String,
        folder: String,
    },
    SettleTab {
        id: String,
    },
    UnsettleTab {
        id: String,
    },
    CloseTab {
        id: String,
    },
    RenameSpace {
        id: String,
    },
    Worktree {
        space: String,
        step: Worktree,
    },
    CloseSpace {
        id: String,
    },
    NewSpace,
    Keybinds,
    Theme {
        name: String,
    },
    QuickActions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub label: String,
    pub action: Action,
    pub icon: Icon,
    pub separated: bool,
    pub checked: bool,
}

pub const DEFAULT_THEMES: [&str; 4] = ["graphite", "ink", "moss", "atom-one-dark"];

pub fn base_name(path: &str) -> String {
    path.split('/')
        .rfind(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub fn theme_label(name: &str) -> String {
    format!("Theme: {}", name.replace('-', " "))
}

pub fn settled_label(tab: &TabFact) -> String {
    if tab.named {
        return tab.name.clone();
    }
    tab.program
        .clone()
        .filter(|p| !p.is_empty())
        .or_else(|| tab.title.clone().filter(|t| !t.is_empty()))
        .unwrap_or_else(|| tab.name.clone())
}

pub fn space_place<'a>(folder: &str, cwds: impl IntoIterator<Item = &'a str>) -> String {
    cwds.into_iter()
        .find(|cwd| !cwd.is_empty())
        .unwrap_or(folder)
        .to_string()
}

pub fn title(target: &Target) -> String {
    match target {
        Target::Link { path } => base_name(path),
        _ => "Actions".to_string(),
    }
}

fn space<'a>(facts: &'a Facts, id: &str) -> Option<&'a SpaceFact> {
    facts.spaces.iter().find(|s| s.id == id)
}

fn term<'a>(facts: &'a Facts, id: &TermId) -> Option<&'a TermFact> {
    facts.terms.iter().find(|t| &t.id == id)
}

fn link_items(path: &str) -> Vec<(String, Action)> {
    if path.is_empty() {
        return Vec::new();
    }
    let open = (
        "Open in here".to_string(),
        Action::LinkOpen {
            path: path.to_string(),
        },
    );
    let reveal = (
        "Show in Finder".to_string(),
        Action::LinkReveal {
            path: path.to_string(),
        },
    );
    vec![open, reveal]
}

fn pane_items(facts: &Facts, tab: &str, terminal: &TermId) -> Vec<(String, Action)> {
    let split = |side| Action::Split {
        tab: tab.to_string(),
        terminal: terminal.clone(),
        side,
    };
    let mut list = vec![
        ("Split right".to_string(), split(Side::Right)),
        ("Split down".to_string(), split(Side::Down)),
        (
            "Zoom".to_string(),
            Action::Zoom {
                tab: tab.to_string(),
                terminal: terminal.clone(),
            },
        ),
    ];
    if let Some(info) = term(facts, terminal) {
        let label = if info.to_program {
            "Use the built-in right-click menu"
        } else {
            "Send right-clicks to pane"
        };
        list.push((
            label.to_string(),
            Action::RightClicks {
                terminal: info.id.clone(),
                to_program: !info.to_program,
            },
        ));
        let parent = info.parent.as_ref().and_then(|p| term(facts, p));
        if let Some(parent) = parent {
            let name = if parent.name.is_empty() {
                parent.id.to_string()
            } else {
                parent.name.clone()
            };
            list.push((
                format!("Send back to {name}"),
                Action::SendBack {
                    terminal: info.id.clone(),
                },
            ));
        }
    }
    list.push((
        "Close pane".to_string(),
        Action::ClosePane {
            tab: tab.to_string(),
            terminal: terminal.clone(),
        },
    ));
    list
}

fn tab_items(
    facts: &Facts,
    space_id: &str,
    id: &Option<String>,
    terminal: &Option<TermId>,
) -> Vec<(String, Action)> {
    let owner = space(facts, space_id);
    let tab = id
        .as_ref()
        .and_then(|id| owner.and_then(|s| s.tabs.iter().find(|t| &t.id == id)));
    let mut list = Vec::new();
    if let Some(id) = id {
        list.push(("Rename".to_string(), Action::RenameTab { id: id.clone() }));
    }
    list.push((
        "New tab".to_string(),
        Action::NewTab {
            space: space_id.to_string(),
        },
    ));
    if let (Some(terminal), Some(owner)) = (terminal, owner) {
        list.push((
            "Save as custom agent".to_string(),
            Action::SaveAgent {
                terminal: terminal.clone(),
                title: tab.map(|t| t.name.clone()).unwrap_or_default(),
                space: owner.id.clone(),
                space_name: owner.name.clone(),
                folder: owner.folder.clone(),
            },
        ));
    }
    if let Some(tab) = tab {
        list.push(if tab.settled {
            (
                "Bring back".to_string(),
                Action::UnsettleTab { id: tab.id.clone() },
            )
        } else {
            (
                "Put tab away".to_string(),
                Action::SettleTab { id: tab.id.clone() },
            )
        });
    }
    if let Some(id) = id {
        list.push(("Close".to_string(), Action::CloseTab { id: id.clone() }));
    }
    list
}

fn settled_items(facts: &Facts, space_id: &str) -> Vec<(String, Action)> {
    space(facts, space_id)
        .map(|s| {
            s.tabs
                .iter()
                .filter(|t| t.settled)
                .map(|t| (settled_label(t), Action::UnsettleTab { id: t.id.clone() }))
                .collect()
        })
        .unwrap_or_default()
}

fn space_items(id: &str, git: Option<Checkout>) -> Vec<(String, Action)> {
    let step = |step| Action::Worktree {
        space: id.to_string(),
        step,
    };
    let mut list = vec![(
        "Rename".to_string(),
        Action::RenameSpace { id: id.to_string() },
    )];
    if git.is_some() {
        list.push(("New worktree".to_string(), step(Worktree::New)));
        list.push(("Open worktree".to_string(), step(Worktree::Open)));
    }
    if git == Some(Checkout::Linked) {
        list.push(("Remove worktree".to_string(), step(Worktree::Remove)));
    }
    list.push((
        "Close".to_string(),
        Action::CloseSpace { id: id.to_string() },
    ));
    list
}

fn default_items(facts: &Facts) -> Vec<(String, Action)> {
    let mut list = vec![
        ("New space".to_string(), Action::NewSpace),
        ("Keybinds".to_string(), Action::Keybinds),
    ];
    let themes: Vec<String> = if facts.themes.is_empty() {
        DEFAULT_THEMES.iter().map(|t| t.to_string()).collect()
    } else {
        facts.themes.clone()
    };
    for name in themes {
        list.push((theme_label(&name), Action::Theme { name }));
    }
    list
}

pub fn items(target: &Target, facts: &Facts, git: Option<Checkout>) -> Vec<(String, Action)> {
    match target {
        Target::Link { path } => link_items(path),
        Target::Pane { tab, terminal } => pane_items(facts, tab, terminal),
        Target::Tab {
            space,
            id,
            terminal,
        } => tab_items(facts, space, id, terminal),
        Target::Settled { space } => settled_items(facts, space),
        Target::Space { id } => space_items(id, git),
        Target::Default => default_items(facts),
    }
}

pub fn icon_for(action: &Action) -> Icon {
    match action {
        Action::LinkOpen { .. } => Icon::Notebook,
        Action::LinkReveal { .. } => Icon::Folder,
        Action::ClosePane { .. } | Action::CloseTab { .. } | Action::CloseSpace { .. } => Icon::X,
        Action::RenameTab { .. } | Action::RenameSpace { .. } => Icon::SquarePen,
        Action::NewTab { .. } | Action::NewSpace => Icon::Plus,
        Action::Worktree { .. } => Icon::GitBranch,
        Action::Keybinds => Icon::Terminal,
        Action::Theme { .. } => Icon::Settings,
        Action::Split { .. } => Icon::PanelRight,
        Action::Zoom { .. } => Icon::Square,
        Action::SettleTab { .. } => Icon::Archive,
        Action::UnsettleTab { .. } => Icon::Pin,
        Action::QuickActions => Icon::Zap,
        _ => Icon::Terminal,
    }
}

fn is_close(action: &Action) -> bool {
    matches!(
        action,
        Action::ClosePane { .. } | Action::CloseTab { .. } | Action::CloseSpace { .. }
    )
}

fn is_theme(action: &Action) -> bool {
    matches!(action, Action::Theme { .. })
}

pub fn entries(target: &Target, facts: &Facts, git: Option<Checkout>) -> Vec<Entry> {
    let mut list = items(target, facts, git);
    if !matches!(target, Target::Link { .. }) {
        list.push(("Quick actions".to_string(), Action::QuickActions));
    }
    let mut out: Vec<Entry> = Vec::with_capacity(list.len());
    for (index, (label, action)) in list.into_iter().enumerate() {
        let after_theme = out.last().is_some_and(|e| is_theme(&e.action));
        let separated = index > 0
            && (matches!(action, Action::QuickActions | Action::Keybinds)
                || is_close(&action)
                || (is_theme(&action) && !after_theme));
        let checked = match &action {
            Action::Theme { name } => facts.theme.as_deref() == Some(name.as_str()),
            _ => false,
        };
        out.push(Entry {
            label,
            icon: icon_for(&action),
            action,
            separated,
            checked,
        });
    }
    out
}
