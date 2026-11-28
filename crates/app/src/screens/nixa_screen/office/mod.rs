pub mod brush;
pub mod bubble;
pub mod call;
pub mod camera;
pub mod fun;
pub mod furniture;
pub mod leave;
#[cfg(test)]
mod fun_tests;
pub mod moment;
pub mod motion;
pub mod npc;
pub mod path;
pub mod plan;
pub mod soap;
pub mod social;
pub mod sprite;
pub mod sync;
#[cfg(test)]
mod sync_tests;
pub mod talk;
#[cfg(test)]
mod tests;
pub mod view;

use super::map::Agent;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub color: usize,
    pub agents: Vec<Agent>,
    pub deep: Vec<Agent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rooms {
    pub list: Vec<Room>,
    pub loose_deep: Vec<Agent>,
    pub active: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    Company,
    Space(String),
}

pub fn shown_tab(picked: Option<&Tab>, rooms: &Rooms) -> Tab {
    if let Some(tab) = picked {
        match tab {
            Tab::Company => return Tab::Company,
            Tab::Space(id) if rooms.list.iter().any(|r| &r.id == id) => return tab.clone(),
            Tab::Space(_) => {}
        }
    }
    match &rooms.active {
        Some(id) if rooms.list.iter().any(|r| &r.id == id) => Tab::Space(id.clone()),
        _ => Tab::Company,
    }
}

pub fn rooms_of<'a>(rooms: &'a Rooms, tab: &Tab) -> Vec<&'a Room> {
    match tab {
        Tab::Company => rooms.list.iter().collect(),
        Tab::Space(id) => rooms.list.iter().filter(|r| &r.id == id).collect(),
    }
}
