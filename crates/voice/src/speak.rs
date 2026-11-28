use crate::devices::find_device;
use crate::own_voice::OwnVoice;
use crate::pcm::{Resampler, to_mono};
use crate::scrub::scrub_for_speech;
use crate::wav::parse_wav;
use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GAP_MS: u64 = 150;
const POLL: Duration = Duration::from_millis(20);
const TAIL_PAD_MS: u64 = 300;
const FIRST_CHUNK: usize = 150;
const NEXT_CHUNK: usize = 600;
const RESERVED: &[&str] = &["stop", "voices", "cycle", "-h"];

pub const CYCLE_VOICES: [&str; 25] = [
    "Zoey", "Juniper", "Everly", "Harper", "Hadley", "Delia", "Easton", "Micah", "Trevor", "Russell", "Preston", "Miles",
    "Maeve", "Freya", "Pippa", "Elodie-Rose", "Imogen", "Lottie", "Alfie", "Hugh", "Edwin", "Spencer", "Toby", "Teddy",
    "Bryony",
];

pub const AGENT_VOICES: [(&str, &str); 9] = [
    ("nixa", "Pippa"),
    ("claude", "Harper"),
    ("codex", "Miles"),
    ("cursor", "Hadley"),
    ("agy", "Preston"),
    ("omp", "Edwin"),
    ("opencode", "Russell"),
    ("gemini", "Easton"),
    ("unknown", "Delia"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SayResult {
    pub interrupted: bool,
    pub ms: u64,
}

const INTERRUPTED: SayResult = SayResult { interrupted: true, ms: 0 };

#[derive(Clone)]
pub struct SpeechHandle(Arc<AtomicBool>);

impl SpeechHandle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
        if let Some(job) = lock().remove(self) {
            let _ = job.done.send(INTERRUPTED);
        }
    }
}

struct Job {
    text: String,
    voice: String,
    handle: SpeechHandle,
    done: Sender<SayResult>,
    on_failure: Option<Box<dyn FnOnce() + Send>>,
    on_part: Option<PartHook>,
}

pub type PartHook = Box<dyn FnMut(&str, bool) + Send>;

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    playing: Option<SpeechHandle>,
    device: Option<String>,
    last_end_ms: u64,
    own: OwnVoice,
}

impl State {
    fn remove(&mut self, handle: &SpeechHandle) -> Option<Job> {
        let index = self.queue.iter().position(|job| Arc::ptr_eq(&job.handle.0, &handle.0))?;
        self.queue.remove(index)
    }
}

struct Speaker {
    state: Mutex<State>,
    wake: Condvar,
}

static CYCLE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn speaker() -> &'static Speaker {
    static SPEAKER: OnceLock<Speaker> = OnceLock::new();
    static STARTED: OnceLock<()> = OnceLock::new();
    let s = SPEAKER.get_or_init(|| Speaker { state: Mutex::new(State::default()), wake: Condvar::new() });
    STARTED.get_or_init(|| {
        thread::Builder::new().name("nixa-speak".into()).spawn(worker).expect("speech thread");
    });
    s
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    speaker().state.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn voice_for(agent: &str) -> &'static str {
    let agent = agent.trim().to_lowercase();
    AGENT_VOICES
        .iter()
        .find(|(a, _)| *a == agent)
        .or_else(|| AGENT_VOICES.iter().find(|(a, _)| *a == "unknown"))
        .map(|(_, v)| *v)
        .unwrap_or("Pippa")
}

