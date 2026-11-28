use crate::brains::SavedBrain;
use crate::provider::Kind;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub const KEYCHAIN_SERVICE: &str = "nixa-brain";

pub fn own_env_name(id: &str) -> String {
    format!("NIXA_BRAIN_KEY_{}", id.to_uppercase())
}

pub fn default_env_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Openai => "OPENAI_API_KEY",
        Kind::Anthropic => "ANTHROPIC_API_KEY",
        Kind::Ollama => "OLLAMA_API_KEY",
    }
}

pub fn env_names(brain: &SavedBrain) -> Vec<String> {
    let mut names = vec![own_env_name(&brain.id)];
    if let Some(name) = brain.key_env.as_deref().filter(|n| crate::brains::key_env_ok(n)) {
        names.push(name.to_string());
    }
    names.push(default_env_name(brain.kind).to_string());
    names
}

pub fn env_key(brain: &SavedBrain) -> Option<String> {
    env_names(brain)
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

pub fn key_is_clean(key: &str) -> bool {
    !key.is_empty() && !key.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\' || c.is_control())
}

pub async fn keychain_key(id: &str) -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", id, "-w"])
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

pub async fn api_key(brain: &SavedBrain) -> Option<String> {
    match env_key(brain) {
        Some(key) => Some(key),
        None => keychain_key(&brain.id).await,
    }
}

pub async fn save_keychain_key(id: &str, key: &str) -> anyhow::Result<()> {
    if !key_is_clean(key) {
        anyhow::bail!("The key has a space or a quote in it. Paste it again.");
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        anyhow::bail!("Bad brain id.");
    }
    let mut child = Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let line = format!("add-generic-password -U -s {KEYCHAIN_SERVICE} -a {id} -w \"{key}\"\n");
        stdin.write_all(line.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("The macOS keychain did not save the key.");
    }
    Ok(())
}

pub async fn delete_keychain_key(id: &str) -> bool {
    Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn key_tail(key: &str) -> String {
    if key.chars().count() > 8 {
        let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    } else if key.is_empty() {
        String::new()
    } else {
        "set".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_order_and_tail() {
        let brain = SavedBrain {
            id: "ab12cd34".into(),
            name: "x".into(),
            kind: Kind::Anthropic,
            base_url: "u".into(),
            model: String::new(),
            effort: None,
            key_env: Some("MY_API_KEY".into()),
            added_at: 0,
        };
        assert_eq!(env_names(&brain), vec!["NIXA_BRAIN_KEY_AB12CD34", "MY_API_KEY", "ANTHROPIC_API_KEY"]);
        let other = SavedBrain { key_env: Some("GITHUB_TOKEN".into()), ..brain.clone() };
        assert_eq!(env_names(&other), vec!["NIXA_BRAIN_KEY_AB12CD34", "ANTHROPIC_API_KEY"]);
        assert_eq!(key_tail("abcdefghijkl"), "…ijkl");
        assert_eq!(key_tail("abc"), "set");
        assert!(!key_is_clean("a b"));
        assert!(!key_is_clean("a\"b"));
        assert!(key_is_clean("abc-123"));
    }
}
