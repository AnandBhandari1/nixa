use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

const SCREENCAPTURE: &str = "/usr/sbin/screencapture";
pub const NO_GRANT: &str = "Screen Recording is not on for this Nixa build, so no picture was taken. To turn it on: System Settings, Privacy & Security, Screen Recording. If Nixa is already in that list, remove it with the minus button and add /Applications/Nixa.app again, because a grant given to an older build does not match a newly signed build. Then quit and reopen Nixa. Without the grant, nixa screenshot --window nixa or --name NAME shows Nixa's own windows, nixa tool call nixa_eyes '{\"action\": \"windows\"}' lists window places, and nixa term read ID reads a terminal.";

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

pub fn screen_access() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub fn need_access() -> Result<(), String> {
    if screen_access() {
        return Ok(());
    }
    Err(NO_GRANT.into())
}

pub fn capture(image: &Path, window: Option<i64>) -> Result<u64, String> {
    need_access()?;
    if let Some(dir) = image.parent() {
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(dir).map_err(|e| format!("The capture folder cannot be made: {e}"))?;
    }
    let mut args: Vec<String> = vec!["-x".into()];
    if let Some(id) = window {
        args.extend(["-o".into(), "-l".into(), id.to_string()]);
    }
    let out = Command::new(SCREENCAPTURE)
        .args(&args)
        .arg(image)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("The screen capture could not start: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if why.is_empty() { "The screen capture failed. Nothing was captured.".into() } else { format!("The screen capture failed: {why}") });
    }
    std::fs::set_permissions(image, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("The capture is missing: {e}"))?;
    let bytes = std::fs::metadata(image).map(|m| m.len()).unwrap_or(0);
    if bytes == 0 {
        return Err("The screen capture made an empty file. Nothing was captured.".into());
    }
    Ok(bytes)
}
