use super::hold::Pass;
use crate::app::{App, lock};
use crate::util::{log, now_ms};
use nixa_voice::{SayResult, scrub_for_speech};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub const GAP_MS: u64 = 150;
pub const USER_TALK_WAIT_MS: u64 = 15_000;
pub const USER_TALK_POLL_MS: u64 = 100;
const CUT: SayResult = SayResult { interrupted: true, ms: 0 };
const NOTHING: SayResult = SayResult { interrupted: false, ms: 0 };

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SayOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mac_only: bool,
}

impl SayOptions {
    pub fn nixa(kind: &str) -> SayOptions {
        SayOptions { agent: Some("nixa".into()), kind: Some(kind.into()), ..SayOptions::default() }
    }

    pub fn is_nixa(&self) -> bool {
        self.agent.as_deref().is_none_or(|a| a == "nixa")
    }

    fn ctx(&self) -> crate::voice_map::Ctx {
        crate::voice_map::Ctx {
            terminal: self.terminal.clone(),
            model: self.model.clone(),
            agent: Some(self.agent.clone().unwrap_or_else(|| "nixa".into())),
            space: self.space.clone(),
            kind: Some(self.kind.clone().unwrap_or_else(|| if self.is_nixa() { "chat".into() } else { "other".into() })),
        }
    }
}

pub struct PhoneSay {
    pub text: String,
    pub voice: String,
    pub options: SayOptions,
    pub done: oneshot::Sender<SayResult>,
    pub stop: CancellationToken,
}

#[derive(Default)]
pub struct Speaker {
    pub generation: u64,
    pub active: u32,
    pub pending: u32,
    pub line: Option<SayOptions>,
    pub last_ended: u64,
    pub last_said: Option<(String, u64, bool)>,
    pub last_spoke_at: u64,
    pub last_ask_at: u64,
    pub deep_reply_at: u64,
    pub phone: Option<UnboundedSender<PhoneSay>>,
    pub phone_on: bool,
    cut: Option<CancellationToken>,
}

pub fn speaking(app: &App) -> bool {
    lock(&app.talk.speaker).active > 0 || nixa_voice::is_speaking()
}

pub fn busy(app: &App) -> bool {
    let s = lock(&app.talk.speaker);
    s.active > 0 || s.pending > 0 || nixa_voice::is_speaking()
}

pub fn user_talking(app: &App) -> bool {
    let live = {
        let live = lock(&app.voice.live);
        live.user_talking || live.holding
    };
    live || lock(&app.talk.gather).pending()
}

pub fn current(app: &App) -> Option<SayOptions> {
    lock(&app.talk.speaker).line.clone()
}

pub fn last_said(app: &App) -> Option<String> {
    lock(&app.talk.speaker).last_said.as_ref().map(|(text, ..)| text.clone())
}

pub fn last_said_mac(app: &App) -> Option<(String, bool)> {
    lock(&app.talk.speaker).last_said.as_ref().map(|(text, _, mac_only)| (text.clone(), *mac_only))
}

pub fn set_phone(app: &App, sink: Option<UnboundedSender<PhoneSay>>) {
    let mut s = lock(&app.talk.speaker);
    s.phone_on = sink.is_some();
    s.phone = sink;
}

pub fn set_phone_on(app: &App, on: bool) {
    let mut s = lock(&app.talk.speaker);
    s.phone_on = on && s.phone.is_some();
}

async fn wait_for_user(app: &Arc<App>) {
    let until = now_ms() + USER_TALK_WAIT_MS;
    while user_talking(app) && now_ms() < until {
        tokio::time::sleep(Duration::from_millis(USER_TALK_POLL_MS)).await;
    }
}

fn same_generation(app: &App, generation: u64) -> bool {
    lock(&app.talk.speaker).generation == generation
}

fn chat_note(app: &Arc<App>, text: &str, start: bool) {
    use crate::universe::voice_log::{CHAT, END, START};
    crate::universe::voice_note(app, crate::universe::NIXA, text, if start { START } else { END }, CHAT);
}

async fn mac_play(app: &Arc<App>, text: &str, voice: &str, stop: &CancellationToken, nixa: bool) -> SayResult {
    if stop.is_cancelled() {
        return CUT;
    }
    let (speech, done) = if nixa {
        let app = app.clone();
        nixa_voice::speak_with_parts(text, voice, move |part, start| chat_note(&app, part, start))
    } else {
        nixa_voice::speak_cancellable(text, voice)
    };
    let mut done = tokio::task::spawn_blocking(move || done.recv().unwrap_or(CUT));
    tokio::select! {
        result = &mut done => result.unwrap_or(CUT),
        _ = stop.cancelled() => {
            speech.cancel();
            done.await.unwrap_or(CUT)
        }
    }
}

