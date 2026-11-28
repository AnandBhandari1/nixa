pub mod aura;
pub mod draw;
pub mod galaxy;
pub mod host;
pub mod input;
pub mod map;
pub mod map_draw;
pub mod meteor;
pub mod mode;
pub mod office;
pub mod orb;
pub mod palette;
pub mod prefs;
#[cfg(test)]
mod tests;
pub mod view;

pub use view::NixaScreen;

use gpui::{App, Entity, Global};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PART: &str = "Nixa screen";
pub const KEY: &str = "⌥⇧M";
pub const MASCOT: &[u8] = include_bytes!("mascot.svg");

pub struct Shared(pub Entity<NixaScreen>);

impl Global for Shared {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    Close,
    KeepOpen,
    Quiet,
    Office,
    CameraCut,
    Soap,
}

pub fn shared(cx: &App) -> Option<Entity<NixaScreen>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn commands() -> Vec<(&'static str, Command)> {
    vec![
        ("Open the Nixa screen", Command::Open),
        ("Close the Nixa screen", Command::Close),
        ("Nixa screen: keep open", Command::KeepOpen),
        ("Nixa screen: quiet galaxy", Command::Quiet),
        ("Nixa screen: office look", Command::Office),
        ("Nixa screen: office camera cut", Command::CameraCut),
        ("Nixa screen: office soap", Command::Soap),
    ]
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
