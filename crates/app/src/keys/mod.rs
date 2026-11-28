pub mod file;
pub mod help;
pub mod names;
pub mod table;
#[cfg(test)]
mod tests;

use crate::actions::ClosePane;
use gpui::{App, DummyKeyboardMapper, Global, KeyBinding};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use table::Entry;

const WATCH: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyTable {
    pub entries: Vec<Entry>,
    pub problems: Vec<String>,
    pub stamp: Option<SystemTime>,
}

impl Global for KeyTable {}

pub fn binding(entry: &Entry) -> Option<KeyBinding> {
    let action = names::action(&entry.name)?;
    KeyBinding::load(&entry.key, action, None, false, None, &DummyKeyboardMapper).ok()
}

pub fn key_bindings(entries: &[Entry]) -> Vec<KeyBinding> {
    let mut list = vec![KeyBinding::new("cmd-w", ClosePane, None)];
    list.extend(entries.iter().filter_map(binding));
    list
}

pub fn build(path: &Path) -> KeyTable {
    let (user, problems) = file::read(path);
    KeyTable {
        entries: table::merged(table::defaults(), &user),
        problems,
        stamp: file::stamp(path),
    }
}

pub fn load(cx: &mut App) {
    let path = file::path();
    let next = if cfg!(test) {
        KeyTable {
            entries: table::defaults(),
            ..KeyTable::default()
        }
    } else {
        build(&path)
    };
    cx.bind_keys(key_bindings(&next.entries));
    cx.set_global(next);
    watch(path, cx);
}

pub fn rebuilt(old: &[KeyBinding], app: Vec<KeyBinding>) -> Vec<KeyBinding> {
    let mut out = Vec::with_capacity(old.len() + app.len());
    let mut app = Some(app);
    for b in old {
        if b.predicate().is_some() {
            out.push(b.clone());
        } else if let Some(list) = app.take() {
            out.extend(list);
        }
    }
    if let Some(list) = app {
        out.extend(list);
    }
    out
}

pub fn reload(cx: &mut App) {
    let next = build(&file::path());
    let old: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    cx.clear_key_bindings();
    cx.bind_keys(rebuilt(&old, key_bindings(&next.entries)));
    cx.set_global(next);
}

fn changed(path: &Path, cx: &App) -> bool {
    cx.try_global::<KeyTable>().map(|t| t.stamp) != Some(file::stamp(path))
}

fn watch(path: PathBuf, cx: &mut App) {
    if cfg!(test) {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(WATCH).await;
            let Ok(now) = cx.update(|cx| changed(&path, cx)) else {
                break;
            };
            if now && cx.update(reload).is_err() {
                break;
            }
        }
    })
    .detach();
}
