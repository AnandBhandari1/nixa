use crate::cf;
use crate::rows::{AppWin, Win, app_windows_of, is_app_path, windows_of};
use serde_json::Value;
use std::ffi::c_void;

pub(crate) const ALL: u32 = 0;
const ON_SCREEN_ONLY: u32 = 1;
pub(crate) const EXCLUDE_DESKTOP: u32 = 1 << 4;
const PATH_MAX: usize = 4096;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> cf::Ref;
}

unsafe extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut c_void, size: u32) -> i32;
}

pub(crate) fn raw(option: u32) -> Vec<Value> {
    let Some(list) = cf::owned(unsafe { CGWindowListCopyWindowInfo(option, 0) }) else { return Vec::new() };
    match cf::json_of(list.0, 0) {
        Value::Array(rows) => rows,
        _ => Vec::new(),
    }
}

pub fn path_of(pid: i64) -> String {
    let Ok(pid) = i32::try_from(pid) else { return String::new() };
    let mut buffer = vec![0u8; PATH_MAX];
    let size = unsafe { proc_pidpath(pid, buffer.as_mut_ptr() as *mut c_void, PATH_MAX as u32) };
    if size <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buffer[..size as usize]).into_owned()
}

pub fn windows() -> Vec<Win> {
    windows_of(&raw(ON_SCREEN_ONLY | EXCLUDE_DESKTOP), std::process::id() as i64)
}

pub fn app_windows() -> Vec<AppWin> {
    let mut known: Vec<(i64, bool)> = Vec::new();
    let mut is_app = |pid: i64| {
        if let Some((_, yes)) = known.iter().find(|(p, _)| *p == pid) {
            return *yes;
        }
        let yes = is_app_path(&path_of(pid));
        known.push((pid, yes));
        yes
    };
    let rows = raw(ALL | EXCLUDE_DESKTOP);
    let pids: Vec<i64> = rows.iter().filter_map(|r| r.get("kCGWindowOwnerPID").and_then(Value::as_i64)).collect();
    let apps: Vec<i64> = pids.into_iter().filter(|p| is_app(*p)).collect();
    app_windows_of(&rows, |pid| apps.contains(&pid))
}

pub fn window(id: i64) -> Result<Win, String> {
    windows().into_iter().find(|w| w.id == id).ok_or_else(|| "The window is gone or not visible. List windows again.".to_string())
}
