use crate::door::{Door, NowFn};
use crate::log;
use crate::wire::PlayedHow;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const CHUNK_MS: u64 = 500;
pub const SLACK_MS: u64 = 6000;
pub const MISSED_MS: u64 = 2 * 60 * 1000;
pub const MISSED_MAX: usize = 3;
pub const SYNTH_MS: u64 = 30_000;
const FIRST_PIECE: usize = 150;
const NEXT_PIECE: usize = 600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub pcm: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SayResult {
    pub interrupted: bool,
    pub ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missed {
    pub text: String,
    pub voice: String,
    pub at: u64,
}

pub type Synth = Arc<dyn Fn(&str, &str, &CancellationToken) -> Option<Pcm> + Send + Sync>;

fn split_pieces(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut push = |cur: &mut String| {
        let piece = cur.trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        cur.clear();
    };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            push(&mut cur);
            continue;
        }
        cur.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_some_and(|n| n.is_whitespace()) {
            while chars.peek().is_some_and(|n| n.is_whitespace() && *n != '\n') {
                chars.next();
            }
            push(&mut cur);
        }
    }
    push(&mut cur);
    out
}

pub fn sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for piece in split_pieces(text) {
        let limit = if out.is_empty() { FIRST_PIECE } else { NEXT_PIECE };
        if !cur.is_empty() && cur.chars().count() + piece.chars().count() > limit {
            out.push(std::mem::take(&mut cur));
        }
        if cur.is_empty() {
            cur = piece;
        } else {
            cur.push(' ');
            cur.push_str(&piece);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn u16_at(raw: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*raw.get(at)?, *raw.get(at + 1)?]))
}

fn u32_at(raw: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*raw.get(at)?, *raw.get(at + 1)?, *raw.get(at + 2)?, *raw.get(at + 3)?]))
}

pub fn wav_pcm(raw: &[u8]) -> Option<Pcm> {
    let start = raw.windows(4).position(|w| w == b"RIFF")?;
    if raw.get(start + 8..start + 12)? != b"WAVE" {
        return None;
    }
    let mut at = start + 12;
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    while at + 8 <= raw.len() {
        let id = &raw[at..at + 4];
        let size = u32_at(raw, at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= raw.len() {
            channels = u16_at(raw, body + 2)?;
            rate = u32_at(raw, body + 4)?;
            bits = u16_at(raw, body + 14)?;
        }
        if id == b"data" {
            if channels != 1 || bits != 16 || rate < 8000 {
                return None;
            }
            let end = raw.len().min(body.saturating_add(size));
            let even = end - (end - body) % 2;
            return Some(Pcm { rate, pcm: raw[body..even].to_vec() });
        }
        at = body.saturating_add(size).saturating_add(size % 2);
    }
    None
}

pub fn ptts_bin() -> PathBuf {
    if let Some(bin) = std::env::var_os("NIXA_PTTS").filter(|b| !b.is_empty()) {
        return PathBuf::from(bin);
    }
    let brew = Path::new("/opt/homebrew/bin/ptts");
    if brew.exists() { brew.to_path_buf() } else { PathBuf::from("ptts") }
}

fn run_ptts(bin: &Path, voice: Option<&str>, text: &str, stop: &CancellationToken) -> Option<Vec<u8>> {
    if stop.is_cancelled() {
        return None;
    }
    let mut cmd = Command::new(bin);
    cmd.args(["--lang", "en", "-o", "/dev/stdout"]);
    if let Some(voice) = voice {
        cmd.args(["--voice", voice]);
    }
    cmd.arg("--").arg(text).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    cmd.env("RUST_LOG", "off").env("NO_COLOR", "1");
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            log("phone-voice", &format!("ptts did not start: {e}"));
            return None;
        }
    };
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if stop.is_cancelled() || started.elapsed() > Duration::from_millis(SYNTH_MS) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let bytes = reader.join().ok()?;
    (status?.success() && !stop.is_cancelled()).then_some(bytes)
}

