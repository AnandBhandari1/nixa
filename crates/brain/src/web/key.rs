use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub const SERVICE: &str = "nixa-web";
pub const EXA: &str = "exa";

pub async fn get(account: &str) -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let key = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!key.is_empty()).then_some(key)
}

pub fn check(account: &str, key: &str) -> Result<(), String> {
    if !crate::keys::key_is_clean(key) {
        return Err("The key has a space or a quote in it. Paste it again.".into());
    }
    if key.chars().count() > 400 {
        return Err("The key is too long.".into());
    }
    if account.is_empty() || !account.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Bad key name.".into());
    }
    Ok(())
}

pub async fn save(account: &str, key: &str) -> Result<(), String> {
    check(account, key)?;
    let mut child = Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "The macOS keychain tool did not start.".to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let line = format!("add-generic-password -U -s {SERVICE} -a {account} -w \"{key}\"\n");
        stdin.write_all(line.as_bytes()).await.map_err(|_| "The macOS keychain did not take the key.".to_string())?;
        stdin.shutdown().await.map_err(|_| "The macOS keychain did not take the key.".to_string())?;
    }
    let status = child.wait().await.map_err(|_| "The macOS keychain did not save the key.".to_string())?;
    if !status.success() {
        return Err("The macOS keychain did not save the key.".into());
    }
    Ok(())
}

pub async fn delete(account: &str) -> bool {
    Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", account])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
