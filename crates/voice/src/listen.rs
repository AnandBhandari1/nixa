use crate::keys::{KeyTap, read_keys, we_are_front};
use crate::mic::{MicOpen, echo_on, open_echo, open_mic};
use crate::own_voice::real_words;
use crate::pcm::{SAMPLE_RATE, level};
use crate::speak::{heard_own_echo, heard_own_voice, is_speaking, stop_speaking};
use crate::stt::{parakeet_paths, stt_ready, transcribe};
use crate::vad::{Vad, VadOut};
use crate::words::{Words, names_for};
use anyhow::{Result, bail};
use std::path::PathBuf;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const RESTART: Duration = Duration::from_millis(1000);
const RECHECK: Duration = Duration::from_millis(15_000);
const KEY_POLL: Duration = Duration::from_millis(20);
const HOLD_MAX_SAMPLES: usize = SAMPLE_RATE as usize * 120;
const HOLD_MIN_SAMPLES: usize = SAMPLE_RATE as usize * 3 / 10;
const PARAKEET: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
const VAD_MODEL: &str = "silero_vad.onnx";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupt {
    Off,
    Speech,
    Words,
}

#[derive(Clone, Debug)]
pub struct ListenConfig {
    pub models_dir: PathBuf,
    pub vad_model: String,
    pub parakeet: String,
    pub silence_ms: u32,
    pub input_device: Option<String>,
    pub names: Vec<String>,
    pub interrupt: Interrupt,
    pub hold_key: bool,
    pub hold_only: bool,
    pub front_bundles: Vec<String>,
    pub level_ms: u64,
    pub echo_cancel: bool,
    pub headphones: bool,
}

impl Default for ListenConfig {
    fn default() -> Self {
        let models_dir = std::env::var("NIXA_VOICE_MODELS")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| nixa_protocol::paths::data_dir().join("voice-models"));
        ListenConfig {
            models_dir,
            vad_model: VAD_MODEL.into(),
            parakeet: PARAKEET.into(),
            silence_ms: 700,
            input_device: None,
            names: names_for("nixa"),
            interrupt: Interrupt::Words,
            hold_key: false,
            hold_only: false,
            front_bundles: Vec::new(),
            level_ms: 100,
            echo_cancel: true,
            headphones: false,
        }
    }
}

impl ListenConfig {
    pub fn vad_path(&self) -> PathBuf {
        self.models_dir.join(&self.vad_model)
    }

    pub fn parakeet_dir(&self) -> PathBuf {
        self.models_dir.join(&self.parakeet)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Mic,
    Hold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ignored {
    NoWords,
    OwnVoice,
    TooShort,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    Ready { device: String, missing: bool },
    SttReady { ms: u64 },
    Level { level: f32, talking: bool },
    SpeechStart,
    SpeechEnd,
    Heard { text: String, via: Via, ms: u64, audio_ms: u64, named: bool, stop: bool },
    Ignored { text: String, why: Ignored },
    HoldStart,
    HoldLive,
    HoldEnd { audio_ms: u64 },
    HoldSent,
    KeyTap { front: bool },
    Interrupted,
    DeviceMissing { wanted: String },
    Error { message: String },
    Stopped,
}

struct Job {
    pcm: Vec<i16>,
    via: Via,
    during_voice: bool,
}

struct Shared {
    run: AtomicBool,
    holding: AtomicBool,
    hold_live: AtomicBool,
    hold: Mutex<Vec<i16>>,
    jobs: Mutex<Option<Sender<Job>>>,
    events: Mutex<Option<Sender<VoiceEvent>>>,
}

struct Running {
    id: u64,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    dispatcher: JoinHandle<()>,
}

static LISTENER: Mutex<Option<Running>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static DISPATCHING: Cell<Option<u64>> = const { Cell::new(None) };
}

fn stale_caller(id: u64) -> bool {
    DISPATCHING.with(|d| d.get().is_some_and(|mine| mine != id))
}

fn emit(shared: &Shared, event: VoiceEvent) {
    if let Some(tx) = shared.events.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(event);
    }
}

fn send_job(shared: &Shared, job: Job) {
    if let Some(tx) = shared.jobs.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(job);
    }
}

fn audio_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}

fn current() -> Option<Arc<Shared>> {
    LISTENER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|r| !stale_caller(r.id))
        .map(|r| r.shared.clone())
}

