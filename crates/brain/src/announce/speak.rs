use super::identity::{announcement, identity, spoken};
use super::{AgentEvent, Track, add_event, is_on, space_for};
use crate::app::{App, lock};
use crate::util::now_ms;
use nixa_protocol::Status;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const SOUND_DIR: &str = "/System/Library/Sounds";
pub const AFPLAY: &str = "/usr/bin/afplay";
pub const SOUND_LIMIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(50);

static PLAYING: AtomicBool = AtomicBool::new(false);
static HUSH: AtomicU64 = AtomicU64::new(0);

pub fn hush() {
    HUSH.fetch_add(1, Ordering::SeqCst);
}

pub fn hushed_since(mark: u64) -> bool {
    HUSH.load(Ordering::SeqCst) != mark
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Blocked,
    Finished,
}

impl Change {
    pub fn kind(self) -> &'static str {
        match self {
            Change::Blocked => "blocked",
            Change::Finished => "done",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Change::Blocked => "needs attention",
            Change::Finished => "finished",
        }
    }
}

pub fn change(before: Option<Status>, now: Option<Status>) -> Option<Change> {
    if now == Some(Status::Blocked) && before != Some(Status::Blocked) {
        return Some(Change::Blocked);
    }
    let was_busy = matches!(before, Some(Status::Working) | Some(Status::Blocked));
    let at_rest = matches!(now, Some(Status::Done) | Some(Status::Idle));
    (was_busy && at_rest).then_some(Change::Finished)
}

pub fn sound_file(kind: &str) -> String {
    let name = match kind {
        "blocked" => "Funk",
        "done" => "Glass",
        _ => "Pop",
    };
    format!("{SOUND_DIR}/{name}.aiff")
}

pub fn play(kind: &str) {
    if cfg!(test) {
        return;
    }
    play_with(AFPLAY, &[sound_file(kind)], &PLAYING, SOUND_LIMIT);
}

pub fn play_with(program: &str, args: &[String], slot: &'static AtomicBool, limit: Duration) -> bool {
    if slot.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return false;
    }
    let program = program.to_string();
    let args = args.to_vec();
    let (told, started) = std::sync::mpsc::channel();
    let mark = HUSH.load(Ordering::SeqCst);
    let worker = std::thread::Builder::new().name("nixa-sound".into()).spawn(move || {
        let child = Command::new(&program).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = child else {
            slot.store(false, Ordering::SeqCst);
            let _ = told.send(false);
            return;
        };
        let _ = told.send(true);
        let until = Instant::now() + limit;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < until && !hushed_since(mark) => std::thread::sleep(POLL),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
        slot.store(false, Ordering::SeqCst);
    });
    if worker.is_err() {
        slot.store(false, Ordering::SeqCst);
        return false;
    }
    started.recv().unwrap_or(false)
}

pub fn say(app: &Arc<App>, text: &str, event: &AgentEvent) {
    if !is_on(app) || !crate::voice::settings(app).voice_on {
        return;
    }
    let voice = crate::voice_map::agent_voice(app, event);
    if crate::talk::hold::catch(app, text, &held_options(event, &voice)) {
        return;
    }
    if cfg!(test) {
        return;
    }
    crate::voice::set_nixa_talking(app, true);
    let kind = event.kind.clone();
    let enabled = Arc::downgrade(app);
    let done = nixa_voice::speak_with_fallback(text, &voice, move || {
        if enabled.upgrade().is_some_and(|app| is_on(&app)) {
            play(&kind);
        }
    });
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let _ = done.recv();
        if !nixa_voice::is_speaking() {
            crate::voice::set_nixa_talking(&app, false);
        }
    });
}

pub fn held_options(event: &AgentEvent, voice: &str) -> crate::talk::speech::SayOptions {
    let given = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    crate::talk::speech::SayOptions {
        voice: given(voice),
        agent: given(&event.agent).or_else(|| Some("agent".into())),
        terminal: given(&event.terminal),
        model: None,
        space: given(&event.space),
        kind: given(&event.kind),
        mac_only: false,
    }
}

pub fn event_for(app: &App, track: &Track, kind: &str, words: &str) -> (AgentEvent, String) {
    let space = space_for(app, &track.info).map(|s| s.name).unwrap_or_default();
    let numbers = lock(&app.announce.numbers).clone();
    let who = identity(&track.info, &track.agent, &numbers, &space);
    let event = AgentEvent {
        time: now_ms(),
        kind: kind.to_string(),
        text: announcement(&who, words),
        terminal: track.info.id.to_string(),
        space: who.space.clone(),
        job: who.job.clone(),
        agent_number: who.number,
        agent: track.agent.clone(),
        offer: None,
    };
    (event, spoken(&who, words))
}

