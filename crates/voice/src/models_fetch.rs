use crate::listen::ListenConfig;
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, RANGE};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const MODELS_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";
pub const MODELS_URL_ENV: &str = "NIXA_VOICE_MODELS_URL";
const VAD_FILE: &str = "silero_vad.onnx";
const PARAKEET_DIR: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
const WORK: &str = ".download";
const LOCK: &str = "lock";
const CHUNK: usize = 1 << 16;
const STEP_BYTES: u64 = 16 << 20;
const LOCK_TRIES: u32 = 20;
const LOCK_WAIT: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub also: Vec<(u64, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unpack {
    Plain,
    TarBz2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub name: String,
    pub what: String,
    pub download: String,
    pub size: u64,
    pub sha256: String,
    pub unpack: Unpack,
    pub files: Vec<ModelFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileState {
    Ok,
    Missing,
    Size { found: u64 },
    Hash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Ready { name: String },
    Start { name: String, size: u64, from: u64 },
    Bytes { name: String, done: u64, size: u64 },
    Unpack { name: String },
    Check { name: String },
    Done { name: String },
}

fn file(path: &str, size: u64, sha256: &str) -> ModelFile {
    ModelFile { path: path.into(), size, sha256: sha256.into(), also: Vec::new() }
}

fn older(mut f: ModelFile, size: u64, sha256: &str) -> ModelFile {
    f.also.push((size, sha256.into()));
    f
}

fn versions(f: &ModelFile) -> Vec<(u64, &str)> {
    let mut all = vec![(f.size, f.sha256.as_str())];
    all.extend(f.also.iter().map(|(s, h)| (*s, h.as_str())));
    all
}

pub fn models() -> Vec<Model> {
    let p = |f: &str| format!("{PARAKEET_DIR}/{f}");
    vec![
        Model {
            name: "vad".into(),
            what: "voice activity (Silero VAD)".into(),
            download: VAD_FILE.into(),
            size: 643_854,
            sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6".into(),
            unpack: Unpack::Plain,
            files: vec![file(VAD_FILE, 643_854, "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6")],
        },
        Model {
            name: "stt".into(),
            what: "speech to text (Parakeet TDT 0.6b v3, int8)".into(),
            download: format!("{PARAKEET_DIR}.tar.bz2"),
            size: 487_211_443,
            sha256: "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9".into(),
            unpack: Unpack::TarBz2,
            files: vec![
                older(
                    file(&p("encoder.int8.onnx"), 652_282_294, "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479"),
                    652_184_281,
                    "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
                ),
                older(
                    file(&p("decoder.int8.onnx"), 11_845_274, "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5"),
                    11_845_275,
                    "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
                ),
                older(
                    file(&p("joiner.int8.onnx"), 6_355_277, "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98"),
                    6_355_277,
                    "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
                ),
                file(&p("tokens.txt"), 93_939, "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d"),
            ],
        },
    ]
}

pub fn models_dir() -> PathBuf {
    ListenConfig::default().models_dir
}

pub fn models_url() -> String {
    url_or_default(std::env::var(MODELS_URL_ENV).ok().as_deref())
}

pub fn url_or_default(given: Option<&str>) -> String {
    let Some(url) = given.map(|u| u.trim().trim_end_matches('/')).filter(|u| !u.is_empty()) else {
        return MODELS_URL.to_string();
    };
    let https = url.len() > 8 && url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://"));
    if !https {
        eprintln!("nixa-voice: {MODELS_URL_ENV} must start with https://, so the default models address is used");
        return MODELS_URL.to_string();
    }
    url.to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path, hasher: &mut Sha256) -> Result<u64> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            return Ok(total);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
}

fn file_state(path: &Path, want: &ModelFile, hash: bool) -> FileState {
    let found = match fs::metadata(path) {
        Ok(m) if m.is_file() => m.len(),
        _ => return FileState::Missing,
    };
    let known = versions(want);
    if !known.iter().any(|(size, _)| *size == found) {
        return FileState::Size { found };
    }
    if !hash {
        return FileState::Ok;
    }
    let mut hasher = Sha256::new();
    match hash_file(path, &mut hasher) {
        Ok(_) => {
            let got = hex(&hasher.finalize());
            if known.iter().any(|(size, sha)| *size == found && *sha == got) { FileState::Ok } else { FileState::Hash }
        }
        Err(_) => FileState::Missing,
    }
}

pub fn check(dir: &Path, model: &Model, hash: bool) -> Vec<(ModelFile, FileState)> {
    model.files.iter().map(|f| (f.clone(), file_state(&dir.join(&f.path), f, hash))).collect()
}

pub fn missing_models(dir: &Path, models: &[Model], hash: bool) -> Vec<Model> {
    models
        .iter()
        .filter(|m| check(dir, m, hash).iter().any(|(_, s)| *s != FileState::Ok))
        .cloned()
        .collect()
}

fn safe_relative(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

fn check_list(models: &[Model]) -> Result<()> {
    for m in models {
        if !safe_relative(&m.download) || m.download.contains('/') {
            bail!("model {} has a bad download name {:?}", m.name, m.download);
        }
        for f in &m.files {
            if !safe_relative(&f.path) {
                bail!("model {} has a bad file path {:?}", m.name, f.path);
            }
        }
        if m.unpack == Unpack::Plain && (m.files.len() != 1 || m.files[0].size != m.size || m.files[0].sha256 != m.sha256) {
            bail!("model {} is one file, its size and hash must match the download", m.name);
        }
    }
    Ok(())
}

struct Lock {
    _file: File,
}

fn lock(work: &Path) -> Result<Lock> {
    let path = work.join(LOCK);
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(&path).with_context(|| format!("open {}", path.display()))?;
    for _ in 0..LOCK_TRIES {
        match f.try_lock() {
            Ok(()) => return Ok(Lock { _file: f }),
            Err(fs::TryLockError::WouldBlock) => std::thread::sleep(LOCK_WAIT),
            Err(fs::TryLockError::Error(e)) => return Err(e).with_context(|| format!("lock {}", path.display())),
        }
    }
    bail!("another nixa is fetching voice models into this folder")
}

fn client() -> Result<Client> {
    Client::builder()
        .user_agent(concat!("nixa/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(None)
        .redirect(reqwest::redirect::Policy::custom(|step| {
            if step.url().scheme() != "https" {
                step.error("a voice model download may only move to an https address")
            } else if step.previous().len() >= 10 {
                step.error("too many redirects")
            } else {
                step.follow()
            }
        }))
        .build()
        .context("start the download client")
}

fn download(client: &Client, url: &str, part: &Path, model: &Model, on: &mut dyn FnMut(&Step)) -> Result<()> {
    let mut have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have > model.size {
        let _ = fs::remove_file(part);
        have = 0;
    }
    let mut hasher = Sha256::new();
    if have > 0 {
        have = hash_file(part, &mut hasher)?;
    }
    if have == model.size {
        if hex(&hasher.clone().finalize()) == model.sha256 {
            on(&Step::Check { name: model.name.clone() });
            return Ok(());
        }
        let _ = fs::remove_file(part);
        have = 0;
        hasher = Sha256::new();
    }
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(RANGE, format!("bytes={have}-"));
    }
    let mut resp = req.send().with_context(|| format!("download {url}"))?;
    let status = resp.status();
    let length = resp.headers().get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mut done = if have > 0 && status == StatusCode::PARTIAL_CONTENT {
        have
    } else if status == StatusCode::OK {
        hasher = Sha256::new();
        0
    } else {
        bail!("download {url}: HTTP {status}");
    };
    if let Some(len) = length
        && done + len != model.size
    {
        bail!("download {url}: the server sends {} bytes, {} expects {}", done + len, model.name, model.size);
    }
    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .append(done > 0)
        .truncate(done == 0)
        .open(part)
        .with_context(|| format!("write {}", part.display()))?;
    on(&Step::Start { name: model.name.clone(), size: model.size, from: done });
    let mut buf = vec![0u8; CHUNK];
    let mut said = done;
    loop {
        let n = resp.read(&mut buf).with_context(|| format!("download {url} at {done} bytes"))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > model.size {
            drop(out);
            let _ = fs::remove_file(part);
            bail!("download {url}: more than the {} bytes expected", model.size);
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).with_context(|| format!("write {}", part.display()))?;
        if done - said >= STEP_BYTES {
            said = done;
            on(&Step::Bytes { name: model.name.clone(), done, size: model.size });
        }
    }
    out.sync_all().with_context(|| format!("write {}", part.display()))?;
    if done != model.size {
        bail!("download {url} stopped at {done} of {} bytes; run fetch again to go on", model.size);
    }
    on(&Step::Bytes { name: model.name.clone(), done, size: model.size });
    on(&Step::Check { name: model.name.clone() });
    let got = hex(&hasher.finalize());
    if got != model.sha256 {
        let _ = fs::remove_file(part);
        bail!("download {url}: hash {got} does not match {}", model.sha256);
    }
    Ok(())
}

fn unpack_tar(part: &Path, into: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(into);
    fs::create_dir_all(into).with_context(|| format!("make {}", into.display()))?;
    let out = Command::new("/usr/bin/tar")
        .arg("-xjf")
        .arg(part)
        .arg("-C")
        .arg(into)
        .output()
        .context("run /usr/bin/tar")?;
    if !out.status.success() {
        bail!("unpack {}: {}", part.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn tops(model: &Model) -> Vec<String> {
    let mut tops: Vec<String> = Vec::new();
    for f in &model.files {
        let top = f.path.split('/').next().unwrap_or_default().to_string();
        if !tops.contains(&top) {
            tops.push(top);
        }
    }
    tops
}

fn place(from: &Path, to: &Path, old: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(old);
    let _ = fs::remove_file(old);
    let had = match fs::rename(to, old) {
        Ok(()) => true,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(e).with_context(|| format!("move the old {} away", to.display())),
    };
    if let Err(e) = fs::rename(from, to) {
        if had {
            let _ = fs::rename(old, to);
        }
        return Err(e).with_context(|| format!("put {} in place", to.display()));
    }
    let _ = fs::remove_dir_all(old);
    let _ = fs::remove_file(old);
    Ok(())
}

fn install(dir: &Path, work: &Path, part: &Path, model: &Model, on: &mut dyn FnMut(&Step)) -> Result<()> {
    match model.unpack {
        Unpack::Plain => {
            let target = dir.join(&model.files[0].path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
            }
            place(part, &target, &work.join(format!("old-{}", model.name)))
        }
        Unpack::TarBz2 => {
            on(&Step::Unpack { name: model.name.clone() });
            let into = work.join(format!("unpack-{}", model.name));
            unpack_tar(part, &into)?;
            on(&Step::Check { name: model.name.clone() });
            for (f, state) in check(&into, model, true) {
                if state != FileState::Ok {
                    let _ = fs::remove_dir_all(&into);
                    bail!("{} in the {} download is {state:?}", f.path, model.name);
                }
            }
            for top in tops(model) {
                place(&into.join(&top), &dir.join(&top), &work.join(format!("old-{}", model.name)))?;
            }
            let _ = fs::remove_dir_all(&into);
            let _ = fs::remove_file(part);
            Ok(())
        }
    }
}

pub fn fetch(dir: &Path, models: &[Model], base_url: &str, on: &mut dyn FnMut(&Step)) -> Result<Vec<String>> {
    check_list(models)?;
    let work = dir.join(WORK);
    fs::create_dir_all(&work).with_context(|| format!("make {}", work.display()))?;
    let _lock = lock(&work)?;
    let client = client()?;
    let base = base_url.trim_end_matches('/');
    let mut fetched = Vec::new();
    for model in models {
        if check(dir, model, true).iter().all(|(_, s)| *s == FileState::Ok) {
            on(&Step::Ready { name: model.name.clone() });
            continue;
        }
        let part = work.join(format!("{}.part", model.download));
        download(&client, &format!("{base}/{}", model.download), &part, model, on)?;
        install(dir, &work, &part, model, on)?;
        for (f, state) in check(dir, model, false) {
            if state != FileState::Ok {
                bail!("{} is {state:?} after the install", f.path);
            }
        }
        on(&Step::Done { name: model.name.clone() });
        fetched.push(model.name.clone());
    }
    Ok(fetched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;
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
        let dir = std::env::temp_dir().join(format!("nixa-models-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn plain(name: &str, bytes: &[u8]) -> Model {
        Model {
            name: name.into(),
            what: "test".into(),
            download: format!("{name}.onnx"),
            size: bytes.len() as u64,
            sha256: sha(bytes),
            unpack: Unpack::Plain,
            files: vec![file(&format!("{name}.onnx"), bytes.len() as u64, &sha(bytes))],
        }
    }

    fn serve(files: Vec<(String, Vec<u8>)>, cut_first_at: Option<usize>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let n = count.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                let _ = reader.read_line(&mut first);
                let mut from = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_lowercase();
                    if let Some(v) = lower.strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap_or(0);
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                let Some((_, body)) = files.iter().find(|(p, _)| *p == path) else {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                };
                let part = &body[from.min(body.len())..];
                let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", part.len());
                let _ = stream.write_all(head.as_bytes());
                let send = match cut_first_at {
                    Some(cut) if n == 0 => &part[..cut.min(part.len())],
                    _ => part,
                };
                let _ = stream.write_all(send);
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn noop() -> impl FnMut(&Step) {
        |_| {}
    }

    #[test]
    fn pinned_list_matches_the_listener_names() {
        let list = models();
        check_list(&list).unwrap();
        let config = ListenConfig::default();
        assert_eq!(list[0].files[0].path, config.vad_model);
        assert!(list[1].files.iter().all(|f| f.path.starts_with(&format!("{}/", config.parakeet))));
        assert_eq!(tops(&list[1]), vec![config.parakeet.clone()]);
    }

    #[test]
    fn states_show_missing_size_and_hash() {
        let t = temp();
        let m = plain("a", b"hello");
        assert_eq!(check(&t.0, &m, true)[0].1, FileState::Missing);
        fs::write(t.0.join("a.onnx"), b"hi").unwrap();
        assert_eq!(check(&t.0, &m, true)[0].1, FileState::Size { found: 2 });
        fs::write(t.0.join("a.onnx"), b"jello").unwrap();
        assert_eq!(check(&t.0, &m, false)[0].1, FileState::Ok);
        assert_eq!(check(&t.0, &m, true)[0].1, FileState::Hash);
        fs::write(t.0.join("a.onnx"), b"hello").unwrap();
        assert_eq!(check(&t.0, &m, true)[0].1, FileState::Ok);
        assert!(missing_models(&t.0, &[m], true).is_empty());
    }

    #[test]
    fn fetches_a_plain_file_and_skips_it_next_time() {
        let t = temp();
        let body = b"silero test model bytes".to_vec();
        let m = plain("vad", &body);
        let (url, hits) = serve(vec![("vad.onnx".into(), body.clone())], None);
        let mut steps = Vec::new();
        let got = fetch(&t.0, std::slice::from_ref(&m), &url, &mut |s| steps.push(s.clone())).unwrap();
        assert!(!steps.iter().any(|s| matches!(s, Step::Ready { .. })));
        assert_eq!(got, vec!["vad".to_string()]);
        assert_eq!(fs::read(t.0.join("vad.onnx")).unwrap(), body);
        assert!(steps.contains(&Step::Done { name: "vad".into() }));
        let again = fetch(&t.0, &[m], &url, &mut noop()).unwrap();
        assert!(again.is_empty());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wrong_hash_is_refused_and_nothing_is_placed() {
        let t = temp();
        let mut m = plain("vad", b"good bytes");
        let (url, _) = serve(vec![("vad.onnx".into(), b"evil bytes".to_vec())], None);
        let err = fetch(&t.0, std::slice::from_ref(&m), &url, &mut noop()).unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");
        assert!(!t.0.join("vad.onnx").exists());
        assert!(!t.0.join(WORK).join("vad.onnx.part").exists());
        m.size += 1;
        m.files[0].size += 1;
        let err = fetch(&t.0, &[m], &url, &mut noop()).unwrap_err().to_string();
        assert!(err.contains("expects"), "{err}");
    }

    #[test]
    fn a_cut_download_goes_on_from_where_it_stopped() {
        let t = temp();
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let m = plain("vad", &body);
        let (url, hits) = serve(vec![("vad.onnx".into(), body.clone())], Some(100_000));
        let err = fetch(&t.0, std::slice::from_ref(&m), &url, &mut noop()).unwrap_err().to_string();
        assert!(err.contains("stopped at") || err.contains("download"), "{err}");
        let part = t.0.join(WORK).join("vad.onnx.part");
        assert_eq!(fs::metadata(&part).unwrap().len(), 100_000);
        let mut starts = Vec::new();
        fetch(&t.0, &[m], &url, &mut |s| {
            if let Step::Start { from, .. } = s {
                starts.push(*from);
            }
        })
        .unwrap();
        assert_eq!(starts, vec![100_000]);
        assert_eq!(fs::read(t.0.join("vad.onnx")).unwrap(), body);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unpacks_a_tar_and_checks_every_file() {
        let t = temp();
        let src = t.0.join("src");
        fs::create_dir_all(src.join("pack/test_wavs")).unwrap();
        fs::write(src.join("pack/tokens.txt"), b"a 1\nb 2\n").unwrap();
        fs::write(src.join("pack/encoder.onnx"), b"encoder").unwrap();
        fs::write(src.join("pack/test_wavs/en.wav"), b"wav").unwrap();
        let tar = t.0.join("pack.tar.bz2");
        let ok = Command::new("/usr/bin/tar").arg("-cjf").arg(&tar).arg("-C").arg(&src).arg("pack").status().unwrap();
        assert!(ok.success());
        let bytes = fs::read(&tar).unwrap();
        let dir = t.0.join("models");
        fs::create_dir_all(dir.join("pack")).unwrap();
        fs::write(dir.join("pack/stale.txt"), b"old").unwrap();
        let mut m = Model {
            name: "stt".into(),
            what: "test".into(),
            download: "pack.tar.bz2".into(),
            size: bytes.len() as u64,
            sha256: sha(&bytes),
            unpack: Unpack::TarBz2,
            files: vec![file("pack/tokens.txt", 8, &sha(b"a 1\nb 2\n")), file("pack/encoder.onnx", 7, &sha(b"encoder"))],
        };
        let (url, _) = serve(vec![("pack.tar.bz2".into(), bytes.clone())], None);
        let mut steps = Vec::new();
        fetch(&dir, std::slice::from_ref(&m), &url, &mut |s| steps.push(s.clone())).unwrap();
        assert!(steps.contains(&Step::Unpack { name: "stt".into() }));
        assert_eq!(fs::read(dir.join("pack/encoder.onnx")).unwrap(), b"encoder");
        assert!(dir.join("pack/test_wavs/en.wav").exists());
        assert!(!dir.join("pack/stale.txt").exists());
        assert!(!dir.join(WORK).join("unpack-stt").exists());
        assert!(!dir.join(WORK).join("pack.tar.bz2.part").exists());
        fs::remove_file(dir.join("pack/encoder.onnx")).unwrap();
        m.files[1].sha256 = sha(b"other");
        let err = fetch(&dir, &[m], &url, &mut noop()).unwrap_err().to_string();
        assert!(err.contains("encoder.onnx"), "{err}");
        assert!(!dir.join("pack/encoder.onnx").exists());
    }

    #[test]
    fn an_older_known_copy_counts_as_ready() {
        let t = temp();
        let m = Model { files: vec![older(file("a.onnx", 5, &sha(b"hello")), 3, &sha(b"old"))], ..plain("a", b"hello") };
        fs::write(t.0.join("a.onnx"), b"old").unwrap();
        assert_eq!(check(&t.0, &m, true)[0].1, FileState::Ok);
        fs::write(t.0.join("a.onnx"), b"odd").unwrap();
        assert_eq!(check(&t.0, &m, true)[0].1, FileState::Hash);
        fs::write(t.0.join("a.onnx"), b"four").unwrap();
        assert_eq!(check(&t.0, &m, false)[0].1, FileState::Size { found: 4 });
    }

    #[test]
    fn a_whole_part_left_over_is_used_without_a_new_download() {
        let t = temp();
        let body = b"complete part".to_vec();
        let m = plain("vad", &body);
        fs::create_dir_all(t.0.join(WORK)).unwrap();
        fs::write(t.0.join(WORK).join("vad.onnx.part"), &body).unwrap();
        let (url, hits) = serve(vec![("vad.onnx".into(), body.clone())], None);
        fetch(&t.0, &[m], &url, &mut noop()).unwrap();
        assert_eq!(fs::read(t.0.join("vad.onnx")).unwrap(), body);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn bad_paths_are_refused() {
        let t = temp();
        let mut m = plain("a", b"x");
        m.files[0].path = "../evil.onnx".into();
        assert!(fetch(&t.0, &[m.clone()], "http://127.0.0.1:9", &mut noop()).is_err());
        m.files[0].path = "/etc/evil".into();
        assert!(check_list(&[m.clone()]).is_err());
        m.files[0].path = "a.onnx".into();
        m.download = "sub/a.onnx".into();
        assert!(check_list(&[m]).is_err());
    }

    #[test]
    fn a_second_fetch_waits_for_the_lock() {
        let t = temp();
        let work = t.0.join(WORK);
        fs::create_dir_all(&work).unwrap();
        let held = lock(&work).unwrap();
        let err = fetch(&t.0, &[plain("a", b"x")], "http://127.0.0.1:9", &mut noop()).unwrap_err().to_string();
        assert!(err.contains("another nixa"), "{err}");
        drop(held);
    }

    #[test]
    fn models_url_comes_from_the_env_or_the_default() {
        assert!(models_url().starts_with("https://"));
        assert!(MODELS_URL.starts_with("https://"));
        assert_eq!(url_or_default(None), MODELS_URL);
        assert_eq!(url_or_default(Some("  ")), MODELS_URL);
        assert_eq!(url_or_default(Some("http://mirror.example/models")), MODELS_URL);
        assert_eq!(url_or_default(Some("file:///tmp/models")), MODELS_URL);
        assert_eq!(url_or_default(Some("https://")), MODELS_URL);
        assert_eq!(url_or_default(Some("https://mirror.example/models/")), "https://mirror.example/models");
    }

}