async fn phone_play(sink: UnboundedSender<PhoneSay>, text: &str, voice: &str, options: &SayOptions, stop: CancellationToken) -> SayResult {
    if stop.is_cancelled() {
        return CUT;
    }
    let (done, result) = oneshot::channel();
    let job = PhoneSay { text: text.to_string(), voice: voice.to_string(), options: options.clone(), done, stop };
    if sink.send(job).is_err() {
        log("speech", "phone voice is gone");
        CUT
    } else {
        result.await.unwrap_or(CUT)
    }
}

async fn play_in_turn(app: &Arc<App>, text: &str, options: &SayOptions, generation: u64, pass: Pass, cancel: &CancellationToken) -> Option<SayResult> {
    if !same_generation(app, generation) {
        return Some(CUT);
    }
    let last_ended = lock(&app.talk.speaker).last_ended;
    let gone = now_ms().saturating_sub(last_ended);
    if last_ended > 0 && gone < GAP_MS {
        tokio::time::sleep(Duration::from_millis(GAP_MS - gone)).await;
    }
    let stop = cancel.child_token();
    let phone = {
        let hold = lock(&app.talk.hold);
        if hold.holds(pass) {
            return None;
        }
        let mut s = lock(&app.talk.speaker);
        if s.generation != generation || stop.is_cancelled() {
            return Some(CUT);
        }
        s.line = Some(options.clone());
        s.cut = Some(stop.clone());
        drop(hold);
        if s.phone_on && !options.mac_only { s.phone.clone() } else { None }
    };
    let voice = options.voice.clone().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| crate::voice_map::pick(app, &options.ctx()));
    let via = if phone.is_some() { "phone" } else { "mac" };
    log("speech", &format!("say via {via}, voice {voice}, {} chars", text.chars().count()));
    let nixa = options.is_nixa();
    let result = match phone {
        Some(sink) => {
            if nixa {
                chat_note(app, text, true);
            }
            let result = phone_play(sink, text, &voice, options, stop).await;
            if nixa {
                chat_note(app, text, false);
            }
            result
        }
        None => mac_play(app, text, &voice, &stop, nixa).await,
    };
    {
        let mut s = lock(&app.talk.speaker);
        s.cut = None;
        s.line = None;
        s.last_ended = now_ms();
    }
    Some(result)
}

async fn queued(app: &Arc<App>, text: &str, options: SayOptions, generation: u64, pass: Pass, cancel: &CancellationToken) -> Option<SayResult> {
    lock(&app.talk.speaker).active += 1;
    crate::voice::set_nixa_talking(app, true);
    let result = tokio::select! {
        _ = cancel.cancelled() => Some(CUT),
        _turn = app.talk.chain.lock() => play_in_turn(app, text, &options, generation, pass, cancel).await,
    };
    let idle = {
        let mut s = lock(&app.talk.speaker);
        s.active = s.active.saturating_sub(1);
        s.active == 0
    };
    if idle && !nixa_voice::is_speaking() {
        crate::voice::set_nixa_talking(app, false);
    }
    result
}

pub async fn say(app: &Arc<App>, text: &str, options: SayOptions) -> SayResult {
    say_as(app, text, options, Pass::Notice).await
}

pub fn with_space(app: &App, text: &str, options: &SayOptions) -> String {
    let Some(key) = options.space.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return text.to_string();
    };
    match crate::spaces::name_of(&app.spaces(), key) {
        Some(name) => crate::spaces::named(&name, text),
        None => text.to_string(),
    }
}

pub async fn say_as(app: &Arc<App>, text: &str, options: SayOptions, pass: Pass) -> SayResult {
    let text = &with_space(app, text, &options);
    let clean = scrub_for_speech(text).trim().to_string();
    if clean.chars().count() >= 2 {
        crate::extras::said(app, &clean);
    }
    let result = if clean.chars().count() < 2 { NOTHING } else { loop {
        let held = {
            let mut hold = lock(&app.talk.hold);
            hold.holds(pass).then(|| hold.keep(&clean, &options))
        };
        if let Some(waiting) = held {
            log("hold", "held a spoken line");
            return waiting.await.unwrap_or(CUT);
        }
        if let Some(result) = say_clean(app, &clean, options.clone(), pass, None).await {
            break result;
        }
    } };
    if pass == Pass::Last {
        super::hold::answered(app);
    }
    result
}

