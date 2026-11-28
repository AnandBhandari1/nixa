pub mod colors;
pub mod input;
pub mod link;
pub mod metrics;
pub mod mouse;
mod paint;
pub mod pasted;
pub mod selection;
pub mod target;
mod view;

pub use colors::Palette;
pub use view::{DEFAULT_FONT, DEFAULT_FONT_SIZE, TermView, TermViewEvent, terminal_font};

pub const CRATE: &str = "nixa-termview";
