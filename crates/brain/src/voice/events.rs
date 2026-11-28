use crate::agent;
use crate::app::{App, lock};
use crate::chat::{Extra, Who};
use crate::util::{log, now_ms};
use nixa_voice::{Ignored, Via, VoiceEvent};
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Stdio};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const READY_CUE: &str = "/System/Library/Sounds/Tink.aiff";
pub const SEND_CUE: &str = "/System/Library/Sounds/Pop.aiff";
const AFPLAY: &str = "/usr/bin/afplay";
const CUE_VOLUME: &str = "0.5";
const CUE_LIMIT: Duration = Duration::from_secs(3);
const CUE_POLL: Duration = Duration::from_millis(50);
pub const CUE_WAITING_MAX: usize = 4;

#[derive(Debug, Default)]
pub struct Cues {
    playing: bool,
    waiting: VecDeque<String>,
}

static CUES: Mutex<Cues> = Mutex::new(Cues { playing: false, waiting: VecDeque::new() });

pub const NOTHING_HEARD: &str = "(nothing heard)";

pub fn talking_event(who: &str, on: bool) -> Value {
    json!({"type": "talking", "who": who, "on": on})
}

fn via_word(via: Via) -> &'static str {
    match via {
        Via::Mic => "mic",
        Via::Hold => "hold",
    }
}

pub fn on_event(app: &Arc<App>, event: VoiceEvent) {
    match event {
        VoiceEvent::Ready { device, missing } => {
            {
                let mut live = lock(&app.voice.live);
                live.listening = true;
                live.device = Some(device.clone());
                live.missing = missing;
                live.error = None;
            }
            log("voice", &format!("mic on: {device}"));
            super::send_state(app);
        }
        VoiceEvent::SttReady { ms } => {
            lock(&app.voice.live).stt_ready = true;
            log("voice", &format!("speech to text ready in {ms} ms"));
            super::send_state(app);
        }
        VoiceEvent::Level { level, talking } => {
            lock(&app.voice.live).level = level;
            crate::talk::flow::on_level(app, level, talking);
        }
        VoiceEvent::SpeechStart => {
            crate::talk::flow::on_speech_start(app);
            set_user_talking(app, true);
        }
        VoiceEvent::SpeechEnd => {
            crate::talk::flow::on_speech_end(app);
            set_user_talking(app, false);
        }
        VoiceEvent::HoldStart => {
            crate::talk::speech::barge_in(app);
            lock(&app.voice.live).holding = true;
            set_user_talking(app, true);
            super::send_state(app);
        }
        VoiceEvent::HoldLive => {
            log("voice", "hold is recording");
            cue(app, READY_CUE);
        }
        VoiceEvent::HoldSent => cue(app, SEND_CUE),
        VoiceEvent::HoldEnd { audio_ms } => {
            lock(&app.voice.live).holding = false;
            set_user_talking(app, false);
            log("voice", &format!("hold ended after {audio_ms} ms"));
            super::send_state(app);
        }
        VoiceEvent::Heard { text, via, ms, audio_ms, named, stop } => {
            lock(&app.voice.live).heard += 1;
            log("voice", &format!("heard by {} in {ms} ms ({audio_ms} ms of audio, named {named}, stop {stop})", via_word(via)));
            match (stop, via) {
                (true, _) => heard(app, &text, stop),
                (false, Via::Hold) => crate::talk::flow::on_held(app, &text, ms, audio_ms),
                (false, Via::Mic) => crate::talk::flow::on_heard(app, &text, ms, audio_ms),
            }
        }
        VoiceEvent::Ignored { why, .. } => ignored(app, why),
        VoiceEvent::KeyTap { front } => log("voice", &format!("talk key tap, Nixa in front: {front}")),
        VoiceEvent::Interrupted => {
            crate::talk::speech::barge_in(app);
            app.send_out(json!({"type": "interrupted"}));
        }
        VoiceEvent::DeviceMissing { wanted } => {
            app.info(&format!("The mic {wanted} is not found. Nixa uses the Mac default mic until it is back."));
        }
        VoiceEvent::Error { message } => {
            log("voice", &message);
            lock(&app.voice.live).error = Some(message);
            super::send_state(app);
        }
        VoiceEvent::Stopped => {
            {
                let mut live = lock(&app.voice.live);
                live.listening = false;
                live.holding = false;
                live.user_talking = false;
            }
            crate::talk::flow::mic_off(app);
            super::send_state(app);
        }
    }
}

