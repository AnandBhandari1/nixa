use crate::id::TermId;
use serde::{Deserialize, Serialize};

pub mod flags {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 1 << 1;
    pub const UNDERLINE: u16 = 1 << 2;
    pub const INVERSE: u16 = 1 << 3;
    pub const DIM: u16 = 1 << 4;
    pub const STRIKE: u16 = 1 << 5;
    pub const HIDDEN: u16 = 1 << 6;
    pub const WIDE: u16 = 1 << 7;
    pub const WIDE_SPACER: u16 = 1 << 8;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 9;
    pub const UNDERCURL: u16 = 1 << 10;
    pub const DOTTED_UNDERLINE: u16 = 1 << 11;
    pub const DASHED_UNDERLINE: u16 = 1 << 12;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn is_default(&self) -> bool {
        *self == Color::Default
    }
}

fn is_zero(n: &u16) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    #[serde(rename = "c")]
    pub ch: char,
    #[serde(rename = "z", default, skip_serializing_if = "Option::is_none")]
    pub zerowidth: Option<String>,
    #[serde(default, skip_serializing_if = "Color::is_default")]
    pub fg: Color,
    #[serde(default, skip_serializing_if = "Color::is_default")]
    pub bg: Color,
    #[serde(rename = "f", default, skip_serializing_if = "is_zero")]
    pub flags: u16,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell {
            ch: ' ',
            zerowidth: None,
            fg: Color::Default,
            bg: Color::Default,
            flags: 0,
        }
    }
}

impl Cell {
    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
    HollowBlock,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
    #[serde(default)]
    pub shape: CursorShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub alt_screen: bool,
    pub mouse_click: bool,
    pub mouse_drag: bool,
    pub mouse_motion: bool,
    pub mouse_sgr: bool,
    pub focus_events: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: TermId,
    pub seq: u64,
    pub rows: u16,
    pub cols: u16,
    pub cells: Vec<Cell>,
    pub cursor: Cursor,
    #[serde(default)]
    pub modes: Modes,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub history: usize,
    #[serde(default)]
    pub scroll: usize,
}

impl Snapshot {
    pub fn row(&self, row: u16) -> &[Cell] {
        let cols = self.cols as usize;
        let start = row as usize * cols;
        self.cells.get(start..start + cols).unwrap_or(&[])
    }

    pub fn row_text(&self, row: u16) -> String {
        let mut out = String::new();
        for cell in self.row(row) {
            if cell.has(flags::WIDE_SPACER) {
                continue;
            }
            out.push(cell.ch);
            if let Some(z) = &cell.zerowidth {
                out.push_str(z);
            }
        }
        out.trim_end().to_string()
    }
}
