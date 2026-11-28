use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Ctrl(char),
    Alt(char),
    F(u8),
    Raw(String),
}

impl Key {
    pub fn to_bytes(&self, app_cursor: bool) -> Vec<u8> {
        let arrow = |c: u8| -> Vec<u8> {
            if app_cursor {
                vec![0x1b, b'O', c]
            } else {
                vec![0x1b, b'[', c]
            }
        };
        match self {
            Key::Enter => vec![b'\r'],
            Key::Esc => vec![0x1b],
            Key::Tab => vec![b'\t'],
            Key::BackTab => b"\x1b[Z".to_vec(),
            Key::Backspace => vec![0x7f],
            Key::Delete => b"\x1b[3~".to_vec(),
            Key::Space => vec![b' '],
            Key::Up => arrow(b'A'),
            Key::Down => arrow(b'B'),
            Key::Right => arrow(b'C'),
            Key::Left => arrow(b'D'),
            Key::Home => arrow(b'H'),
            Key::End => arrow(b'F'),
            Key::PageUp => b"\x1b[5~".to_vec(),
            Key::PageDown => b"\x1b[6~".to_vec(),
            Key::Ctrl(c) => ctrl_byte(*c).map(|b| vec![b]).unwrap_or_default(),
            Key::Alt(c) => {
                let mut out = vec![0x1b];
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                out
            }
            Key::F(n) => function_key(*n),
            Key::Raw(s) => s.as_bytes().to_vec(),
        }
    }
}

fn ctrl_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' | '/' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn function_key(n: u8) -> Vec<u8> {
    let seq: &[u8] = match n {
        1 => b"\x1bOP",
        2 => b"\x1bOQ",
        3 => b"\x1bOR",
        4 => b"\x1bOS",
        5 => b"\x1b[15~",
        6 => b"\x1b[17~",
        7 => b"\x1b[18~",
        8 => b"\x1b[19~",
        9 => b"\x1b[20~",
        10 => b"\x1b[21~",
        11 => b"\x1b[23~",
        12 => b"\x1b[24~",
        _ => b"",
    };
    seq.to_vec()
}

impl FromStr for Key {
    type Err = String;

    fn from_str(s: &str) -> Result<Key, String> {
        let low = s.to_ascii_lowercase();
        let key = match low.as_str() {
            "enter" | "return" | "cr" => Key::Enter,
            "esc" | "escape" => Key::Esc,
            "tab" => Key::Tab,
            "shift+tab" | "backtab" => Key::BackTab,
            "backspace" | "bs" => Key::Backspace,
            "delete" | "del" => Key::Delete,
            "space" => Key::Space,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" | "pgup" => Key::PageUp,
            "pagedown" | "pgdn" => Key::PageDown,
            _ => {
                if let Some(rest) = low.strip_prefix("ctrl+").or_else(|| low.strip_prefix("c-")) {
                    let c = single_char(rest).ok_or_else(|| format!("bad key {s:?}"))?;
                    ctrl_byte(c).ok_or_else(|| format!("bad key {s:?}"))?;
                    Key::Ctrl(c)
                } else if let Some(rest) = s.get(4..).filter(|_| low.starts_with("alt+")) {
                    Key::Alt(single_char(rest).ok_or_else(|| format!("bad key {s:?}"))?)
                } else if let Some(n) = low.strip_prefix('f').and_then(|r| r.parse::<u8>().ok()) {
                    if !(1..=12).contains(&n) {
                        return Err(format!("bad key {s:?}"));
                    }
                    Key::F(n)
                } else {
                    return Err(format!(
                        "unknown key {s:?}, use enter, esc, tab, shift+tab, backspace, delete, space, up, down, left, right, home, end, pageup, pagedown, ctrl+<c>, alt+<c>, f1..f12"
                    ));
                }
            }
        };
        Ok(key)
    }
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    let c = it.next()?;
    if it.next().is_some() { None } else { Some(c) }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Enter => f.write_str("enter"),
            Key::Esc => f.write_str("esc"),
            Key::Tab => f.write_str("tab"),
            Key::BackTab => f.write_str("shift+tab"),
            Key::Backspace => f.write_str("backspace"),
            Key::Delete => f.write_str("delete"),
            Key::Space => f.write_str("space"),
            Key::Up => f.write_str("up"),
            Key::Down => f.write_str("down"),
            Key::Left => f.write_str("left"),
            Key::Right => f.write_str("right"),
            Key::Home => f.write_str("home"),
            Key::End => f.write_str("end"),
            Key::PageUp => f.write_str("pageup"),
            Key::PageDown => f.write_str("pagedown"),
            Key::Ctrl(c) => write!(f, "ctrl+{c}"),
            Key::Alt(c) => write!(f, "alt+{c}"),
            Key::F(n) => write!(f, "f{n}"),
            Key::Raw(s) => write!(f, "raw:{s:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_bytes() {
        assert_eq!("ctrl+c".parse::<Key>().unwrap().to_bytes(false), vec![3]);
        assert_eq!("Enter".parse::<Key>().unwrap(), Key::Enter);
        assert_eq!("up".parse::<Key>().unwrap().to_bytes(true), b"\x1bOA".to_vec());
        assert_eq!("up".parse::<Key>().unwrap().to_bytes(false), b"\x1b[A".to_vec());
        assert_eq!("f5".parse::<Key>().unwrap(), Key::F(5));
        assert!("f13".parse::<Key>().is_err());
        assert!("ctrl+".parse::<Key>().is_err());
        assert!("nope".parse::<Key>().is_err());
    }

    #[test]
    fn display_parses_back() {
        for k in [Key::Enter, Key::BackTab, Key::Ctrl('c'), Key::Alt('x'), Key::F(12), Key::PageDown] {
            assert_eq!(k.to_string().parse::<Key>().unwrap(), k);
        }
    }
}
