use gpui::{App, Global};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
}

pub type Notify = fn(&str, &str, Level, &mut App);
pub type OpenAi = fn(&str, &Path, &str, &mut App);
pub type OpenRebase = fn(&Path, &str, Option<&str>, &mut App);

#[derive(Clone, Copy)]
pub struct Links {
    pub notify: Notify,
    pub changed: fn(&Path, &mut App),
    pub open_ai: Option<OpenAi>,
    pub open_rebase: Option<OpenRebase>,
    pub open_identity: Option<fn(&Path, &mut App)>,
    pub open_folder: Option<fn(&str, &mut App)>,
}

impl Global for Links {}

fn quiet(_: &str, _: &str, _: Level, _: &mut App) {}

fn nothing(_: &Path, _: &mut App) {}

impl Default for Links {
    fn default() -> Self {
        Links {
            notify: quiet,
            changed: nothing,
            open_ai: None,
            open_rebase: None,
            open_identity: None,
            open_folder: None,
        }
    }
}

pub fn links(cx: &App) -> Links {
    cx.try_global::<Links>().copied().unwrap_or_default()
}

pub fn set_links(links: Links, cx: &mut App) {
    cx.set_global(links);
}

pub fn notify(title: &str, body: &str, level: Level, cx: &mut App) {
    (links(cx).notify)(title, body, level, cx);
}

pub fn changed(repo: &Path, cx: &mut App) {
    (links(cx).changed)(repo, cx);
    super::refresh_all(repo, cx);
}