pub fn is_listening() -> bool {
    current().is_some()
}

pub fn holding() -> bool {
    current().is_some_and(|s| s.holding.load(Ordering::SeqCst))
}

fn begin_hold(shared: &Shared) {
    if shared.holding.swap(true, Ordering::SeqCst) {
        return;
    }
    shared.hold.lock().unwrap_or_else(|e| e.into_inner()).clear();
    shared.hold_live.store(false, Ordering::SeqCst);
    if is_speaking() {
        stop_speaking();
        emit(shared, VoiceEvent::Interrupted);
    }
    emit(shared, VoiceEvent::HoldStart);
}

fn end_hold(shared: &Shared) {
    if !shared.holding.swap(false, Ordering::SeqCst) {
        return;
    }
    let pcm = std::mem::take(&mut *shared.hold.lock().unwrap_or_else(|e| e.into_inner()));
    emit(shared, VoiceEvent::HoldEnd { audio_ms: audio_ms(pcm.len()) });
    if pcm.len() < HOLD_MIN_SAMPLES {
        emit(shared, VoiceEvent::Ignored { text: String::new(), why: Ignored::TooShort });
        return;
    }
    emit(shared, VoiceEvent::HoldSent);
    send_job(shared, Job { pcm, via: Via::Hold, during_voice: false });
}

pub fn hold_start() -> bool {
    current().map(|s| begin_hold(&s)).is_some()
}

pub fn hold_end() -> bool {
    current().map(|s| end_hold(&s)).is_some()
}

pub fn start_listening(config: ListenConfig, on_event: impl FnMut(VoiceEvent) + Send + 'static) -> Result<()> {
    stop_running(None);
    let vad = Vad::new(&config.vad_path(), config.silence_ms)?;
    let paths = parakeet_paths(&config.parakeet_dir());
    if !paths.encoder.exists() || !paths.tokens.exists() {
        bail!("speech to text model missing in {}", config.parakeet_dir().display());
    }
    let (event_tx, event_rx) = channel();
    let (job_tx, job_rx) = channel();
    let shared = Arc::new(Shared {
        run: AtomicBool::new(true),
        holding: AtomicBool::new(false),
        hold_live: AtomicBool::new(false),
        hold: Mutex::new(Vec::new()),
        jobs: Mutex::new(Some(job_tx)),
        events: Mutex::new(Some(event_tx)),
    });
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let dispatcher = spawn("nixa-voice-events", move || dispatch(id, event_rx, on_event))?;
    let mut threads = Vec::new();
    let (s, c) = (shared.clone(), config.clone());
    threads.push(spawn("nixa-voice-stt", move || stt_loop(&s, &c, job_rx))?);
    let (s, c) = (shared.clone(), config.clone());
    threads.push(spawn("nixa-voice-mic", move || mic_loop(&s, &c, vad))?);
    if config.hold_key {
        let (s, bundles) = (shared.clone(), config.front_bundles.clone());
        threads.push(spawn("nixa-voice-keys", move || key_loop(&s, &bundles))?);
    }
    *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Running { id, shared, threads, dispatcher });
    Ok(())
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>> {
    Ok(thread::Builder::new().name(name.into()).spawn(f)?)
}

fn dispatch(id: u64, rx: Receiver<VoiceEvent>, mut on_event: impl FnMut(VoiceEvent)) {
    DISPATCHING.with(|d| d.set(Some(id)));
    for event in rx {
        on_event(event);
    }
}

fn join(handle: JoinHandle<()>) {
    if handle.thread().id() != thread::current().id() {
        let _ = handle.join();
    }
}

pub fn stop() {
    stop_running(DISPATCHING.with(|d| d.get()));
}

fn stop_running(caller: Option<u64>) {
    let running = {
        let mut slot = LISTENER.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|r| caller.is_some_and(|c| c != r.id)) {
            return;
        }
        slot.take()
    };
    let Some(running) = running else {
        return;
    };
    let shared = running.shared;
    shared.run.store(false, Ordering::SeqCst);
    shared.holding.store(false, Ordering::SeqCst);
    shared.jobs.lock().unwrap_or_else(|e| e.into_inner()).take();
    for handle in running.threads {
        join(handle);
    }
    emit(&shared, VoiceEvent::Stopped);
    shared.events.lock().unwrap_or_else(|e| e.into_inner()).take();
    join(running.dispatcher);
}