pub fn chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut sentences = Vec::new();
    for line in text.split('\n') {
        let mut start = 0;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for (k, (i, c)) in chars.iter().enumerate() {
            if matches!(c, '.' | '!' | '?') && chars.get(k + 1).is_some_and(|(_, n)| n.is_whitespace()) {
                sentences.push(line[start..i + c.len_utf8()].to_string());
                start = i + c.len_utf8();
            }
        }
        sentences.push(line[start..].to_string());
    }
    for s in sentences.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let limit = if out.is_empty() { FIRST_CHUNK } else { NEXT_CHUNK };
        if !cur.is_empty() && cur.len() + s.len() > limit {
            out.push(std::mem::take(&mut cur));
        }
        if cur.is_empty() {
            cur = s.to_string();
        } else {
            cur = format!("{cur} {s}");
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn tool_path() -> String {
    let base = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    format!("/opt/homebrew/bin:/usr/local/bin:{base}")
}

fn ptts_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NIXA_PTTS")
        && Path::new(&p).exists()
    {
        return Some(PathBuf::from(p));
    }
    tool_path().split(':').map(|d| Path::new(d).join("ptts")).find(|p| p.exists())
}

pub fn list_voices() -> Vec<String> {
    let listed = ptts_bin().and_then(|bin| {
        Command::new(bin)
            .args(["--list-voices", "--lang", "en"])
            .env("PATH", tool_path())
            .env("HF_HUB_OFFLINE", "1")
            .stdin(Stdio::null())
            .output()
            .ok()
    });
    let names: Vec<String> = listed
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let cols: Vec<&str> = l.split('\t').collect();
                    (cols.len() > 1 && cols[1] == "en").then(|| cols[0].trim().to_string())
                })
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() { CYCLE_VOICES.iter().map(|s| s.to_string()).collect() } else { names }
}

pub fn set_output_device(name: Option<String>) {
    lock().device = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
}

pub fn output_device() -> Option<String> {
    lock().device.clone()
}

pub fn speak(text: &str, voice: &str) -> Receiver<SayResult> {
    speak_cancellable(text, voice).1
}

pub fn speak_with_fallback(text: &str, voice: &str, on_failure: impl FnOnce() + Send + 'static) -> Receiver<SayResult> {
    enqueue(text, voice, Some(Box::new(on_failure)), None).1
}

pub fn speak_cancellable(text: &str, voice: &str) -> (SpeechHandle, Receiver<SayResult>) {
    enqueue(text, voice, None, None)
}

pub fn speak_with_parts(text: &str, voice: &str, on_part: impl FnMut(&str, bool) + Send + 'static) -> (SpeechHandle, Receiver<SayResult>) {
    enqueue(text, voice, None, Some(Box::new(on_part)))
}

fn enqueue(text: &str, voice: &str, on_failure: Option<Box<dyn FnOnce() + Send>>, on_part: Option<PartHook>) -> (SpeechHandle, Receiver<SayResult>) {
    let (done, result) = channel();
    let mut voice = voice.trim().to_string();
    if voice.is_empty() {
        voice = voice_for("nixa").to_string();
    }
    if voice == "cycle" {
        voice = CYCLE_VOICES[CYCLE.fetch_add(1, Ordering::SeqCst) % CYCLE_VOICES.len()].to_string();
    }
    let s = speaker();
    let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
    let handle = SpeechHandle(Arc::new(AtomicBool::new(false)));
    st.queue.push_back(Job { text: text.to_string(), voice, handle: handle.clone(), done, on_failure, on_part });
    s.wake.notify_all();
    (handle, result)
}

pub fn speak_wait(text: &str, voice: &str) -> SayResult {
    speak(text, voice).recv().unwrap_or(INTERRUPTED)
}

pub fn stop_speaking() {
    let s = speaker();
    let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = &st.playing {
        handle.0.store(true, Ordering::Relaxed);
    }
    let dropped = std::mem::take(&mut st.queue);
    drop(st);
    for job in dropped {
        let _ = job.done.send(INTERRUPTED);
    }
    s.wake.notify_all();
}

pub fn is_speaking() -> bool {
    let st = lock();
    st.playing.is_some() || !st.queue.is_empty()
}

pub(crate) fn last_spoke_ms() -> u64 {
    lock().last_end_ms
}

