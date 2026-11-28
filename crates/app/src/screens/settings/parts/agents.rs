use crate::screens::custom_agents::{self, CustomAgentsView, Mode};
use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Global};

pub const PART: &str = "Agents and team";

struct AgentsPart(Entity<CustomAgentsView>);

impl Global for AgentsPart {}

pub fn shared(cx: &mut App) -> Entity<CustomAgentsView> {
    if let Some(part) = cx.try_global::<AgentsPart>() {
        return part.0.clone();
    }
    let view = custom_agents::new(Mode::Settings, custom_agents::brain(), cx);
    cx.set_global(AgentsPart(view.clone()));
    view
}

pub fn render(cx: &mut App) -> AnyElement {
    shared(cx).into_any_element()
}