pub fn ptts_synth(bin: PathBuf) -> Synth {
    Arc::new(move |text, voice, stop| {
        let named = (!voice.is_empty() && voice != "cycle").then_some(voice);
        let raw = match named.and_then(|v| run_ptts(&bin, Some(v), text, stop)) {
            Some(raw) => Some(raw),
            None => run_ptts(&bin, None, text, stop),
        };
        raw.and_then(|raw| wav_pcm(&raw))
    })
}

#[derive(Clone)]
pub struct PhoneSink {
    door: Door,
    synth: Synth,
    chunk_ms: u64,
    slack_ms: u64,
    now: NowFn,
    lines: Arc<AtomicU32>,
    missed: Arc<Mutex<Vec<Missed>>>,
}

pub struct PhoneJob {
    line: u32,
    door: Door,
    stop: CancellationToken,
    result: JoinHandle<SayResult>,
}

impl PhoneJob {
    pub fn line(&self) -> u32 {
        self.line
    }

    pub fn stop(&self) {
        if self.stop.is_cancelled() {
            return;
        }
        self.stop.cancel();
        self.door.audio_stop(self.line);
    }

    pub async fn wait(&mut self) -> SayResult {
        (&mut self.result).await.unwrap_or(SayResult { interrupted: true, ms: 0 })
    }
}

fn fresh(list: &[Missed], now: u64) -> Vec<Missed> {
    list.iter().filter(|m| now.saturating_sub(m.at) <= MISSED_MS).cloned().collect()
}