fn stt_loop(shared: &Shared, config: &ListenConfig, jobs: Receiver<Job>) {
    let dir = config.parakeet_dir();
    match stt_ready(&dir) {
        Ok(ms) => emit(shared, VoiceEvent::SttReady { ms }),
        Err(e) => emit(shared, VoiceEvent::Error { message: format!("speech to text: {e:#}") }),
    }
    let words = Words::new(&config.names);
    for job in jobs {
        hear(shared, config, &words, job);
    }
}

fn hear(shared: &Shared, config: &ListenConfig, words: &Words, job: Job) {
    let heard = match transcribe(&config.parakeet_dir(), &job.pcm) {
        Ok(t) => t,
        Err(e) => {
            emit(shared, VoiceEvent::Error { message: format!("speech to text: {e:#}") });
            return;
        }
    };
    let text = heard.text.trim().to_string();
    if !real_words(&text) {
        emit(shared, VoiceEvent::Ignored { text, why: Ignored::NoWords });
        return;
    }
    let from_mic = job.via == Via::Mic;
    if from_mic && (heard_own_voice(&text) || (job.during_voice && heard_own_echo(&text))) {
        emit(shared, VoiceEvent::Ignored { text, why: Ignored::OwnVoice });
        return;
    }
    let stop = words.is_stop(&text);
    let named = words.named(&text);
    if is_speaking() && (stop || (from_mic && config.interrupt == Interrupt::Words)) {
        stop_speaking();
        emit(shared, VoiceEvent::Interrupted);
    }
    emit(
        shared,
        VoiceEvent::Heard { text, via: job.via, ms: heard.ms, audio_ms: audio_ms(job.pcm.len()), named, stop },
    );
}

struct MicState {
    open: Option<MicOpen>,
    failed: Arc<AtomicBool>,
    retry_at: Instant,
    recheck_at: Instant,
    said_missing: bool,
    was_holding: bool,
    meter: Vec<i16>,
    metering: bool,
    voice_at_start: bool,
    echo_broken: bool,
}

fn device_back(wanted: Option<&str>) -> bool {
    crate::devices::find_device(true, wanted).is_ok_and(|(_, missing)| !missing)
}

