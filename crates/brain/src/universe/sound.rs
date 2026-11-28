use crate::app::{App, lock};
use crate::util::now_ms;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const METEOR_MS: u64 = 1400;
pub const GAP_MS: u64 = 500;
pub const MAX_AGE_MS: u64 = 1000;
pub const AHEAD_MS: u64 = 250;
pub const SEEN_MAX: usize = 64;
pub const POLL_MS: u64 = 40;
pub const RATE: u32 = 22_050;
pub const VOLUME: &str = "0.07";
pub const AFPLAY: &str = "/usr/bin/afplay";
pub const SOUNDS_DIR: &str = "sounds";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

impl Direction {
    pub fn parse(text: &str) -> Option<Direction> {
        match text {
            "incoming" => Some(Direction::Incoming),
            "outgoing" => Some(Direction::Outgoing),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Direction::Incoming => "incoming",
            Direction::Outgoing => "outgoing",
        }
    }
}

#[derive(Default)]
pub struct Playing {
    stop: AtomicBool,
    done: AtomicBool,
}

#[derive(Default)]
pub struct Sound {
    seen: VecDeque<(Direction, u64)>,
    next_at: u64,
    playing: Option<Arc<Playing>>,
    pub played: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Quiet {
    pub nixa_on: bool,
    pub voice_on: bool,
    pub speaking: bool,
    pub user_talking: bool,
    pub holding: bool,
    pub phone: bool,
    pub quiet_time: bool,
}

pub fn allowed(q: &Quiet) -> bool {
    q.nixa_on && q.voice_on && !q.speaking && !q.user_talking && !q.holding && !q.phone && !q.quiet_time
}

pub fn quiet_of(app: &App) -> Quiet {
    let live = lock(&app.voice.live).clone();
    Quiet {
        nixa_on: crate::settings_api::nixa_on(&app.dir),
        voice_on: crate::voice::settings(app).voice_on,
        speaking: live.nixa_talking || crate::talk::speech::speaking(app),
        user_talking: live.user_talking,
        holding: live.holding,
        phone: crate::phone_door::voice_on_phone(app),
        quiet_time: lock(&app.talk.presence).quiet(now_ms()),
    }
}

pub fn pick(s: &mut Sound, direction: Direction, at: u64, now: u64, ok: bool) -> bool {
    if s.seen.contains(&(direction, at)) {
        return false;
    }
    s.seen.push_back((direction, at));
    while s.seen.len() > SEEN_MAX {
        s.seen.pop_front();
    }
    let busy = s.playing.as_ref().is_some_and(|p| !p.done.load(Ordering::SeqCst));
    if !ok || busy || at > now + AHEAD_MS || now.saturating_sub(at) > MAX_AGE_MS || now < s.next_at {
        return false;
    }
    s.next_at = now + METEOR_MS + GAP_MS;
    s.played += 1;
    true
}

pub fn meteor(app: &Arc<App>, direction: Direction, at: u64) -> bool {
    let now = now_ms();
    let ok = allowed(&quiet_of(app));
    let playing = {
        let mut s = lock(&app.universe.sound);
        if !pick(&mut s, direction, at, now, ok) {
            return false;
        }
        let playing = Arc::new(Playing::default());
        s.playing = Some(playing.clone());
        playing
    };
    let Some(file) = sound_file(&app.dir, direction) else {
        playing.done.store(true, Ordering::SeqCst);
        return false;
    };
    let left = METEOR_MS.saturating_sub(now.saturating_sub(at)).max(1);
    let app = app.clone();
    std::thread::spawn(move || play(&app, &file, &playing, left));
    true
}

fn play(app: &App, file: &Path, playing: &Playing, left_ms: u64) {
    let started = Command::new(AFPLAY).arg("-v").arg(VOLUME).arg(file).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    if let Ok(mut child) = started {
        let until = Instant::now() + Duration::from_millis(left_ms);
        loop {
            if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                break;
            }
            if playing.stop.load(Ordering::SeqCst) || Instant::now() >= until || !allowed(&quiet_of(app)) {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        }
    }
    playing.done.store(true, Ordering::SeqCst);
}

pub fn stop(app: &App) {
    if let Some(playing) = &lock(&app.universe.sound).playing {
        playing.stop.store(true, Ordering::SeqCst);
    }
}

pub fn sound_file(dir: &Path, direction: Direction) -> Option<PathBuf> {
    let folder = dir.join(SOUNDS_DIR);
    nixa_protocol::paths::ensure_private_dir(&folder).ok()?;
    let file = folder.join(format!("meteor-{}.wav", direction.word()));
    let bytes = wave(direction);
    let same = std::fs::metadata(&file).is_ok_and(|m| m.len() == bytes.len() as u64);
    if !same {
        let part = folder.join(format!("meteor-{}.part", direction.word()));
        std::fs::write(&part, &bytes).ok()?;
        std::fs::rename(&part, &file).ok()?;
    }
    Some(file)
}

pub fn wave(direction: Direction) -> Vec<u8> {
    let count = (RATE as u64 * METEOR_MS / 1000) as usize;
    let data = count * 2;
    let mut wav = Vec::with_capacity(44 + data);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data as u32).to_le_bytes());
    let incoming = direction == Direction::Incoming;
    let mut seed: u32 = if incoming { 227 } else { 1227 };
    let (mut low, mut bass) = (0.0f64, 0.0f64);
    let rate = RATE as f64;
    for i in 0..count {
        let t = i as f64 / (count - 1) as f64;
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = seed as f64 / 2_147_483_648.0 - 1.0;
        let cutoff = if incoming { 500.0 + 3200.0 * t * t } else { 400.0 + 2200.0 * (1.0 - t).powi(2) };
        low += (1.0 - (-2.0 * std::f64::consts::PI * cutoff / rate).exp()) * (noise - low);
        bass += 0.025 * (low - bass);
        let shape = if incoming { 0.25 + 0.75 * t } else { 1.0 - 0.65 * t };
        let envelope = (std::f64::consts::PI * t).sin().powi(2) * shape;
        let sample = ((low - bass) * envelope * 11_000.0).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}
