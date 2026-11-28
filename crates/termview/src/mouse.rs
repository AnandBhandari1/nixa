use gpui::{Modifiers, MouseButton};
use nixa_protocol::Modes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAct {
    Press(MouseButton),
    Release(MouseButton),
    Drag(MouseButton),
    Move,
    WheelUp,
    WheelDown,
}

pub fn wants_mouse(modes: &Modes) -> bool {
    modes.mouse_click || modes.mouse_drag || modes.mouse_motion
}

fn button_code(b: MouseButton) -> Option<u8> {
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

pub fn report(
    act: MouseAct,
    row: u16,
    col: u16,
    mods: &Modifiers,
    modes: &Modes,
) -> Option<Vec<u8>> {
    let base = match act {
        MouseAct::Press(b) => button_code(b)?,
        MouseAct::Release(b) => {
            if modes.mouse_sgr {
                button_code(b)?
            } else {
                3
            }
        }
        MouseAct::Drag(b) => {
            if !(modes.mouse_drag || modes.mouse_motion) {
                return None;
            }
            button_code(b)? + 32
        }
        MouseAct::Move => {
            if !modes.mouse_motion {
                return None;
            }
            35
        }
        MouseAct::WheelUp => 64,
        MouseAct::WheelDown => 65,
    };
    let code = base + (mods.shift as u8) * 4 + (mods.alt as u8) * 8 + (mods.control as u8) * 16;
    let x = col as u32 + 1;
    let y = row as u32 + 1;
    if modes.mouse_sgr {
        let end = if matches!(act, MouseAct::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    if x > 95 || y > 95 {
        return None;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        32 + x as u8,
        32 + y as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_and_normal_reports() {
        let mods = Modifiers::default();
        let mut modes = Modes {
            mouse_click: true,
            mouse_sgr: true,
            ..Modes::default()
        };
        assert_eq!(
            report(MouseAct::Press(MouseButton::Left), 2, 4, &mods, &modes),
            Some(b"\x1b[<0;5;3M".to_vec())
        );
        assert_eq!(
            report(MouseAct::Release(MouseButton::Left), 2, 4, &mods, &modes),
            Some(b"\x1b[<0;5;3m".to_vec())
        );
        assert_eq!(report(MouseAct::Move, 0, 0, &mods, &modes), None);
        modes.mouse_sgr = false;
        assert_eq!(
            report(MouseAct::WheelUp, 0, 0, &mods, &modes),
            Some(vec![0x1b, b'[', b'M', 96, 33, 33])
        );
    }
}