pub(crate) fn heard_own_voice(text: &str) -> bool {
    lock().own.is_own(text, now_ms())
}

pub(crate) fn heard_own_echo(text: &str) -> bool {
    lock().own.is_echo(text, now_ms())
}

fn worker() {
    let s = speaker();
    loop {
        let job = {
            let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = st.queue.pop_front() {
                    st.playing = Some(job.handle.clone());
                    break job;
                }
                st = s.wake.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        let mut job = job;
        let (result, failed) = run_job(&mut job);
        let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
        st.playing = None;
        st.last_end_ms = now_ms();
        drop(st);
        if failed && !stopped(&job.handle.0) && let Some(on_failure) = job.on_failure {
            on_failure();
        }
        let _ = job.done.send(result);
    }
}

fn safe_text(text: &str) -> String {
    let t = text.trim();
    if RESERVED.contains(&t) || (t.starts_with("--") && t.len() > 2 && t[2..].chars().all(|c| c.is_alphanumeric() || c == '_')) {
        format!("{t}.")
    } else {
        t.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Playback {
    Played,
    Interrupted,
    Failed,
}

fn run_job(job: &mut Job) -> (SayResult, bool) {
    let started = Instant::now();
    let done = |playback| (
        SayResult { interrupted: playback == Playback::Interrupted, ms: started.elapsed().as_millis() as u64 },
        playback == Playback::Failed,
    );
    if stopped(&job.handle.0) {
        return done(Playback::Interrupted);
    }
    let text = scrub_for_speech(&job.text);
    if text.chars().count() < 2 {
        return (SayResult::default(), false);
    }
    let last = last_spoke_ms();
    let since = now_ms().saturating_sub(last);
    if last > 0 && since < GAP_MS {
        thread::sleep(Duration::from_millis(GAP_MS - since));
    }
    if stopped(&job.handle.0) {
        return done(Playback::Interrupted);
    }
    lock().own.remember(&text, now_ms());
    let device = output_device();
    let mut quiet = |_: &str, _: bool| {};
    let hook: &mut dyn FnMut(&str, bool) = match job.on_part.as_mut() {
        Some(h) => h.as_mut(),
        None => &mut quiet,
    };
    done(play_text(&safe_text(&text), &job.voice, device.as_deref(), &job.handle.0, hook))
}

fn stopped(cancelled: &AtomicBool) -> bool {
    cancelled.load(Ordering::Relaxed)
}

fn wait_child(child: &mut Child, cancelled: &AtomicBool) -> Option<bool> {
    loop {
        if stopped(cancelled) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) => thread::sleep(POLL),
            Err(_) => return Some(false),
        }
    }
}

fn spawn_ptts(bin: &Path, voice: &str, part: &str, wav: &Path) -> Option<Child> {
    Command::new(bin)
        .args(["--lang", "en", "--voice", voice, "-o"])
        .arg(wav)
        .arg("--")
        .arg(part)
        .env("PATH", tool_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn temp_dir() -> PathBuf {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let n = COUNT.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("nixa-speak-{}-{n}", std::process::id()))
}

fn said(text: &str, voice: &str, device: Option<&str>, cancelled: &AtomicBool, hook: &mut dyn FnMut(&str, bool)) -> Playback {
    hook(text, true);
    let outcome = say(text, voice, device, cancelled);
    hook(text, false);
    outcome
}

fn play_text(text: &str, voice: &str, device: Option<&str>, cancelled: &AtomicBool, hook: &mut dyn FnMut(&str, bool)) -> Playback {
    let parts = chunks(text);
    if parts.is_empty() {
        return Playback::Played;
    }
    let Some(bin) = ptts_bin() else {
        return said(&parts.join(" "), voice, device, cancelled, hook);
    };
    let dir = temp_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return said(&parts.join(" "), voice, device, cancelled, hook);
    }
    let wav_at = |i: usize| dir.join(format!("{i}.wav"));
    let mut next = spawn_ptts(&bin, voice, &parts[0], &wav_at(0));
    let mut outcome = Playback::Played;
    for i in 0..parts.len() {
        let made = match next.take() {
            Some(mut child) => wait_child(&mut child, cancelled),
            None => Some(false),
        };
        let Some(made) = made else {
            outcome = Playback::Interrupted;
            break;
        };
        if i + 1 < parts.len() {
            next = spawn_ptts(&bin, voice, &parts[i + 1], &wav_at(i + 1));
        }
        if made {
            hook(&parts[i], true);
        }
        let played = made && match play_file(&wav_at(i), device, cancelled) {
            Ok(stop) => {
                if stop {
                    outcome = Playback::Interrupted;
                }
                true
            }
            Err(_) => device.is_none() && afplay(&wav_at(i), cancelled).is_some_and(|ok| ok),
        };
        if made {
            hook(&parts[i], false);
        }
        if outcome == Playback::Interrupted || stopped(cancelled) {
            outcome = Playback::Interrupted;
            break;
        }
        if !played {
            outcome = said(&parts[i..].join(" "), voice, device, cancelled, hook);
            break;
        }
    }
    if let Some(mut child) = next {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&dir);
    outcome
}

fn afplay(wav: &Path, cancelled: &AtomicBool) -> Option<bool> {
    let mut child = Command::new("/usr/bin/afplay").arg(wav).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    wait_child(&mut child, cancelled)
}

fn say_once(text: &str, voice: Option<&str>, device: Option<&str>, cancelled: &AtomicBool) -> Option<bool> {
    let mut cmd = Command::new("/usr/bin/say");
    if let Some(d) = device {
        cmd.args(["-a", d]);
    }
    if let Some(v) = voice {
        cmd.args(["-v", v]);
    }
    let Ok(mut child) = cmd.args(["-f", "-"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return Some(false);
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    wait_child(&mut child, cancelled)
}

fn say(text: &str, voice: &str, device: Option<&str>, cancelled: &AtomicBool) -> Playback {
    let result = match say_once(text, Some(voice), device, cancelled) {
        Some(false) if !stopped(cancelled) => say_once(text, None, device, cancelled),
        result => result,
    };
    playback_result(result, stopped(cancelled))
}

fn playback_result(result: Option<bool>, cancelled: bool) -> Playback {
    match (result, cancelled) {
        (_, true) | (None, _) => Playback::Interrupted,
        (Some(true), _) => Playback::Played,
        (Some(false), _) => Playback::Failed,
    }
}

struct Feed {
    samples: Vec<f32>,
    at: usize,
}

fn build_out<T>(device: &cpal::Device, config: &StreamConfig, feed: Arc<Mutex<Feed>>) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let stream = device.build_output_stream::<T, _, _>(
        *config,
        move |out: &mut [T], _| {
            let mut f = feed.lock().unwrap_or_else(|e| e.into_inner());
            for frame in out.chunks_mut(channels) {
                let v = f.samples.get(f.at).copied().unwrap_or(0.0);
                f.at += 1;
                for s in frame.iter_mut() {
                    *s = T::from_sample(v);
                }
            }
        },
        |_| {},
        None,
    )?;
    Ok(stream)
}

fn play_file(wav: &Path, device: Option<&str>, cancelled: &AtomicBool) -> Result<bool> {
    let bytes = std::fs::read(wav).context("read speech wav")?;
    let wav = parse_wav(&bytes)?;
    let mono = to_mono(&wav.samples, wav.channels as usize);
    play_samples(&mono, wav.rate, device, cancelled)
}

pub(crate) fn play_samples(mono: &[f32], rate: u32, device: Option<&str>, cancelled: &AtomicBool) -> Result<bool> {
    let (out_device, _) = find_device(false, device)?;
    let supported = out_device.default_output_config().context("read the speaker format")?;
    let config = supported.config();
    let mut samples = Resampler::new(rate, config.sample_rate).push(mono);
    samples.extend(std::iter::repeat_n(0.0, (config.sample_rate as u64 * TAIL_PAD_MS / 1000) as usize));
    let total = samples.len();
    let feed = Arc::new(Mutex::new(Feed { samples, at: 0 }));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_out::<f32>(&out_device, &config, feed.clone())?,
        SampleFormat::I16 => build_out::<i16>(&out_device, &config, feed.clone())?,
        SampleFormat::I32 => build_out::<i32>(&out_device, &config, feed.clone())?,
        SampleFormat::U16 => build_out::<u16>(&out_device, &config, feed.clone())?,
        other => bail!("speaker sample format {other} is not supported"),
    };
    stream.play().context("start the speaker")?;
    let limit = Instant::now() + Duration::from_millis(total as u64 * 1000 / config.sample_rate.max(1) as u64 + 2000);
    loop {
        if stopped(cancelled) {
            return Ok(true);
        }
        if feed.lock().unwrap_or_else(|e| e.into_inner()).at >= total {
            return Ok(false);
        }
        if Instant::now() > limit {
            bail!("speaker playback timed out");
        }
        thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_failure_and_interruption_are_distinct() {
        assert_eq!(playback_result(Some(false), false), Playback::Failed);
        assert_eq!(playback_result(Some(false), true), Playback::Interrupted);
        assert_eq!(playback_result(None, false), Playback::Interrupted);
        assert_eq!(playback_result(Some(true), false), Playback::Played);
    }

    #[test]
    fn first_chunk_is_short_then_longer() {
        let text = "One two three. ".repeat(40);
        let parts = chunks(&text);
        assert!(parts.len() >= 2);
        assert!(parts[0].len() <= FIRST_CHUNK + 20, "{}", parts[0].len());
        assert!(parts[1].len() > FIRST_CHUNK);
        assert!(parts.iter().all(|p| p.len() <= NEXT_CHUNK + 20));
        assert_eq!(parts.join(" ").split_whitespace().count(), text.split_whitespace().count());
    }

    #[test]
    fn chunks_split_on_lines_and_drop_blanks() {
        assert_eq!(chunks("Hello there.\n\nNext line"), vec!["Hello there. Next line".to_string()]);
        assert!(chunks("   ").is_empty());
        assert_eq!(chunks("v1.2 is out"), vec!["v1.2 is out".to_string()]);
    }

    #[test]
    fn agent_voices_match_nixa() {
        assert_eq!(voice_for("nixa"), "Pippa");
        assert_eq!(voice_for("Claude"), "Harper");
        assert_eq!(voice_for("someone"), "Delia");
    }

    #[test]
    #[ignore]
    fn ptts_makes_a_wav_we_can_read() {
        let bin = ptts_bin().expect("ptts");
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("t.wav");
        let mut child = spawn_ptts(&bin, "Toby", "Hello, this is how I sound.", &wav).unwrap();
        assert_eq!(wait_child(&mut child, &AtomicBool::new(false)), Some(true));
        let parsed = parse_wav(&std::fs::read(&wav).unwrap()).unwrap();
        eprintln!("ptts wav: {} Hz, {} ch, {} samples", parsed.rate, parsed.channels, parsed.samples.len());
        assert!(parsed.samples.len() > parsed.rate as usize / 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn plays_silence_on_the_default_output() {
        let stop = play_samples(&[0.0; 4800], 24_000, None, &AtomicBool::new(false)).unwrap();
        assert!(!stop);
    }

    #[test]
    fn reserved_words_get_a_dot() {
        assert_eq!(safe_text("stop"), "stop.");
        assert_eq!(safe_text("--help"), "--help.");
        assert_eq!(safe_text("stop the build"), "stop the build");
    }
}
