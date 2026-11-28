use super::perms::{Grant, Perm};
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

type Id = *mut c_void;
type Msg2 = unsafe extern "C" fn(Id, Id, Id) -> isize;
type Msg3 = unsafe extern "C" fn(Id, Id, Id, *const c_void);

const LISTEN_EVENT: u32 = 1;
const BLOCK_IS_GLOBAL: i32 = 1 << 28;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFDictionaryCreate(
        alloc: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: Id;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    fn objc_msgSend();
}

unsafe extern "C" {
    static _NSConcreteGlobalBlock: u8;
}

#[repr(C)]
struct Descriptor {
    reserved: usize,
    size: usize,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*const Block, u8),
    descriptor: *const Descriptor,
}

struct BlockAt(usize);

static DESCRIPTOR: Descriptor = Descriptor { reserved: 0, size: std::mem::size_of::<Block>() };
static DONE_BLOCK: OnceLock<BlockAt> = OnceLock::new();

unsafe extern "C" fn answered(_: *const Block, _: u8) {}

fn done_block() -> *const c_void {
    let at = DONE_BLOCK.get_or_init(|| {
        let block = Box::new(Block {
            isa: &raw const _NSConcreteGlobalBlock as *const c_void,
            flags: BLOCK_IS_GLOBAL,
            reserved: 0,
            invoke: answered,
            descriptor: &DESCRIPTOR,
        });
        BlockAt(Box::leak(block) as *const Block as usize)
    });
    at.0 as *const c_void
}

fn capture_class() -> Option<(Id, Id)> {
    unsafe {
        let class = objc_getClass(c"AVCaptureDevice".as_ptr());
        let media = AVMediaTypeAudio;
        (!class.is_null() && !media.is_null()).then_some((class, media))
    }
}

fn mic_status() -> Grant {
    let Some((class, media)) = capture_class() else {
        return Grant::Unknown;
    };
    let code = unsafe {
        let send = std::mem::transmute::<unsafe extern "C" fn(), Msg2>(objc_msgSend);
        send(class, sel_registerName(c"authorizationStatusForMediaType:".as_ptr()), media)
    };
    match code {
        0 => Grant::NotAsked,
        1 | 2 => Grant::Denied,
        3 => Grant::Allowed,
        _ => Grant::Unknown,
    }
}

fn mic_ask() {
    let Some((class, media)) = capture_class() else {
        return;
    };
    unsafe {
        let send = std::mem::transmute::<unsafe extern "C" fn(), Msg3>(objc_msgSend);
        send(class, sel_registerName(c"requestAccessForMediaType:completionHandler:".as_ptr()), media, done_block());
    }
}

fn input_status() -> Grant {
    match unsafe { IOHIDCheckAccess(LISTEN_EVENT) } {
        0 => Grant::Allowed,
        1 => Grant::Denied,
        2 => Grant::NotAsked,
        _ => Grant::Unknown,
    }
}

fn ax_ask() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &raw const kCFTypeDictionaryKeyCallBacks as *const c_void,
            &raw const kCFTypeDictionaryValueCallBacks as *const c_void,
        );
        if dict.is_null() {
            return;
        }
        AXIsProcessTrustedWithOptions(dict);
        CFRelease(dict);
    }
}

fn yes_no(on: bool) -> Grant {
    if on { Grant::Allowed } else { Grant::Denied }
}

pub fn check(p: Perm) -> Grant {
    match p {
        Perm::Mic => mic_status(),
        Perm::InputMonitoring => input_status(),
        Perm::Accessibility => yes_no(unsafe { AXIsProcessTrusted() } != 0),
        Perm::ScreenRecording => yes_no(unsafe { CGPreflightScreenCaptureAccess() }),
    }
}

pub fn ask(p: Perm) {
    match p {
        Perm::Mic => mic_ask(),
        Perm::InputMonitoring => {
            unsafe { IOHIDRequestAccess(LISTEN_EVENT) };
        }
        Perm::Accessibility => ax_ask(),
        Perm::ScreenRecording => {
            unsafe { CGRequestScreenCaptureAccess() };
        }
    }
}
