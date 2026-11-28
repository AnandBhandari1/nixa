use crate::rows::order_lines;
use objc2::encode::{Encode, Encoding};
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use std::ffi::CString;
use std::path::Path;

const ACCURATE: isize = 0;

#[link(name = "Vision", kind = "framework")]
unsafe extern "C" {}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    origin: Point,
    size: Size,
}

unsafe impl Encode for Point {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

unsafe impl Encode for Size {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

unsafe impl Encode for Rect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}

fn class(name: &str) -> Result<&'static AnyClass, String> {
    let name = CString::new(name).map_err(|_| "bad class name".to_string())?;
    AnyClass::get(&name).ok_or_else(|| "Apple Vision text reading is not available on this Mac.".to_string())
}

unsafe fn ns_text(text: &str) -> Result<Retained<AnyObject>, String> {
    let c = CString::new(text).map_err(|_| "The path has a zero byte.".to_string())?;
    let made: Option<Retained<AnyObject>> = unsafe { msg_send![class("NSString")?, stringWithUTF8String: c.as_ptr()] };
    made.ok_or_else(|| "The text could not be made.".to_string())
}

unsafe fn rust_text(value: &AnyObject) -> String {
    let raw: *const std::ffi::c_char = unsafe { msg_send![value, UTF8String] };
    if raw.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned()
}

pub fn read_text(image: &Path) -> Result<String, String> {
    let path = image.to_str().ok_or("The capture path is not plain text.")?.to_string();
    autoreleasepool(|_| unsafe {
        let path = ns_text(&path)?;
        let url: Option<Retained<AnyObject>> = msg_send![class("NSURL")?, fileURLWithPath: &*path];
        let url = url.ok_or("The capture path is not valid.")?;
        let request: Option<Retained<AnyObject>> = msg_send![class("VNRecognizeTextRequest")?, new];
        let request = request.ok_or("Apple Vision text reading could not start.")?;
        let _: () = msg_send![&*request, setRecognitionLevel: ACCURATE];
        let options: Option<Retained<AnyObject>> = msg_send![class("NSDictionary")?, dictionary];
        let options = options.ok_or("Apple Vision options could not be made.")?;
        let handler: Allocated<AnyObject> = msg_send![class("VNImageRequestHandler")?, alloc];
        let handler: Option<Retained<AnyObject>> = msg_send![handler, initWithURL: &*url, options: &*options];
        let handler = handler.ok_or("Apple Vision could not open the capture.")?;
        let requests: Option<Retained<AnyObject>> = msg_send![class("NSArray")?, arrayWithObject: &*request];
        let requests = requests.ok_or("Apple Vision could not start.")?;
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let done: Bool = msg_send![&*handler, performRequests: &*requests, error: &mut error];
        if !done.as_bool() {
            let why = if error.is_null() {
                String::new()
            } else {
                let text: Option<Retained<AnyObject>> = msg_send![&*error, localizedDescription];
                text.map(|t| rust_text(&t)).unwrap_or_default()
            };
            return Err(format!("Apple Vision could not read the capture. {why}").trim().to_string());
        }
        let results: Option<Retained<AnyObject>> = msg_send![&*request, results];
        let Some(results) = results else { return Ok(String::new()) };
        let count: usize = msg_send![&*results, count];
        let mut lines: Vec<(f64, String)> = Vec::new();
        for index in 0..count {
            let item: Option<Retained<AnyObject>> = msg_send![&*results, objectAtIndex: index];
            let Some(item) = item else { continue };
            let rect: Rect = msg_send![&*item, boundingBox];
            let top: Option<Retained<AnyObject>> = msg_send![&*item, topCandidates: 1usize];
            let Some(top) = top else { continue };
            let first: Option<Retained<AnyObject>> = msg_send![&*top, firstObject];
            let Some(first) = first else { continue };
            let text: Option<Retained<AnyObject>> = msg_send![&*first, string];
            if let Some(text) = text {
                lines.push((rect.origin.y + rect.size.height / 2.0, rust_text(&text)));
            }
        }
        Ok(order_lines(lines))
    })
}
