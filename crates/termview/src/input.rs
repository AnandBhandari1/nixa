use gpui::{Keystroke, Modifiers};
use nixa_protocol::Key;

fn modifier_param(m: &Modifiers) -> u8 {
    1 + m.shift as u8 + (m.alt as u8) * 2 + (m.control as u8) * 4
}

fn csi_modified(m: &Modifiers, final_byte: char) -> Vec<u8> {
    format!("\x1b[1;{}{}", modifier_param(m), final_byte).into_bytes()
}

fn tilde_modified(m: &Modifiers, n: u8) -> Vec<u8> {
    format!("\x1b[{};{}~", n, modifier_param(m)).into_bytes()
}

fn has_mods(m: &Modifiers) -> bool {
    m.shift || m.alt || m.control
}

fn named(key: &str, m: &Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let cursor_final = match key {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(f) = cursor_final {
        if has_mods(m) {
            return Some(csi_modified(m, f));
        }
        let k = match f {
            'A' => Key::Up,
            'B' => Key::Down,
            'C' => Key::Right,
            'D' => Key::Left,
            'H' => Key::Home,
            _ => Key::End,
        };
        return Some(k.to_bytes(app_cursor));
    }
    let tilde = match key {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        _ => None,
    };
    if let Some(n) = tilde {
        if has_mods(m) {
            return Some(tilde_modified(m, n));
        }
        return Some(format!("\x1b[{n}~").into_bytes());
    }
    let alt_prefix = |bytes: Vec<u8>| -> Vec<u8> {
        if m.alt {
            let mut out = vec![0x1b];
            out.extend(bytes);
            out
        } else {
            bytes
        }
    };
    match key {
        "enter" => {
            if m.shift || m.alt {
                Some(b"\x1b\r".to_vec())
            } else {
                Some(vec![b'\r'])
            }
        }
        "escape" => Some(vec![0x1b]),
        "tab" => {
            if m.shift {
                Some(Key::BackTab.to_bytes(false))
            } else {
                Some(vec![b'\t'])
            }
        }
        "backspace" => {
            if m.control {
                Some(alt_prefix(vec![0x08]))
            } else {
                Some(alt_prefix(vec![0x7f]))
            }
        }
        "space" if m.control => Some(vec![0]),
        _ => function_key(key, m),
    }
}

fn function_key(key: &str, m: &Modifiers) -> Option<Vec<u8>> {
    let n: u8 = key.strip_prefix('f')?.parse().ok()?;
    if !(1..=12).contains(&n) {
        return None;
    }
    if !has_mods(m) {
        return Some(Key::F(n).to_bytes(false));
    }
    let code = match n {
        1 => return Some(csi_modified(m, 'P')),
        2 => return Some(csi_modified(m, 'Q')),
        3 => return Some(csi_modified(m, 'R')),
        4 => return Some(csi_modified(m, 'S')),
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        _ => 24,
    };
    Some(tilde_modified(m, code))
}

pub fn key_bytes(ks: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if m.platform {
        return None;
    }
    if let Some(bytes) = named(ks.key.as_str(), m, app_cursor) {
        return Some(bytes);
    }
    if m.control {
        let mut chars = ks.key.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            let bytes = Key::Ctrl(c).to_bytes(false);
            if !bytes.is_empty() {
                if m.alt {
                    let mut out = vec![0x1b];
                    out.extend(bytes);
                    return Some(out);
                }
                return Some(bytes);
            }
        }
    }
    let text = ks
        .key_char
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let mut chars = ks.key.chars();
            match (chars.next(), chars.next()) {
                (Some(_), None) => Some(ks.key.as_str()),
                _ => None,
            }
        })?;
    if text.chars().any(|c| c.is_control()) && !m.control {
        return None;
    }
    Some(text.as_bytes().to_vec())
}

pub fn line_key(ks: &Keystroke) -> Option<&'static str> {
    let m = &ks.modifiers;
    if !m.platform || m.shift || m.alt || m.control {
        return None;
    }
    match ks.key.as_str() {
        "left" => Some("\x01"),
        "right" => Some("\x05"),
        "backspace" => Some("\x15"),
        _ => None,
    }
}

pub fn is_image_paste(ks: &Keystroke) -> bool {
    let m = &ks.modifiers;
    m.control && !m.platform && !m.shift && !m.alt && ks.key == "v"
}

pub fn paste_bytes(text: &str, bracketed: bool) -> String {
    if bracketed {
        let clean: String = text.chars().filter(|&c| c != '\x1b').collect();
        format!("\x1b[200~{clean}\x1b[201~")
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn plain_and_control_keys() {
        assert_eq!(key_bytes(&ks("enter"), false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(&ks("ctrl-c"), false), Some(vec![3]));
        assert_eq!(key_bytes(&ks("up"), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            key_bytes(&ks("shift-up"), false),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(key_bytes(&ks("shift-tab"), false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(key_bytes(&ks("f5"), false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(key_bytes(&ks("cmd-c"), false), None);
        assert_eq!(key_bytes(&ks("a"), false), Some(b"a".to_vec()));
    }

    #[test]
    fn line_keys_map_to_shell_keys() {
        assert_eq!(line_key(&ks("cmd-left")), Some("\x01"));
        assert_eq!(line_key(&ks("cmd-right")), Some("\x05"));
        assert_eq!(line_key(&ks("cmd-backspace")), Some("\x15"));
        assert_eq!(line_key(&ks("cmd-shift-left")), None);
        assert_eq!(line_key(&ks("left")), None);
        assert!(is_image_paste(&ks("ctrl-v")));
        assert!(!is_image_paste(&ks("cmd-v")));
        assert!(!is_image_paste(&ks("ctrl-shift-v")));
    }

    #[test]
    fn paste_wraps_when_bracketed() {
        assert_eq!(paste_bytes("a\nb", false), "a\rb");
        assert_eq!(paste_bytes("a\x1bb", true), "\x1b[200~ab\x1b[201~");
    }
}