fn cue(app: &App, file: &str) {
    if cfg!(test) || lock(&app.talk.speaker).phone_on || !Path::new(file).exists() {
        return;
    }
    if !ask_cue(&mut lock(&CUES), file) {
        return;
    }
    let started = std::thread::Builder::new().name("nixa-cue".into()).spawn(|| {
        loop {
            let next = next_cue(&mut lock(&CUES));
            let Some(file) = next else { break };
            play_cue(&file);
        }
    });
    if started.is_err() {
        let mut cues = lock(&CUES);
        cues.playing = false;
        cues.waiting.clear();
    }
}

pub fn ask_cue(cues: &mut Cues, file: &str) -> bool {
    if cues.waiting.len() < CUE_WAITING_MAX {
        cues.waiting.push_back(file.to_string());
    }
    !std::mem::replace(&mut cues.playing, true)
}

pub fn next_cue(cues: &mut Cues) -> Option<String> {
    let next = cues.waiting.pop_front();
    cues.playing = next.is_some();
    next
}

fn play_cue(file: &str) {
    let child = Command::new(AFPLAY).args(["-v", CUE_VOLUME, file]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else {
        log("voice", "the recording cue did not play");
        return;
    };
    let until = Instant::now() + CUE_LIMIT;
    while matches!(child.try_wait(), Ok(None)) {
        if Instant::now() >= until {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(CUE_POLL);
    }
    let _ = child.wait();
}

fn set_user_talking(app: &Arc<App>, on: bool) {
    let changed = {
        let mut live = lock(&app.voice.live);
        let changed = live.user_talking != on;
        live.user_talking = on;
        changed
    };
    if changed {
        app.send_out(talking_event("user", on));
    }
}

fn heard(app: &Arc<App>, text: &str, stop: bool) {
    app.send_out(json!({"type": "heard", "text": text}));
    if stop {
        lock(&app.talk.gather).drop_all();
        crate::talk::speech::stop(app);
        crate::talk::finish_confirm(app, false, None, crate::talk::End::No);
        lock(&app.talk.heard).stopped_at = now_ms();
        app.add_chat(Who::User, text, Extra::default());
        if agent::interrupt(app) {
            log("voice", "stopped the brain");
        }
        crate::talk::hold::answered(app);
        app.send_out(json!({"type": "stopped"}));
        return;
    }
    agent::user_said(app, text, false);
}

fn ignored(app: &Arc<App>, why: Ignored) {
    match why {
        Ignored::TooShort => app.send_out(json!({"type": "heard", "text": NOTHING_HEARD})),
        Ignored::OwnVoice => log("voice", "heard Nixa's own voice, dropped"),
        Ignored::NoWords => log("voice", "no words in the phrase"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_rounded() {
        assert_eq!(talking_event("nixa", false), json!({"type": "talking", "who": "nixa", "on": false}));
    }

    #[test]
    fn a_cue_that_comes_while_another_plays_waits_its_turn() {
        let mut cues = Cues::default();
        assert!(ask_cue(&mut cues, READY_CUE));
        assert!(!ask_cue(&mut cues, SEND_CUE));
        assert_eq!(next_cue(&mut cues).as_deref(), Some(READY_CUE));
        assert!(!ask_cue(&mut cues, READY_CUE));
        assert_eq!(next_cue(&mut cues).as_deref(), Some(SEND_CUE));
        assert_eq!(next_cue(&mut cues).as_deref(), Some(READY_CUE));
        assert_eq!(next_cue(&mut cues), None);
        assert!(ask_cue(&mut cues, SEND_CUE));
        assert_eq!(next_cue(&mut cues).as_deref(), Some(SEND_CUE));
        assert_eq!(next_cue(&mut cues), None);
    }
}
