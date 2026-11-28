use super::prefs;
use std::path::Path;
use super::quick::{self, QUICK_AGENTS};
use crate::workspace::Workspace;
use gpui::Context;

pub(super) fn command(index: usize, model: Option<&str>) -> Option<(String, Vec<String>)> {
    let button = quick::button_in(&crate::programs::default_path(), index)?;
    quick::start(button, model).or_else(|| quick::start(button, None))
}

pub(super) fn saved_command(file: &Path, index: usize) -> Option<(String, Vec<String>)> {
    let agent = QUICK_AGENTS.get(index)?;
    let model = prefs::model_for(file, agent.id);
    command(index, model.as_deref())
}

fn start_tab(ws: &mut Workspace, run: Option<(String, Vec<String>)>, cx: &mut Context<Workspace>) {
    let Some((name, run)) = run else {
        return;
    };
    ws.new_tab(run, Some(name), cx);
    cx.notify();
}

pub(super) fn launch_saved(ws: &mut Workspace, index: usize, cx: &mut Context<Workspace>) {
    start_tab(ws, saved_command(&prefs::file(), index), cx);
}
