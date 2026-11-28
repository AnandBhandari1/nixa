use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

pub const BIN_NAME: &str = "nixa";
pub const KEEP_LINES: usize = 4;
pub const CREDIT: &str = "Speech to text: Parakeet TDT 0.6B v3 by NVIDIA, CC BY 4.0 (creativecommons.org/licenses/by/4.0). Voice activity: Silero VAD by the Silero Team, MIT.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Models {
    pub ready: bool,
    pub dir: String,
    pub missing: Vec<String>,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    Line(String),
    Done(Result<(), String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelsState {
    Loading,
    Known(Models),
    Fetching { line: String },
    Failed(String),
}

pub fn nixa_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(BIN_NAME)))
        .unwrap_or_else(|| PathBuf::from(BIN_NAME))
}

pub fn parse_status(v: &Value) -> Option<Models> {
    let list = v.get("models")?.as_array()?;
    let not_ready: Vec<&Value> = list
        .iter()
        .filter(|m| m.get("ready").and_then(Value::as_bool) != Some(true))
        .collect();
    Some(Models {
        ready: v.get("ready").and_then(Value::as_bool).unwrap_or(false) && not_ready.is_empty(),
        dir: v.get("dir").and_then(Value::as_str).unwrap_or_default().to_string(),
        missing: not_ready
            .iter()
            .map(|m| {
                m.get("what")
                    .or_else(|| m.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("voice model")
                    .to_string()
            })
            .collect(),
        bytes: not_ready
            .iter()
            .filter_map(|m| m.get("download_size").and_then(Value::as_u64))
            .sum(),
    })
}

pub fn size_words(bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    if bytes < MB {
        return "less than 1 MB".into();
    }
    format!("about {} MB", (bytes + MB / 2) / MB)
}

pub fn status() -> Result<Models, String> {
    let bin = nixa_bin();
    let out = Command::new(&bin)
        .args(["--json", "voice", "models", "status"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Cannot run {}: {e}", bin.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let value: Value = text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok())
        .ok_or_else(|| last_words(&String::from_utf8_lossy(&out.stderr), "The voice model check gave no answer."))?;
    parse_status(&value).ok_or_else(|| "The voice model check gave an answer Nixa cannot read.".into())
}

pub fn last_words(text: &str, fallback: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return fallback.to_string();
    }
    lines[lines.len().saturating_sub(KEEP_LINES)..].join(" ")
}

pub fn progress(line: &str) -> Option<String> {
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
}

pub fn fetch(tx: Sender<Fetch>) {
    std::thread::spawn(move || {
        let bin = nixa_bin();
        let child = Command::new(&bin)
            .args(["voice", "models", "fetch"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                let _ = tx.send(Fetch::Done(Err(format!("Cannot run {}: {e}", bin.display()))));
                return;
            }
        };
        let mut seen = Vec::new();
        if let Some(err) = child.stderr.take() {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if let Some(line) = progress(&line) {
                    seen.push(line.clone());
                    if tx.send(Fetch::Line(line)).is_err() {
                        break;
                    }
                }
            }
        }
        let done = match child.wait() {
            Ok(code) if code.success() => Ok(()),
            Ok(_) => Err(last_words(&seen.join("\n"), "The download stopped.")),
            Err(e) => Err(format!("The download stopped: {e}")),
        };
        let _ = tx.send(Fetch::Done(done));
    });
}
