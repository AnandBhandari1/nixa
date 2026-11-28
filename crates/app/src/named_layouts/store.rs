use crate::panel;
use crate::workspace::{PanelKind, Workspace};
use gpui::Context;
use nixa_protocol::paths;
use nixa_ui::metrics::{
    PANEL_MAX, PANEL_MIN, PANEL_WIDTH, SIDEBAR_MAX, SIDEBAR_MIN, SIDEBAR_NARROW, SIDEBAR_WIDTH,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FOLDER: &str = "layouts";
pub const NAME_MAX: usize = 40;

fn sidebar_default() -> f32 {
    SIDEBAR_WIDTH
}

fn panel_default() -> f32 {
    PANEL_WIDTH
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default = "sidebar_default")]
    pub sidebar_width: f32,
    #[serde(default = "sidebar_default")]
    pub sidebar_open_width: f32,
    #[serde(default = "panel_default")]
    pub panel_width: f32,
    #[serde(default)]
    pub panel: Option<String>,
}

pub fn folder() -> PathBuf {
    paths::data_dir().join(FOLDER)
}

pub fn clean_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let ok = !name.is_empty()
        && name.chars().count() <= NAME_MAX
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ');
    ok.then(|| name.to_string())
}

pub fn file_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

pub fn list(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .filter(|n| clean_name(n).as_deref() == Some(n.as_str()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn read(dir: &Path, name: &str) -> anyhow::Result<Saved> {
    let text = std::fs::read_to_string(file_of(dir, name))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn write(dir: &Path, name: &str, saved: &Saved) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(saved)?;
    std::fs::write(file_of(dir, name), text + "\n")?;
    Ok(())
}

pub fn delete(dir: &Path, name: &str) -> anyhow::Result<()> {
    std::fs::remove_file(file_of(dir, name))?;
    Ok(())
}

pub fn capture(ws: &Workspace) -> Saved {
    Saved {
        sidebar_width: ws.sidebar_w.round(),
        sidebar_open_width: ws.sidebar_open_w.round(),
        panel_width: ws.panel_w.round(),
        panel: ws.panel.map(|k| panel::word(k).to_string()),
    }
}

fn clamp_or(value: f32, default: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

pub fn fixed(saved: &Saved) -> (f32, f32, f32, Option<PanelKind>) {
    let sidebar = clamp_or(saved.sidebar_width, SIDEBAR_WIDTH, SIDEBAR_MIN, SIDEBAR_MAX);
    let open = clamp_or(
        saved.sidebar_open_width,
        sidebar.max(SIDEBAR_NARROW),
        SIDEBAR_NARROW,
        SIDEBAR_MAX,
    );
    let panel_w = clamp_or(saved.panel_width, PANEL_WIDTH, PANEL_MIN, PANEL_MAX);
    let kind = saved.panel.as_deref().and_then(panel::from_word);
    (sidebar, open, panel_w, kind)
}

pub fn apply(ws: &mut Workspace, saved: &Saved, cx: &mut Context<Workspace>) {
    let (sidebar, open, panel_w, kind) = fixed(saved);
    ws.sidebar_w = sidebar;
    ws.sidebar_open_w = open;
    ws.panel_w = panel_w;
    ws.set_panel(kind, cx);
}

pub fn apply_default(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    ws.sidebar_w = SIDEBAR_WIDTH;
    ws.sidebar_open_w = SIDEBAR_WIDTH;
    ws.panel_w = PANEL_WIDTH;
    cx.notify();
}
