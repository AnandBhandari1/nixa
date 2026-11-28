use crate::cf;
use crate::list;
use crate::rows::{AppWin, app_windows_of, is_part_name, pick_app_window};
use serde_json::{Value, json};
use std::ffi::{c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const INCLUDING_WINDOW: u32 = 1 << 3;
const IGNORE_FRAMING: u32 = 1;
const PNG_TYPE: &str = "public.png";
const CREATE_IMAGE: &std::ffi::CStr = c"CGWindowListCreateImage";

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: Point,
    size: Size,
}

type WindowImage = unsafe extern "C" fn(Rect, u32, u32, u32) -> cf::Ref;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: cf::Ref) -> usize;
    fn CGImageGetHeight(image: cf::Ref) -> usize;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithURL(url: cf::Ref, kind: cf::Ref, count: usize, options: cf::Ref) -> cf::Ref;
    fn CGImageDestinationAddImage(destination: cf::Ref, image: cf::Ref, properties: cf::Ref);
    fn CGImageDestinationFinalize(destination: cf::Ref) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(alloc: cf::Ref, buffer: *const u8, length: isize, is_directory: u8) -> cf::Ref;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

fn any_image() -> *mut c_void {
    -2isize as *mut c_void
}

fn window_image() -> Option<WindowImage> {
    let found = unsafe { dlsym(any_image(), CREATE_IMAGE.as_ptr()) };
    if found.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute::<*mut c_void, WindowImage>(found) })
}

pub fn own_windows() -> Vec<AppWin> {
    let me = std::process::id() as i64;
    app_windows_of(&list::raw(list::ALL | list::EXCLUDE_DESKTOP), |pid| pid == me)
}

fn save_png(image: cf::Ref, path: &Path) -> Result<(), String> {
    let bytes = path.as_os_str().as_bytes();
    let url = cf::owned(unsafe { CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, 0) })
        .ok_or("The picture place is not valid. Nothing was captured.")?;
    let kind = cf::string(PNG_TYPE);
    let destination = cf::owned(unsafe { CGImageDestinationCreateWithURL(url.0, kind.0, 1, std::ptr::null()) })
        .ok_or("The picture file could not be made. Nothing was captured.")?;
    unsafe { CGImageDestinationAddImage(destination.0, image, std::ptr::null()) };
    if !unsafe { CGImageDestinationFinalize(destination.0) } {
        return Err("The picture could not be written. Nothing was captured.".into());
    }
    Ok(())
}

pub fn capture_own(window: i64, image: &Path) -> Result<u64, String> {
    let id = u32::try_from(window).map_err(|_| "Bad window id.".to_string())?;
    let create = window_image().ok_or("This macOS has no way for Nixa to draw its own window. Nothing was captured.")?;
    if let Some(dir) = image.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("The capture folder cannot be made: {e}"))?;
    }
    let everything = Rect { origin: Point { x: f64::INFINITY, y: f64::INFINITY }, size: Size { width: 0.0, height: 0.0 } };
    let picture = cf::owned(unsafe { create(everything, INCLUDING_WINDOW, id, IGNORE_FRAMING) })
        .ok_or("The Nixa window picture came back empty. Nothing was captured.")?;
    let (width, height) = unsafe { (CGImageGetWidth(picture.0), CGImageGetHeight(picture.0)) };
    if width <= 1 || height <= 1 {
        return Err("The Nixa window picture came back empty. Is the window on screen? Nothing was captured.".into());
    }
    save_png(picture.0, image)?;
    std::fs::set_permissions(image, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("The capture is missing: {e}"))?;
    let bytes = std::fs::metadata(image).map(|m| m.len()).unwrap_or(0);
    if bytes == 0 {
        return Err("The Nixa window picture made an empty file. Nothing was captured.".into());
    }
    Ok(bytes)
}

pub fn shot(name: &str, image: &Path) -> Result<Value, String> {
    let windows = own_windows();
    let found = pick_app_window(&windows, name)?.clone();
    let bytes = capture_own(found.id, image)?;
    let mut out = json!({"image": image, "window": found.title, "id": found.id, "bytes": bytes, "own": true});
    if is_part_name(name) {
        out["note"] = json!(format!("The {name} is drawn inside the main window, so this is the whole main window."));
    }
    Ok(out)
}