pub fn announce(app: &Arc<App>, track: &Track, change: Change) -> AgentEvent {
    let (event, words) = event_for(app, track, change.kind(), change.words());
    add_event(app, event.clone());
    say(app, &words, &event);
    event
}

#[cfg(test)]
mod hold_tests {
    use super::*;

    #[tokio::test]
    async fn an_agent_notice_is_held_when_nixa_speaks_on_the_phone() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        crate::talk::speech::set_phone(&app, Some(tx));
        crate::app::lock(&app.voice.settings).voice_on = true;
        crate::talk::hold::start(&app, "p1");
        let event = AgentEvent {
            time: now_ms(),
            kind: "blocked".into(),
            text: "Agent 2 needs attention".into(),
            terminal: "m2".into(),
            space: String::new(),
            job: String::new(),
            agent_number: Some(2),
            agent: "claude".into(),
            offer: None,
        };
        say(&app, "Agent 2 needs attention.", &event);
        assert_eq!(lock(&app.talk.hold).texts(), ["Agent 2 needs attention."]);
        crate::talk::speech::set_phone(&app, None);
        crate::talk::hold::cancel(&app, "p2");
        say(&app, "Agent 3 needs attention.", &event);
        assert_eq!(lock(&app.talk.hold).held(), 1);
    }

    #[tokio::test]
    async fn nothing_is_said_when_voice_is_off_even_with_a_phone() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        crate::talk::speech::set_phone(&app, Some(tx));
        assert!(!crate::voice::settings(&app).voice_on);
        crate::talk::hold::start(&app, "p1");
        let event = AgentEvent {
            time: now_ms(),
            kind: "blocked".into(),
            text: "Agent 2 needs attention".into(),
            terminal: "m2".into(),
            space: String::new(),
            job: String::new(),
            agent_number: Some(2),
            agent: "claude".into(),
            offer: None,
        };
        say(&app, "Agent 2 needs attention.", &event);
        assert_eq!(lock(&app.talk.hold).held(), 0);
        crate::talk::hold::cancel(&app, "p2");
        let said = crate::talk::speech::say(&app, "Nixa line while voice is off.", crate::talk::speech::SayOptions::nixa("chat")).await;
        assert_eq!(said.ms, 0);
        assert!(rx.try_recv().is_err());
    }
}

#[cfg(test)]
mod sound_tests {
    use super::*;

    fn wait_free(slot: &AtomicBool, max: Duration) -> bool {
        let until = Instant::now() + max;
        while slot.load(Ordering::SeqCst) {
            if Instant::now() > until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn sleepers() -> usize {
        let out = Command::new("/bin/ps").args(["-o", "args=", "-g", &std::process::id().to_string()]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("sleep 31")).count()
    }

    #[test]
    fn hush_stops_a_playing_alert_at_once() {
        static SLOT: AtomicBool = AtomicBool::new(false);
        let args = vec!["32".to_string()];
        assert!(play_with("/bin/sleep", &args, &SLOT, Duration::from_secs(20)));
        let started = Instant::now();
        hush();
        assert!(wait_free(&SLOT, Duration::from_secs(2)), "the alert kept playing after the user spoke");
        assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    }

    #[test]
    fn a_stuck_player_makes_one_process_and_one_thread_not_one_per_sound() {
        static SLOT: AtomicBool = AtomicBool::new(false);
        let args = vec!["31".to_string()];
        let started: Vec<bool> = (0..50).map(|_| play_with("/bin/sleep", &args, &SLOT, Duration::from_millis(600))).collect();
        assert_eq!(started.iter().filter(|s| **s).count(), 1);
        assert!(sleepers() <= 1, "more than one stuck player is alive");
        assert!(wait_free(&SLOT, Duration::from_secs(5)), "the stuck player was not stopped at the time limit");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(sleepers(), 0, "the stuck player is still alive after the time limit");
        assert!(play_with("/bin/sleep", &args, &SLOT, Duration::from_millis(200)));
        assert!(wait_free(&SLOT, Duration::from_secs(5)));
    }

    #[test]
    fn a_short_sound_frees_the_slot_when_it_ends() {
        static SLOT: AtomicBool = AtomicBool::new(false);
        assert!(play_with("/usr/bin/true", &[], &SLOT, SOUND_LIMIT));
        assert!(wait_free(&SLOT, Duration::from_secs(2)));
        assert!(play_with("/usr/bin/true", &[], &SLOT, SOUND_LIMIT));
        assert!(wait_free(&SLOT, Duration::from_secs(2)));
    }

    #[test]
    fn a_missing_player_frees_the_slot() {
        static SLOT: AtomicBool = AtomicBool::new(false);
        assert!(!play_with("/no/such/player", &[], &SLOT, SOUND_LIMIT));
        assert!(!SLOT.load(Ordering::SeqCst));
    }
}