fn mic_loop(shared: &Shared, config: &ListenConfig, mut vad: Vad) {
    let (tx, rx) = channel::<Vec<i16>>();
    let now = Instant::now();
    let mut st = MicState {
        open: None,
        failed: Arc::new(AtomicBool::new(false)),
        retry_at: now,
        recheck_at: now + RECHECK,
        said_missing: false,
        was_holding: false,
        meter: Vec::new(),
        metering: false,
        voice_at_start: false,
        echo_broken: false,
    };
    let wanted = config.input_device.as_deref();
    while shared.run.load(Ordering::SeqCst) {
        let now = Instant::now();
        let wanted_open = mic_wanted(config.hold_only, shared.holding.load(Ordering::SeqCst));
        if !wanted_open && st.open.is_some() {
            st.open = None;
            vad.reset();
        }
        if wanted_open && st.open.is_none() && now >= st.retry_at {
            st.failed.store(false, Ordering::SeqCst);
            let echo = config.echo_cancel && !st.echo_broken;
            let opened = if echo { open_echo(wanted, tx.clone(), st.failed.clone()) } else { open_mic(wanted, tx.clone(), st.failed.clone()) };
            match opened {
                Ok(m) => {
                    if m.missing && !st.said_missing {
                        emit(shared, VoiceEvent::DeviceMissing { wanted: wanted.unwrap_or_default().to_string() });
                    }
                    st.said_missing = m.missing;
                    st.recheck_at = now + RECHECK;
                    emit(shared, VoiceEvent::Ready { device: m.name.clone(), missing: m.missing });
                    st.open = Some(m);
                }
                Err(e) if echo => {
                    emit(shared, VoiceEvent::Error { message: format!("mic: {e:#}; using the mic without echo cancel") });
                    st.echo_broken = true;
                    st.retry_at = now;
                }
                Err(e) => {
                    emit(shared, VoiceEvent::Error { message: format!("mic: {e:#}") });
                    st.retry_at = now + RESTART;
                }
            }
        }
        if st.open.is_some() && st.failed.load(Ordering::SeqCst) {
            st.open = None;
            vad.reset();
            emit(shared, VoiceEvent::Error { message: "mic stopped, starting it again".into() });
            st.retry_at = now + RESTART;
        }
        if st.open.as_ref().is_some_and(|m| m.missing) && now >= st.recheck_at {
            st.recheck_at = now + RECHECK;
            if device_back(wanted) {
                st.open = None;
                vad.reset();
                st.retry_at = now;
            }
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => on_frame(shared, config, &mut vad, &mut st, frame),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

pub(crate) fn mic_wanted(hold_only: bool, holding: bool) -> bool {
    !hold_only || holding
}

pub(crate) fn echo_safe(headphones: bool, echo_on: bool) -> bool {
    headphones || echo_on
}

pub(crate) fn barge_in(interrupt: Interrupt, speaking: bool, echo_safe: bool) -> bool {
    speaking
        && match interrupt {
            Interrupt::Off => false,
            Interrupt::Speech => true,
            Interrupt::Words => echo_safe,
        }
}

fn on_frame(shared: &Shared, config: &ListenConfig, vad: &mut Vad, st: &mut MicState, frame: Vec<i16>) {
    let holding = shared.holding.load(Ordering::SeqCst);
    if holding != st.was_holding {
        vad.reset();
        st.was_holding = holding;
    }
    if holding {
        let full = {
            let mut hold = shared.hold.lock().unwrap_or_else(|e| e.into_inner());
            hold.extend_from_slice(&frame);
            hold.len() >= HOLD_MAX_SAMPLES
        };
        if !shared.hold_live.swap(true, Ordering::SeqCst) {
            emit(shared, VoiceEvent::HoldLive);
        }
        if full {
            end_hold(shared);
        }
    } else if !config.hold_only {
        for out in vad.push(&frame) {
            match out {
                VadOut::Start => {
                    st.voice_at_start = is_speaking();
                    emit(shared, VoiceEvent::SpeechStart);
                    let safe = echo_safe(config.headphones, st.open.as_ref().is_some_and(echo_on));
                    if barge_in(config.interrupt, st.voice_at_start, safe) {
                        stop_speaking();
                        emit(shared, VoiceEvent::Interrupted);
                    }
                }
                VadOut::End => emit(shared, VoiceEvent::SpeechEnd),
                VadOut::Phrase(pcm) => {
                    let during_voice = std::mem::take(&mut st.voice_at_start) || is_speaking();
                    send_job(shared, Job { pcm, via: Via::Mic, during_voice });
                }
            }
        }
    }
    let hears = holding || !config.hold_only;
    if config.level_ms > 0 && !hears && st.metering {
        st.metering = false;
        st.meter.clear();
        emit(shared, VoiceEvent::Level { level: 0.0, talking: false });
    }
    if config.level_ms > 0 && hears {
        st.metering = true;
        st.meter.extend_from_slice(&frame);
        let every = (SAMPLE_RATE as u64 * config.level_ms / 1000) as usize;
        if st.meter.len() >= every {
            let talking = holding || vad.is_speaking();
            emit(shared, VoiceEvent::Level { level: level(&st.meter), talking });
            st.meter.clear();
        }
    }
}

fn key_loop(shared: &Shared, bundles: &[String]) {
    let mut tap = KeyTap::default();
    let mut command = KeyTap::default();
    while shared.run.load(Ordering::SeqCst) {
        let keys = read_keys();
        let f9 = tap.step(&keys.f9, || we_are_front(bundles));
        let right = command.step(&keys.command, || we_are_front(bundles));
        if let Some(front) = f9.or(right) {
            emit(shared, VoiceEvent::KeyTap { front });
            if shared.holding.load(Ordering::SeqCst) {
                end_hold(shared);
            } else {
                begin_hold(shared);
            }
        }
        thread::sleep(KEY_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn install_fake(id: u64) -> Receiver<VoiceEvent> {
        let (event_tx, event_rx) = channel();
        let shared = Arc::new(Shared {
            run: AtomicBool::new(true),
            holding: AtomicBool::new(false),
            hold_live: AtomicBool::new(false),
            hold: Mutex::new(Vec::new()),
            jobs: Mutex::new(None),
            events: Mutex::new(Some(event_tx)),
        });
        let dispatcher = thread::spawn(|| {});
        *LISTENER.lock().unwrap() = Some(Running { id, shared, threads: Vec::new(), dispatcher });
        event_rx
    }

    #[test]
    fn old_callback_cannot_stop_the_new_listener() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let events = install_fake(9_001);
        let old = thread::spawn(|| {
            DISPATCHING.with(|d| d.set(Some(9_000)));
            stop();
            hold_start()
        });
        assert!(!old.join().unwrap(), "a stale callback must not see the new listener");
        assert!(is_listening(), "the new listener must stay alive");
        let own = thread::spawn(|| {
            DISPATCHING.with(|d| d.set(Some(9_001)));
            stop();
        });
        own.join().unwrap();
        assert!(!is_listening());
        assert_eq!(events.recv().unwrap(), VoiceEvent::Stopped);
    }

    #[test]
    fn default_config_uses_our_data_folder() {
        let c = ListenConfig::default();
        if std::env::var("NIXA_VOICE_MODELS").is_err() {
            assert!(c.models_dir.ends_with(".config/nixa/voice-models") || c.models_dir.ends_with("voice-models"));
        }
        assert!(c.vad_path().ends_with("silero_vad.onnx"));
        assert_eq!(c.silence_ms, 700);
        assert!(c.names.iter().any(|n| n == "nixa"));
    }

    #[test]
    fn hotkey_mode_opens_the_mic_only_while_recording() {
        assert!(!mic_wanted(true, false));
        assert!(mic_wanted(true, true));
        assert!(mic_wanted(false, false));
        assert!(mic_wanted(false, true));
    }

    #[test]
    fn user_speech_stops_nixa_at_once_when_echo_is_safe() {
        assert!(barge_in(Interrupt::Words, true, true));
        assert!(barge_in(Interrupt::Speech, true, false));
    }

    #[test]
    fn nixa_does_not_stop_itself_on_its_own_echo() {
        assert!(!barge_in(Interrupt::Words, true, false), "no echo cancel and no headphones waits for the words");
        assert!(!barge_in(Interrupt::Words, false, true), "nothing to stop when Nixa is quiet");
        assert!(!barge_in(Interrupt::Off, true, true));
    }

    #[test]
    fn echo_is_safe_with_headphones_or_a_working_echo_cancel() {
        assert!(echo_safe(true, false));
        assert!(echo_safe(false, true));
        assert!(!echo_safe(false, false));
    }

    #[test]
    fn hold_without_listener_does_nothing() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!hold_start());
        assert!(!hold_end());
        assert!(!holding());
    }

    #[test]
    #[ignore]
    fn restart_from_inside_the_callback_keeps_the_new_listener() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config = ListenConfig { level_ms: 0, ..ListenConfig::default() };
        let (tx, rx) = channel();
        let again = config.clone();
        let first_tx = tx.clone();
        start_listening(config, move |e| {
            if matches!(e, VoiceEvent::Ready { .. }) {
                let tx = first_tx.clone();
                start_listening(again.clone(), move |e| {
                    let _ = tx.send(format!("new {e:?}"));
                })
                .unwrap();
            }
            if e == VoiceEvent::Stopped {
                stop();
                let _ = first_tx.send("old stopped".to_string());
            }
        })
        .unwrap();
        let until = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < until && !seen.iter().any(|s: &String| s.starts_with("new Ready")) {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) {
                seen.push(s);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        while let Ok(s) = rx.try_recv() {
            seen.push(s);
        }
        eprintln!("{seen:?}");
        assert!(seen.iter().any(|s| s == "old stopped"));
        assert!(seen.iter().any(|s| s.starts_with("new Ready")));
        assert!(is_listening(), "the new listener was stopped by the old callback");
        stop();
        assert!(!is_listening());
    }

    #[test]
    #[ignore]
    fn round_trip_through_blackhole() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use crate::speak::{play_samples, set_output_device, speak, speak_wait};
        use crate::wav::parse_wav;
        let device = "BlackHole 2ch";
        let config = ListenConfig { input_device: Some(device.into()), interrupt: Interrupt::Off, level_ms: 0, echo_cancel: false, ..ListenConfig::default() };
        let (tx, rx) = channel();
        start_listening(config.clone(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let wait_for = |want: &dyn Fn(&VoiceEvent) -> bool, secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < until {
                if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
                    eprintln!("event: {e:?}");
                    if want(&e) {
                        return Some(e);
                    }
                }
            }
            None
        };
        assert!(wait_for(&|e| matches!(e, VoiceEvent::SttReady { .. }), 30).is_some());
        set_output_device(Some(device.into()));
        let said = speak_wait("The build is done and all of the tests pass now.", "Toby");
        eprintln!("said: {said:?}");
        assert!(!said.interrupted);
        assert!(wait_for(&|e| matches!(e, VoiceEvent::Ignored { why: Ignored::OwnVoice, .. }), 15).is_some());
        let bytes = std::fs::read(config.parakeet_dir().join("test_wavs/en.wav")).unwrap();
        let wav = parse_wav(&bytes).unwrap();
        play_samples(&wav.samples, wav.rate, Some(device), &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let heard = wait_for(&|e| matches!(e, VoiceEvent::Heard { .. }), 15).expect("heard");
        if let VoiceEvent::Heard { text, .. } = heard {
            assert!(text.to_lowercase().contains("country"), "{text}");
        }
        let long = speak("This is a long line that keeps going. It has many parts. It should be cut off early by the stop call.", "Harper");
        std::thread::sleep(Duration::from_millis(1500));
        let started = Instant::now();
        stop_speaking();
        let cut = long.recv().unwrap();
        eprintln!("cut: {cut:?} after stop in {} ms", started.elapsed().as_millis());
        assert!(cut.interrupted);
        assert!(started.elapsed() < Duration::from_millis(500));
        set_output_device(None);
        stop();
        assert!(!is_listening());
    }

    #[test]
    #[ignore]
    fn hold_only_hears_nothing_between_presses() {
        use crate::pcm::{Resampler, f32_to_i16, to_mono};
        use crate::wav::parse_wav;
        let config = ListenConfig { hold_only: true, level_ms: 100, ..ListenConfig::default() };
        let bytes = std::fs::read(config.parakeet_dir().join("test_wavs/en.wav")).unwrap();
        let wav = parse_wav(&bytes).unwrap();
        let mono = Resampler::new(wav.rate, SAMPLE_RATE).push(&to_mono(&wav.samples, wav.channels as usize));
        let pcm = f32_to_i16(&mono);
        let (event_tx, event_rx) = channel();
        let (job_tx, job_rx) = channel();
        let shared = Shared {
            run: AtomicBool::new(true),
            holding: AtomicBool::new(false),
            hold_live: AtomicBool::new(false),
            hold: Mutex::new(Vec::new()),
            jobs: Mutex::new(Some(job_tx)),
            events: Mutex::new(Some(event_tx)),
        };
        let mut vad = Vad::new(&config.vad_path(), config.silence_ms).unwrap();
        let mut st = MicState {
            open: None,
            failed: Arc::new(AtomicBool::new(false)),
            retry_at: Instant::now(),
            recheck_at: Instant::now(),
            said_missing: false,
            was_holding: false,
            meter: Vec::new(),
            metering: true,
            voice_at_start: false,
            echo_broken: false,
        };
        for frame in pcm.chunks(1600) {
            on_frame(&shared, &config, &mut vad, &mut st, frame.to_vec());
        }
        let quiet: Vec<VoiceEvent> = event_rx.try_iter().collect();
        assert_eq!(quiet, vec![VoiceEvent::Level { level: 0.0, talking: false }]);
        assert!(job_rx.try_recv().is_err());
        shared.holding.store(true, Ordering::SeqCst);
        for frame in pcm.chunks(1600) {
            on_frame(&shared, &config, &mut vad, &mut st, frame.to_vec());
        }
        assert_eq!(shared.hold.lock().unwrap().len(), pcm.len());
        let held: Vec<VoiceEvent> = event_rx.try_iter().collect();
        assert_eq!(held.iter().filter(|e| **e == VoiceEvent::HoldLive).count(), 1, "{held:?}");
        assert!(held.iter().all(|e| matches!(e, VoiceEvent::Level { talking: true, .. } | VoiceEvent::HoldLive)), "{held:?}");
        assert!(!held.is_empty());
    }

    #[test]
    fn start_fails_without_models() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let config = ListenConfig { models_dir: PathBuf::from("/nonexistent"), ..ListenConfig::default() };
        assert!(start_listening(config, |_| {}).is_err());
        assert!(!is_listening());
    }
}
