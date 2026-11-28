use std::process::Command;

pub const KEYCHAIN_SERVICE: &str = "nixa";

pub fn key(name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(name)
        && !value.trim().is_empty()
    {
        return Some(value.trim().to_string());
    }
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", name, "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}
