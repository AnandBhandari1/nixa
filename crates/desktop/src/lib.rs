pub mod picture;
pub mod rows;

#[cfg(target_os = "macos")]
mod ax;
#[cfg(target_os = "macos")]
mod capture;
#[cfg(target_os = "macos")]
mod cf;
#[cfg(target_os = "macos")]
mod list;
#[cfg(target_os = "macos")]
mod ocr;
#[cfg(target_os = "macos")]
mod ops;
#[cfg(target_os = "macos")]
mod own;

use serde_json::Value;

#[cfg(target_os = "macos")]
pub fn execute(input: &Value) -> Result<Value, String> {
    ops::execute(input)
}

#[cfg(not(target_os = "macos"))]
pub fn execute(_: &Value) -> Result<Value, String> {
    Err("Desktop eyes and hands need macOS. Nothing was captured and no input was sent.".into())
}

#[cfg(target_os = "macos")]
pub fn own_shot(name: &str, image: &std::path::Path) -> Result<Value, String> {
    own::shot(name, image)
}

#[cfg(not(target_os = "macos"))]
pub fn own_shot(_: &str, _: &std::path::Path) -> Result<Value, String> {
    Err("A picture of the Nixa window needs macOS. Nothing was captured.".into())
}