impl PhoneSink {
    pub fn new(door: Door, synth: Synth) -> PhoneSink {
        PhoneSink {
            door,
            synth,
            chunk_ms: CHUNK_MS,
            slack_ms: SLACK_MS,
            now: Arc::new(crate::now_ms),
            lines: Arc::new(AtomicU32::new(0)),
            missed: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_timing(mut self, chunk_ms: u64, slack_ms: u64) -> PhoneSink {
        self.chunk_ms = chunk_ms.max(1);
        self.slack_ms = slack_ms;
        self
    }

    pub fn with_clock(mut self, now: NowFn) -> PhoneSink {
        self.now = now;
        self
    }

    fn missed(&self) -> std::sync::MutexGuard<'_, Vec<Missed>> {
        self.missed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn keep(&self, text: &str, voice: &str, line: u32) {
        let now = (self.now)();
        let mut missed = self.missed();
        let mut list = fresh(&missed, now);
        list.push(Missed { text: text.to_string(), voice: voice.to_string(), at: now });
        let extra = list.len().saturating_sub(MISSED_MAX);
        list.drain(..extra);
        *missed = list;
        log("phone-voice", &format!("no phone played line {line}, kept for later ({} kept)", missed.len()));
    }

    pub fn take_missed(&self) -> Vec<Missed> {
        let now = (self.now)();
        let mut missed = self.missed();
        let out = fresh(&missed, now);
        missed.clear();
        out
    }

    pub fn clear_missed(&self) {
        self.missed().clear();
    }

    pub fn play(&self, text: &str, voice: &str, keep_if_missed: bool) -> PhoneJob {
        let line = self.lines.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let stop = CancellationToken::new();
        let sink = self.clone();
        let token = stop.clone();
        let (text, voice) = (text.to_string(), voice.to_string());
        let result = tokio::spawn(async move { sink.run(line, text, voice, keep_if_missed, token).await });
        PhoneJob { line, door: self.door.clone(), stop, result }
    }

    async fn run(&self, line: u32, text: String, voice: String, keep: bool, stop: CancellationToken) -> SayResult {
        let started = (self.now)();
        let since = || (self.now)().saturating_sub(started);
        if self.door.audio_ready() == 0 {
            if keep {
                self.keep(&text, &voice, line);
            }
            return SayResult { interrupted: true, ms: 0 };
        }
        let (mut parts, mut sound_ms, mut begun) = (0u32, 0u64, false);
        for piece in sentences(&text) {
            if stop.is_cancelled() {
                break;
            }
            let (synth, v, t) = (self.synth.clone(), voice.clone(), stop.clone());
            let sound = tokio::task::spawn_blocking(move || synth(&piece, &v, &t)).await.ok().flatten();
            if stop.is_cancelled() {
                break;
            }
            let Some(sound) = sound.filter(|s| s.rate > 0) else {
                log("phone-voice", &format!("no sound for a sentence of line {line}"));
                continue;
            };
            if !begun {
                if !self.door.audio_start(line, sound.rate) {
                    if keep {
                        self.keep(&text, &voice, line);
                    }
                    return SayResult { interrupted: true, ms: since() };
                }
                begun = true;
            }
            let step = ((u64::from(sound.rate) * self.chunk_ms / 1000) * 2).max(2) as usize;
            for chunk in sound.pcm.chunks(step) {
                self.door.audio_chunk(line, parts, chunk);
                parts += 1;
            }
            sound_ms += (sound.pcm.len() as u64 / 2) * 1000 / u64::from(sound.rate);
        }
        if stop.is_cancelled() {
            return SayResult { interrupted: true, ms: since() };
        }
        if !begun {
            return SayResult { interrupted: false, ms: 0 };
        }
        self.door.audio_end(line, parts);
        let how = tokio::select! {
            how = self.door.audio_played(line, sound_ms + self.slack_ms) => how,
            _ = stop.cancelled() => PlayedHow::Stopped,
        };
        if how == PlayedHow::Timeout {
            log("phone-voice", &format!("the phone did not say line {line} was played"));
        }
        if keep && matches!(how, PlayedHow::Gone | PlayedHow::Timeout) {
            self.keep(&text, &voice, line);
        }
        SayResult { interrupted: how != PlayedHow::Played, ms: since() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_are_grouped_short_first_then_long() {
        assert_eq!(sentences("Hi. How are you?  Fine!"), vec!["Hi. How are you? Fine!"]);
        assert_eq!(sentences("one\n\ntwo"), vec!["one two"]);
        let long = "This sentence is about forty characters. ".repeat(6);
        let out = sentences(&long);
        assert!(out[0].chars().count() <= FIRST_PIECE);
        assert!(out.len() >= 2);
        assert!(out.iter().skip(1).all(|p| p.chars().count() <= NEXT_PIECE));
        assert_eq!(out.join(" "), long.trim());
        assert!(sentences("   ").is_empty());
        assert_eq!(sentences("v1.2 is out"), vec!["v1.2 is out"]);
    }

    fn wav(rate: u32, channels: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut out = b"junkRIFF".to_vec();
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn wav_files_give_mono_pcm_only() {
        let pcm = wav_pcm(&wav(24_000, 1, 16, &[1, 2, 3, 4, 5])).unwrap();
        assert_eq!(pcm, Pcm { rate: 24_000, pcm: vec![1, 2, 3, 4] });
        assert!(wav_pcm(&wav(24_000, 2, 16, &[1, 2])).is_none());
        assert!(wav_pcm(&wav(24_000, 1, 8, &[1, 2])).is_none());
        assert!(wav_pcm(&wav(4_000, 1, 16, &[1, 2])).is_none());
        assert!(wav_pcm(b"RIFF").is_none());
        assert!(wav_pcm(b"nothing here").is_none());
        let mut huge = wav(16_000, 1, 16, &[9, 9]);
        let at = huge.len() - 6;
        huge[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(wav_pcm(&huge).unwrap().pcm, vec![9, 9]);
    }

    #[test]
    fn ptts_bin_follows_the_env() {
        assert!(!ptts_bin().as_os_str().is_empty());
    }

    #[test]
    fn missing_ptts_gives_no_sound() {
        let synth = ptts_synth(PathBuf::from("/nonexistent/ptts"));
        assert!(synth("hello", "Toby", &CancellationToken::new()).is_none());
    }
}
