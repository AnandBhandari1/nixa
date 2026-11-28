use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub const SERVICE: &str = "nixa-connector";
pub const MAX_SECRET: usize = 4000;
pub const SYSTEM_TOOL: &str = "/usr/bin/security";

#[derive(Debug, Clone, PartialEq)]
pub struct Keychain {
    pub bin: PathBuf,
    pub file: Option<PathBuf>,
}

pub fn system() -> Keychain {
    Keychain { bin: PathBuf::from(SYSTEM_TOOL), file: None }
}

pub fn check_entry(entry: &str) -> Result<(), String> {
    if super::manifest::valid_name(entry) {
        Ok(())
    } else {
        Err(format!("\"{entry}\" is not a secret entry name."))
    }
}

pub fn check_secret(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("Nothing typed. Nothing saved.".into());
    }
    if value.chars().count() > MAX_SECRET {
        return Err(format!("The secret is longer than {MAX_SECRET} characters."));
    }
    if !value.chars().all(|c| (' '..='~').contains(&c)) {
        return Err("The secret must be plain printable text on one line.".into());
    }
    Ok(())
}

pub fn account(connector: &str, entry: &str) -> Result<String, String> {
    check_entry(connector)?;
    check_entry(entry)?;
    Ok(format!("{connector}:{entry}"))
}

pub fn hex(value: &str) -> String {
    value.bytes().map(|b| format!("{b:02x}")).collect()
}

fn file_arg(kc: &Keychain) -> Result<Option<String>, String> {
    match &kc.file {
        None => Ok(None),
        Some(path) => {
            let text = path.to_string_lossy().to_string();
            if text.contains(['"', '\\', '\n']) {
                return Err("The keychain file path is not usable.".into());
            }
            Ok(Some(text))
        }
    }
}

pub fn save_line(kc: &Keychain, connector: &str, entry: &str, value: &str) -> Result<String, String> {
    let account = account(connector, entry)?;
    check_secret(value)?;
    let file = file_arg(kc)?.map(|f| format!(" \"{f}\"")).unwrap_or_default();
    Ok(format!("add-generic-password -U -s {SERVICE} -a {account} -X {}{file}\n", hex(value)))
}

fn with_file(kc: &Keychain, mut args: Vec<String>) -> Result<Vec<String>, String> {
    if let Some(file) = file_arg(kc)? {
        args.push(file);
    }
    Ok(args)
}

pub async fn get(kc: &Keychain, connector: &str, entry: &str) -> Option<String> {
    find(kc, &account(connector, entry).ok()?).await
}

pub async fn get_legacy(kc: &Keychain, entry: &str) -> Option<String> {
    check_entry(entry).ok()?;
    find(kc, entry).await
}

async fn find(kc: &Keychain, account: &str) -> Option<String> {
    let args = with_file(kc, vec!["find-generic-password".into(), "-s".into(), SERVICE.into(), "-a".into(), account.into(), "-w".into()]).ok()?;
    let out = Command::new(&kc.bin).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub async fn save(kc: &Keychain, connector: &str, entry: &str, value: &str) -> Result<(), String> {
    let line = save_line(kc, connector, entry, value)?;
    let mut child = Command::new(&kc.bin)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "The macOS keychain tool did not start.".to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(line.as_bytes()).await.map_err(|_| "The macOS keychain did not take the secret.".to_string())?;
        stdin.shutdown().await.map_err(|_| "The macOS keychain did not take the secret.".to_string())?;
    }
    let status = child.wait().await.map_err(|_| "The macOS keychain did not save the secret.".to_string())?;
    if !status.success() {
        return Err("The macOS keychain did not save the secret.".into());
    }
    if get(kc, connector, entry).await.as_deref() != Some(value) {
        return Err("The macOS keychain did not keep the secret.".into());
    }
    Ok(())
}

pub async fn remove(kc: &Keychain, connector: &str, entry: &str) -> bool {
    match account(connector, entry) {
        Ok(account) => delete(kc, &account).await,
        Err(_) => false,
    }
}

pub async fn remove_legacy(kc: &Keychain, entry: &str) -> bool {
    check_entry(entry).is_ok() && delete(kc, entry).await
}

async fn delete(kc: &Keychain, account: &str) -> bool {
    let Ok(args) = with_file(kc, vec!["delete-generic-password".into(), "-s".into(), SERVICE.into(), "-a".into(), account.into()]) else { return false };
    Command::new(&kc.bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
