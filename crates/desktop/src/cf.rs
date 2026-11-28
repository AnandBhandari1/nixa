use serde_json::{Map, Value, json};
use std::ffi::{c_char, c_void};

pub type Ref = *const c_void;

const UTF8: u32 = 0x0800_0100;
const SINT64: isize = 4;
const FLOAT64: isize = 6;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFRetain(value: Ref) -> Ref;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFURLGetTypeID() -> usize;
    fn CFStringGetLength(text: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(text: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFStringCreateWithBytes(alloc: Ref, bytes: *const u8, length: isize, encoding: u32, external: u8) -> Ref;
    fn CFNumberIsFloatType(number: Ref) -> u8;
    fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> u8;
    fn CFBooleanGetValue(flag: Ref) -> u8;
    fn CFArrayGetCount(list: Ref) -> isize;
    fn CFArrayGetValueAtIndex(list: Ref, index: isize) -> Ref;
    fn CFDictionaryGetCount(map: Ref) -> isize;
    fn CFDictionaryGetKeysAndValues(map: Ref, keys: *mut Ref, values: *mut Ref);
    fn CFURLGetString(url: Ref) -> Ref;
}

pub struct Owned(pub Ref);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

pub fn owned(value: Ref) -> Option<Owned> {
    (!value.is_null()).then_some(Owned(value))
}

pub fn retain(value: Ref) -> Owned {
    if value.is_null() {
        return Owned(value);
    }
    Owned(unsafe { CFRetain(value) })
}

pub fn string(text: &str) -> Owned {
    Owned(unsafe { CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), text.len() as isize, UTF8, 0) })
}

fn kind(value: Ref) -> usize {
    unsafe { CFGetTypeID(value) }
}

pub fn is_string(value: Ref) -> bool {
    !value.is_null() && kind(value) == unsafe { CFStringGetTypeID() }
}

pub fn is_array(value: Ref) -> bool {
    !value.is_null() && kind(value) == unsafe { CFArrayGetTypeID() }
}

pub fn text_of(value: Ref) -> Option<String> {
    if !is_string(value) {
        return None;
    }
    let length = unsafe { CFStringGetLength(value) };
    let size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) } + 1;
    if size <= 0 {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    if unsafe { CFStringGetCString(value, buffer.as_mut_ptr() as *mut c_char, size, UTF8) } == 0 {
        return None;
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

pub fn bool_of(value: Ref) -> Option<bool> {
    if value.is_null() {
        return None;
    }
    let id = kind(value);
    if id == unsafe { CFBooleanGetTypeID() } {
        return Some(unsafe { CFBooleanGetValue(value) } != 0);
    }
    if id == unsafe { CFNumberGetTypeID() } {
        return number_of(value).map(|n| n.as_f64().unwrap_or(0.0) != 0.0);
    }
    None
}

fn number_of(value: Ref) -> Option<Value> {
    if unsafe { CFNumberIsFloatType(value) } != 0 {
        let mut out = 0f64;
        (unsafe { CFNumberGetValue(value, FLOAT64, &mut out as *mut f64 as *mut c_void) } != 0).then(|| json!(out))
    } else {
        let mut out = 0i64;
        (unsafe { CFNumberGetValue(value, SINT64, &mut out as *mut i64 as *mut c_void) } != 0).then(|| json!(out))
    }
}

pub fn items(list: Ref) -> Vec<Ref> {
    if !is_array(list) {
        return Vec::new();
    }
    let count = unsafe { CFArrayGetCount(list) };
    (0..count).map(|i| unsafe { CFArrayGetValueAtIndex(list, i) }).collect()
}

pub fn url_text(value: Ref) -> Option<String> {
    if value.is_null() || kind(value) != unsafe { CFURLGetTypeID() } {
        return text_of(value);
    }
    text_of(unsafe { CFURLGetString(value) })
}

pub fn json_of(value: Ref, depth: usize) -> Value {
    if value.is_null() || depth > 8 {
        return Value::Null;
    }
    let id = kind(value);
    if id == unsafe { CFStringGetTypeID() } {
        return text_of(value).map(Value::String).unwrap_or(Value::Null);
    }
    if id == unsafe { CFBooleanGetTypeID() } {
        return json!(unsafe { CFBooleanGetValue(value) } != 0);
    }
    if id == unsafe { CFNumberGetTypeID() } {
        return number_of(value).unwrap_or(Value::Null);
    }
    if id == unsafe { CFArrayGetTypeID() } {
        return Value::Array(items(value).into_iter().map(|v| json_of(v, depth + 1)).collect());
    }
    if id == unsafe { CFDictionaryGetTypeID() } {
        let count = unsafe { CFDictionaryGetCount(value) }.max(0) as usize;
        let mut keys: Vec<Ref> = vec![std::ptr::null(); count];
        let mut values: Vec<Ref> = vec![std::ptr::null(); count];
        unsafe { CFDictionaryGetKeysAndValues(value, keys.as_mut_ptr(), values.as_mut_ptr()) };
        let mut map = Map::new();
        for (key, item) in keys.into_iter().zip(values) {
            if let Some(name) = text_of(key) {
                map.insert(name, json_of(item, depth + 1));
            }
        }
        return Value::Object(map);
    }
    if id == unsafe { CFURLGetTypeID() } {
        return url_text(value).map(Value::String).unwrap_or(Value::Null);
    }
    Value::Null
}