pub(super) async fn say_clean(app: &Arc<App>, clean: &str, options: SayOptions, pass: Pass, read_stop: Option<&CancellationToken>) -> Option<SayResult> {
    let unstopped = CancellationToken::new();
    let cancel = read_stop.unwrap_or(&unstopped);
    if cancel.is_cancelled() {
        return Some(CUT);
    }
    let clean = clean.to_string();
    let nixa = read_stop.is_none() && options.is_nixa();
    if nixa {
        lock(&app.talk.speaker).last_said = Some((clean.clone(), now_ms(), options.mac_only));
    }
    let mut result = NOTHING;
    if crate::voice::settings(app).voice_on {
        let generation = {
            let mut s = lock(&app.talk.speaker);
            s.pending += 1;
            s.generation
        };
        tokio::select! {
            _ = wait_for_user(app) => {},
            _ = cancel.cancelled() => {},
        }
        let still = {
            let mut s = lock(&app.talk.speaker);
            s.pending = s.pending.saturating_sub(1);
            s.generation == generation && !cancel.is_cancelled()
        };
        result = if still {
            match queued(app, &clean, options, generation, pass, cancel).await {
                Some(result) => result,
                None => return None,
            }
        } else {
            CUT
        };
        app.send_out(json!({"type": "said", "interrupted": result.interrupted, "ms": result.ms}));
    } else if nixa {
        log("speech", "not said: Nixa speech is off");
    }
    if nixa {
        let now = now_ms();
        let asks = !result.interrupted && crate::voice::asks_user(&clean);
        {
            let mut s = lock(&app.talk.speaker);
            s.last_spoke_at = now;
            if asks {
                s.last_ask_at = now;
            }
        }
    }
    Some(result)
}

pub async fn speak(app: &Arc<App>, text: &str, options: SayOptions) -> Option<SayResult> {
    speak_as(app, text, options, Pass::Notice).await
}

pub async fn speak_as(app: &Arc<App>, text: &str, options: SayOptions, pass: Pass) -> Option<SayResult> {
    let text = &with_space(app, text, &options);
    let now = now_ms();
    let held = {
        let mut p = lock(&app.talk.presence);
        let held = p.quiet(now);
        if held {
            p.hold(now, text, options.mac_only);
        }
        held
    };
    if held {
        log("presence", "held a spoken line");
        super::send_state(app);
        if pass == Pass::Last {
            super::hold::answered(app);
        }
        return None;
    }
    Some(say_as(app, text, options, pass).await)
}

pub fn say_later(app: &Arc<App>, text: &str, options: SayOptions) {
    say_later_as(app, text, options, Pass::Notice);
}

fn keep_now(app: &App, text: &str, options: &SayOptions, pass: Pass) -> bool {
    let clean = scrub_for_speech(&with_space(app, text, options)).trim().to_string();
    if clean.chars().count() < 2 {
        return false;
    }
    let mut hold = lock(&app.talk.hold);
    if !hold.holds(pass) {
        return false;
    }
    drop(hold.keep(&clean, options));
    drop(hold);
    crate::extras::said(app, &clean);
    log("hold", "held a spoken line");
    true
}

fn presence_holds(app: &App) -> bool {
    let now = now_ms();
    let p = lock(&app.talk.presence);
    p.quiet(now)
}

pub fn say_later_as(app: &Arc<App>, text: &str, options: SayOptions, pass: Pass) {
    if keep_now(app, text, &options, pass) {
        return;
    }
    let (app, text) = (app.clone(), text.to_string());
    tokio::spawn(async move {
        say_as(&app, &text, options, pass).await;
    });
}

pub fn speak_later(app: &Arc<App>, text: &str, options: SayOptions) {
    speak_later_as(app, text, options, Pass::Notice);
}

pub fn speak_later_as(app: &Arc<App>, text: &str, options: SayOptions, pass: Pass) {
    if !presence_holds(app) && keep_now(app, text, &options, pass) {
        return;
    }
    let (app, text) = (app.clone(), text.to_string());
    tokio::spawn(async move {
        speak_as(&app, &text, options, pass).await;
    });
}

pub fn barge_in(app: &App) {
    stop(app);
    crate::universe::stop_sound(app);
    crate::announce::hush();
    log("speech", "the user spoke, all Nixa sound stopped");
}

pub fn stop(app: &App) {
    let phone = {
        let mut s = lock(&app.talk.speaker);
        s.generation += 1;
        s.line = None;
        s.cut.take()
    };
    if let Some(stop) = phone {
        stop.cancel();
    }
    nixa_voice::stop_speaking();
    crate::voice::set_nixa_talking(app, false);
}

#[cfg(test)]
mod space_tests {
    use super::*;

    #[test]
    fn a_spoken_line_names_its_space() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let conn = nixa_store::open(&app.db_file).unwrap();
        nixa_store::spaces::set_space(&conn, "s9", &nixa_store::spaces::SpacePatch { name: Some("shop".into()), folder: Some("/work/shop".into()), ..Default::default() }).unwrap();
        let mut options = SayOptions::nixa("chat");
        assert_eq!(with_space(&app, "Agent 3 is done.", &options), "Agent 3 is done.");
        options.space = Some("s9".into());
        assert_eq!(with_space(&app, "Agent 3 is done.", &options), "shop: Agent 3 is done.");
        assert_eq!(with_space(&app, "Agent 3 done: fix in shop", &options), "Agent 3 done: fix in shop");
        options.space = Some("gone".into());
        assert_eq!(with_space(&app, "Agent 3 is done.", &options), "Agent 3 is done.");
    }
}
