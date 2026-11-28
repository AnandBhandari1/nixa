use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::{CStr, c_char};

const COMBINED_SESSION: i32 = 0;
const F9: u16 = 0x65;
const RIGHT_COMMAND: u16 = 0x36;
const LAST_KEY: u16 = 0x7E;
const COMMAND_MOD: u64 = 0x0010_0000;
const SHIFT_MOD: u64 = 0x0002_0000;
const OPTION_MOD: u64 = 0x0008_0000;
const OTHER_MODS: u64 = COMMAND_MOD | 0x0004_0000 | SHIFT_MOD;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGPreflightListenEventAccess() -> bool;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

pub fn key_access() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct KeyTap {
    held: bool,
    mixed: bool,
    front: bool,
}

pub struct KeyNow {
    pub down: bool,
    pub other_mods: bool,
}

impl KeyTap {
    pub fn step(&mut self, k: &KeyNow, front: impl Fn() -> bool) -> Option<bool> {
        if k.down && !self.held {
            self.held = true;
            self.mixed = k.other_mods;
            self.front = front();
        } else if k.down && self.held {
            self.mixed |= k.other_mods;
        } else if !k.down && self.held {
            self.held = false;
            if !self.mixed {
                return Some(self.front);
            }
        }
        None
    }
}

pub struct Keys {
    pub f9: KeyNow,
    pub command: KeyNow,
}

fn key_down(key: u16) -> bool {
    unsafe { CGEventSourceKeyState(COMBINED_SESSION, key) }
}

pub fn command_mixed(flags: u64, other_key: bool) -> bool {
    flags & ((OTHER_MODS & !COMMAND_MOD) | OPTION_MOD) != 0 || other_key
}

pub(crate) fn read_keys() -> Keys {
    let flags = unsafe { CGEventSourceFlagsState(COMBINED_SESSION) };
    let f9 = KeyNow { down: key_down(F9), other_mods: flags & OTHER_MODS != 0 };
    let down = key_down(RIGHT_COMMAND);
    let other_key = down && (0..=LAST_KEY).any(|k| k != RIGHT_COMMAND && key_down(k));
    Keys { f9, command: KeyNow { down, other_mods: down && command_mixed(flags, other_key) } }
}

unsafe fn ns_text(value: *mut AnyObject) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let text: *const c_char = unsafe { msg_send![value, UTF8String] };
    (!text.is_null()).then(|| unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned())
}

pub(crate) struct FrontApp {
    pub bundle: Option<String>,
    pub name: Option<String>,
    pub pid: i32,
}

pub(crate) fn front_app() -> FrontApp {
    autoreleasepool(|_| unsafe {
        let none = FrontApp { bundle: None, name: None, pid: 0 };
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return none;
        }
        let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
        if app.is_null() {
            return none;
        }
        let pid: i32 = msg_send![app, processIdentifier];
        let id: *mut AnyObject = msg_send![app, bundleIdentifier];
        let name: *mut AnyObject = msg_send![app, localizedName];
        FrontApp { bundle: ns_text(id), name: ns_text(name), pid }
    })
}

pub(crate) fn is_ours(front: &FrontApp, own_pid: i32, wanted: &[String]) -> bool {
    let hit = |v: &Option<String>| v.as_ref().is_some_and(|v| wanted.iter().any(|w| w == v));
    front.pid == own_pid || hit(&front.bundle) || hit(&front.name)
}

pub(crate) fn we_are_front(bundles: &[String]) -> bool {
    is_ours(&front_app(), std::process::id() as i32, bundles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(down: bool, other: bool) -> KeyNow {
        KeyNow { down, other_mods: other }
    }

    #[test]
    fn clean_f9_tap_reports_front() {
        let mut t = KeyTap::default();
        assert_eq!(t.step(&k(true, false), || true), None);
        assert_eq!(t.step(&k(true, false), || false), None);
        assert_eq!(t.step(&k(false, false), || false), Some(true));
        assert_eq!(t.step(&k(false, false), || false), None);
    }

    #[test]
    fn a_long_f9_press_still_counts_once() {
        let mut t = KeyTap::default();
        for _ in 0..100 {
            assert_eq!(t.step(&k(true, false), || false), None);
        }
        assert_eq!(t.step(&k(false, false), || true), Some(false));
    }

    #[test]
    fn f9_with_command_is_ignored() {
        let mut t = KeyTap::default();
        t.step(&k(true, true), || false);
        assert_eq!(t.step(&k(false, false), || false), None);
        t.step(&k(true, false), || false);
        t.step(&k(true, true), || false);
        assert_eq!(t.step(&k(false, false), || false), None);
    }

    #[test]
    fn right_command_alone_is_a_tap_and_a_shortcut_is_not() {
        assert!(!command_mixed(COMMAND_MOD, false));
        assert!(command_mixed(COMMAND_MOD, true));
        assert!(command_mixed(COMMAND_MOD | SHIFT_MOD, false));
        assert!(command_mixed(COMMAND_MOD | OPTION_MOD, false));
        let mut t = KeyTap::default();
        t.step(&k(true, command_mixed(COMMAND_MOD, false)), || false);
        assert_eq!(t.step(&k(false, false), || false), Some(false));
        t.step(&k(true, false), || false);
        t.step(&k(true, command_mixed(COMMAND_MOD, true)), || false);
        assert_eq!(t.step(&k(false, false), || false), None);
    }

    #[test]
    fn front_app_answers() {
        assert!(front_app().pid >= 0);
    }

    #[test]
    fn front_app_matches_bundle_name_or_pid() {
        let wanted = vec!["dev.nixa.app".to_string(), "nixa-app".to_string()];
        let app = |bundle: Option<&str>, name: Option<&str>, pid: i32| FrontApp { bundle: bundle.map(String::from), name: name.map(String::from), pid };
        assert!(is_ours(&app(Some("dev.nixa.app"), Some("Nixa"), 5), 9, &wanted));
        assert!(is_ours(&app(None, Some("nixa-app"), 5), 9, &wanted));
        assert!(is_ours(&app(None, None, 9), 9, &wanted));
        assert!(!is_ours(&app(Some("com.apple.Terminal"), Some("Terminal"), 5), 9, &wanted));
    }
}
