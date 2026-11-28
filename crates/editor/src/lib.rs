pub mod area;
pub mod buffer;
pub mod doc;
pub mod files;
pub mod find;
pub mod image;
pub mod keys;
pub mod lang;
pub mod markdown;
pub mod md_view;
mod mermaid;
pub mod syntax;
pub mod view;

#[cfg(test)]
mod tests;

pub use keys::bind_keys;
pub use view::{Editor, EditorCommand, EditorEvent, Mode, Options, State, commands, run_command};

pub const CRATE: &str = "nixa-editor";
