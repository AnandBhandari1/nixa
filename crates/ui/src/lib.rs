pub mod agent_icons;
pub mod button;
pub mod chip;
pub mod icon_button;
pub mod icons;
pub mod list;
pub mod menu;
pub mod metrics;
pub mod modal;
pub mod popover;
pub mod scroll;
pub mod section;
pub mod split;
pub mod tab;
pub mod text;
pub mod text_input;
pub mod theme;
pub mod toast;
pub mod toggle;

pub use icons::{Assets, Icon, icon};
pub use theme::{Theme, theme};

pub const CRATE: &str = "nixa-ui";

pub fn child_id(parent: &gpui::ElementId, part: impl Into<gpui::SharedString>) -> gpui::ElementId {
    gpui::ElementId::NamedChild(Box::new(parent.clone()), part.into())
}

pub fn init(cx: &mut gpui::App) {
    theme::install(cx);
    text_input::bind_keys(cx);
}
