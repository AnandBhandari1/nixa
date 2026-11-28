use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp() -> Temp {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("nixa-voice-cli-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Temp(dir)
}

fn nixa(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nixa"));
    cmd.args(args).env_remove("NIXA_VOICE_MODELS").env_remove("NIXA_VOICE_MODELS_URL");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

fn serve(reply: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let _ = stream.write_all(reply);
        }
    });
    format!("http://{addr}")
}

#[test]
fn status_shows_missing_models_and_fails() {
    let t = temp();
    let dir = t.0.to_string_lossy().to_string();
    let o = nixa(&["voice", "models", "--dir", &dir], &[]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
    let out = text(&o.stdout);
    assert!(out.contains("missing") && out.contains("silero_vad.onnx") && out.contains("encoder.int8.onnx"), "{out}");
    let o = nixa(&["--json", "voice", "models", "status", "--dir", &dir], &[]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ready"], false);
    assert_eq!(v["models"].as_array().unwrap().len(), 2);
}

#[test]
fn models_folder_comes_from_the_env() {
    let t = temp();
    let dir = t.0.join("vm");
    let o = nixa(&["--json", "voice", "models"], &[("NIXA_VOICE_MODELS", dir.to_str().unwrap())]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["dir"].as_str(), dir.to_str());
}

#[test]
fn dry_run_lists_downloads_and_writes_nothing() {
    let t = temp();
    let dir = t.0.join("vm");
    let o = nixa(&["voice", "models", "fetch", "--dry-run", "--dir", dir.to_str().unwrap()], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let out = text(&o.stdout);
    assert!(out.contains("Dry run") && out.contains("silero_vad.onnx") && out.contains(".tar.bz2") && out.contains("sha256"), "{out}");
    assert!(!dir.exists());
    let o = nixa(&["--json", "voice", "models", "fetch", "-n", "--dir", dir.to_str().unwrap()], &[("NIXA_VOICE_MODELS_URL", "http://127.0.0.1:9/m/")]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["download"][0]["url"], "http://127.0.0.1:9/m/silero_vad.onnx");
    assert_eq!(v["bytes"], 643_854 + 487_211_443u64);
}

#[test]
fn wrong_size_download_is_refused() {
    let t = temp();
    let url = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
    let o = nixa(&["voice", "models", "fetch", "--dir", t.0.to_str().unwrap()], &[("NIXA_VOICE_MODELS_URL", &url)]);
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(err.contains("643854"), "{err}");
    assert!(!t.0.join("silero_vad.onnx").exists());
}

#[test]
fn server_errors_are_reported() {
    let t = temp();
    let url = serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let o = nixa(&["voice", "models", "fetch", "--dir", t.0.to_str().unwrap()], &[("NIXA_VOICE_MODELS_URL", &url)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("404"), "{}", text(&o.stderr));
}

#[test]
fn live_nixa_folders_are_refused() {
    let t = temp();
    let live = t.0.join("Library/Application Support/OtherApp/data/voice-models");
    let foreign_cfg = t.0.join("projects/other/.config/other/voice-models");
    let home = t.0.to_str().unwrap();
    for bad in [&live, &foreign_cfg] {
        for args in [vec!["voice", "models", "--dir"], vec!["voice", "models", "fetch", "--dir"]] {
            let mut a = args.clone();
            a.push(bad.to_str().unwrap());
            let o = nixa(&a, &[("HOME", home)]);
            assert_eq!(o.status.code(), Some(1));
            assert!(text(&o.stderr).contains("belongs to another local app"), "{}", text(&o.stderr));
        }
        assert!(!bad.exists());
    }
    let ours = t.0.join("Library/Application Support/Nixa/voice-models");
    let o = nixa(&["voice", "models", "fetch", "-n", "--dir", ours.to_str().unwrap()], &[("HOME", home)]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
}

#[test]
fn help_and_usage() {
    let o = nixa(&["help", "voice"], &[]);
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o.stdout).contains("models fetch"));
    let o = nixa(&["voice"], &[]);
    assert_eq!(o.status.code(), Some(2));
    let o = nixa(&["voice", "mic", "--seconds", "0"], &[]);
    assert_eq!(o.status.code(), Some(2));
    let o = nixa(&["voice", "models", "zap"], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&nixa(&["help"], &[]).stdout).contains("voice"));
}
