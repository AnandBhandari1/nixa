use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

pub const MAX_OUT: u64 = 1024 * 1024;
pub const MAX_ERR: u64 = 4096;
pub const MAX_IMAGE: u64 = 20 * 1024 * 1024;
pub const MAX_NOTE: usize = 500;
pub const PRIVATE_FOLDER: &str = "private";
pub const ENV_KEEP: [&str; 4] = ["HOME", "PATH", "TMPDIR", "LANG"];
pub const IMAGE_TYPES: [(&str, &str); 4] = [("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("png", "image/png"), ("webp", "image/webp")];

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub path: PathBuf,
    pub mime: String,
    pub bytes: u64,
    pub note: String,
}

pub fn program(folder: &Path, name: &str) -> Result<PathBuf, String> {
    let root = folder.canonicalize().map_err(|_| "The connector folder was not found.".to_string())?;
    let file = root.join(name).canonicalize().map_err(|_| format!("The program {name} was not found in the connector folder."))?;
    if !file.starts_with(&root) || file == root {
        return Err("The program is outside the connector folder.".into());
    }
    let info = std::fs::metadata(&file).map_err(|_| format!("The program {name} was not found."))?;
    let mode = info.permissions().mode();
    if !info.is_file() || mode & 0o111 == 0 {
        return Err(format!("The program {name} is not a runnable file. Run chmod +x on it."));
    }
    if mode & 0o022 != 0 {
        return Err(format!("The program {name} can be changed by other users, so it does not run."));
    }
    let folder_mode = std::fs::metadata(&root).map(|m| m.permissions().mode()).unwrap_or(0o777);
    if folder_mode & 0o002 != 0 {
        return Err("The connector folder can be changed by everyone, so its program does not run.".into());
    }
    Ok(file)
}

pub fn env_name(entry: &str) -> String {
    format!("NIXA_SECRET_{}", entry.to_uppercase().replace('-', "_"))
}

pub fn private_folder(root: &Path, connector: &str) -> Result<PathBuf, String> {
    let base = root.join(PRIVATE_FOLDER);
    let dir = base.join(connector);
    nixa_protocol::paths::ensure_private_dir(&base).and_then(|_| nixa_protocol::paths::ensure_private_dir(&dir)).map_err(|e| format!("No private folder for connector files: {e}"))?;
    Ok(dir)
}

async fn read_some<R: tokio::io::AsyncRead + Unpin>(reader: Option<R>, max: u64) -> Vec<u8> {
    let mut data = Vec::new();
    if let Some(reader) = reader {
        let _ = reader.take(max + 1).read_to_end(&mut data).await;
    }
    data
}

pub struct Job<'a> {
    pub program: &'a Path,
    pub folder: &'a Path,
    pub tool: &'a str,
    pub input: &'a Map<String, Value>,
    pub secrets: &'a HashMap<String, String>,
    pub dir: Option<&'a Path>,
}

pub async fn run(job: Job<'_>) -> Result<String, String> {
    let mut command = Command::new(job.program);
    command.arg(job.tool).current_dir(job.folder).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for key in ENV_KEEP {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    command.env("NIXA_CONNECTOR_ACTION", job.tool);
    if let Some(dir) = job.dir {
        command.env("NIXA_CONNECTOR_DIR", dir);
    }
    for (entry, value) in job.secrets {
        command.env(env_name(entry), value);
    }
    let mut child = command.spawn().map_err(|e| format!("The program did not start: {e}"))?;
    let message = json!({"action": job.tool, "input": job.input}).to_string();
    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            let _ = stdin.write_all(message.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let (out, err) = tokio::join!(read_some(child.stdout.take(), MAX_OUT), read_some(child.stderr.take(), MAX_ERR));
    if out.len() as u64 > MAX_OUT {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return Err(format!("The program printed more than {} MB.", MAX_OUT / 1024 / 1024));
    }
    let status = child.wait().await.map_err(|e| format!("The program did not end well: {e}"))?;
    if !status.success() {
        let why = crate::util::clip_flat(String::from_utf8_lossy(&err).trim(), 300);
        let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into());
        return Err(if why.is_empty() { format!("The program stopped with code {code}.") } else { format!("The program stopped with code {code}: {why}") });
    }
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

pub fn image_result(output: &str, dir: &Path) -> Result<Image, String> {
    let value: Value = serde_json::from_str(output).map_err(|_| "An image action must print {\"path\": ...} to a file in its private folder, never image data.".to_string())?;
    let path = value.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or("An image action must print {\"path\": ...} to a file in its private folder, never image data.")?;
    let file = Path::new(path).canonicalize().map_err(|_| "The image file was not found.".to_string())?;
    let root = dir.canonicalize().map_err(|_| "The private folder was not found.".to_string())?;
    if !file.starts_with(&root) || file == root {
        return Err("The image file is outside the connector's private folder.".into());
    }
    let info = std::fs::metadata(&file).map_err(|_| "The image file was not found.".to_string())?;
    if !info.is_file() || info.len() == 0 {
        return Err("The image file is empty or not a file.".into());
    }
    if info.len() > MAX_IMAGE {
        return Err(format!("The image file is bigger than {} MB.", MAX_IMAGE / 1024 / 1024));
    }
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    let mime = IMAGE_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, m)| m.to_string()).ok_or("The image file must be jpg, jpeg, png or webp.")?;
    let note = value.get("note").and_then(Value::as_str).map(|n| crate::util::clip(n.trim(), MAX_NOTE)).unwrap_or_default();
    Ok(Image { path: file, mime, bytes: info.len(), note })
}

pub fn image_text(image: &Image) -> String {
    let mut lines = vec![format!("Image file: {}", image.path.display()), format!("Type: {}, {} KB", image.mime, (image.bytes / 1024).max(1))];
    if !image.note.is_empty() {
        lines.push(image.note.clone());
    }
    lines.join("\n")
}
