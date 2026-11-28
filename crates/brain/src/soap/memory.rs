use crate::util::{read_json, write_json};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE: &str = "office-soap.json";
pub const KEEP: usize = 12;
pub const KEEP_LINES: usize = 30;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Story {
    pub date: String,
    pub scenes: Vec<String>,
    pub lines: Vec<String>,
}

pub fn load(path: &Path, today: &str) -> Story {
    let story: Story = read_json(path).unwrap_or_default();
    if story.date == today { story } else { Story { date: today.to_string(), ..Story::default() } }
}

pub fn add(story: &mut Story, line: String) {
    story.scenes.push(line);
    let extra = story.scenes.len().saturating_sub(KEEP);
    story.scenes.drain(..extra);
}

pub fn add_lines(story: &mut Story, lines: Vec<String>) {
    story.lines.extend(lines);
    let extra = story.lines.len().saturating_sub(KEEP_LINES);
    story.lines.drain(..extra);
}

pub fn save(path: &Path, story: &Story) -> std::io::Result<()> {
    write_json(path, story)
}
