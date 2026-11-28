use crate::cf::{self, Owned, Ref};
use crate::rows::{Bounds, MAX_DEPTH, MAX_ELEMENTS, Win, bounds_close, element_row, label_of};
use serde_json::Value;
use std::ffi::c_void;

const POINT: u32 = 1;
const SIZE: u32 = 2;
const WAIT_SECS: f32 = 2.0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementCopyAttributeValue(element: Ref, name: Ref, value: *mut Ref) -> i32;
    fn AXUIElementPerformAction(element: Ref, action: Ref) -> i32;
    fn AXUIElementSetAttributeValue(element: Ref, name: Ref, value: Ref) -> i32;
    fn AXUIElementIsAttributeSettable(element: Ref, name: Ref, settable: *mut u8) -> i32;
    fn AXUIElementSetMessagingTimeout(element: Ref, seconds: f32) -> i32;
    fn AXValueGetValue(value: Ref, kind: u32, out: *mut c_void) -> u8;
}

pub struct Tree {
    pub rows: Vec<Value>,
    pub nodes: Vec<(String, Owned)>,
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn attr(element: Ref, name: &str) -> Option<Owned> {
    let key = cf::string(name);
    let mut value: Ref = std::ptr::null();
    let done = unsafe { AXUIElementCopyAttributeValue(element, key.0, &mut value) };
    if done != 0 {
        return None;
    }
    cf::owned(value)
}

pub fn text(element: Ref, name: &str) -> String {
    attr(element, name).and_then(|v| cf::text_of(v.0)).unwrap_or_default()
}

fn pair(element: Ref, name: &str, kind: u32) -> Option<(f64, f64)> {
    let value = attr(element, name)?;
    let mut out = [0f64; 2];
    (unsafe { AXValueGetValue(value.0, kind, out.as_mut_ptr() as *mut c_void) } != 0).then_some((out[0], out[1]))
}

pub fn bounds(element: Ref) -> Option<Bounds> {
    let (x, y) = pair(element, "AXPosition", POINT)?;
    let (width, height) = pair(element, "AXSize", SIZE)?;
    Some(Bounds { x, y, width, height })
}

pub fn app(pid: i64) -> Result<Owned, String> {
    let pid = i32::try_from(pid).map_err(|_| "Bad process id.".to_string())?;
    let element = cf::owned(unsafe { AXUIElementCreateApplication(pid) }).ok_or("The app is gone.")?;
    unsafe { AXUIElementSetMessagingTimeout(element.0, WAIT_SECS) };
    Ok(element)
}

pub fn window_of(target: &Win) -> Result<Owned, String> {
    if !trusted() {
        return Err("Accessibility permission is needed for desktop controls. The user turns it on in System Settings, Privacy & Security, Accessibility.".into());
    }
    let app = app(target.pid)?;
    let list = attr(app.0, "AXWindows").ok_or("The app shows no accessible windows.")?;
    let found: Vec<Ref> = cf::items(list.0)
        .into_iter()
        .filter(|w| text(*w, "AXTitle") == target.title && bounds(*w).is_some_and(|b| bounds_close(&b, &target.bounds)))
        .collect();
    if found.len() != 1 {
        return Err("Cannot tie the accessible window to its capture. Nothing was clicked or typed.".into());
    }
    Ok(cf::retain(found[0]))
}

fn walk(element: Ref, path: String, depth: usize, tree: &mut Tree) {
    if tree.rows.len() >= MAX_ELEMENTS || depth > MAX_DEPTH {
        return;
    }
    let role = text(element, "AXRole");
    let subrole = text(element, "AXSubrole");
    let label = label_of(&[text(element, "AXTitle"), text(element, "AXDescription"), text(element, "AXHelp")]);
    let enabled = attr(element, "AXEnabled").and_then(|v| cf::bool_of(v.0)).unwrap_or(false);
    let value = attr(element, "AXValue").and_then(|v| cf::text_of(v.0));
    let url = attr(element, "AXURL").and_then(|v| cf::url_text(v.0));
    let (row, secret) = element_row(&path, &role, &subrole, &label, enabled, value.as_deref(), url.as_deref());
    tree.rows.push(row);
    tree.nodes.push((path.clone(), cf::retain(element)));
    if secret {
        return;
    }
    let Some(children) = attr(element, "AXChildren") else { return };
    for (index, child) in cf::items(children.0).into_iter().enumerate() {
        walk(child, format!("{path}.{index}"), depth + 1, tree);
    }
}

pub fn tree(root: Ref) -> Tree {
    let mut tree = Tree { rows: Vec::new(), nodes: Vec::new() };
    walk(root, "0".into(), 0, &mut tree);
    tree
}

pub fn press(element: Ref) -> Result<(), String> {
    let action = cf::string("AXPress");
    match unsafe { AXUIElementPerformAction(element, action.0) } {
        0 => Ok(()),
        code => Err(format!("Accessibility input returned {code}. Inspect before retrying.")),
    }
}

pub fn set_text(element: Ref, value: &str) -> Result<(), String> {
    let name = cf::string("AXValue");
    let mut settable = 0u8;
    if unsafe { AXUIElementIsAttributeSettable(element, name.0, &mut settable) } != 0 || settable == 0 {
        return Err("This field does not take safe accessibility text entry.".into());
    }
    let text = cf::string(value);
    match unsafe { AXUIElementSetAttributeValue(element, name.0, text.0) } {
        0 => Ok(()),
        code => Err(format!("Accessibility input returned {code}. Inspect before retrying.")),
    }
}
