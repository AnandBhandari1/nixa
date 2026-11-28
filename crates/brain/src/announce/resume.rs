use super::identity::identity;
use super::speak::{event_for, say};
use super::{add_event, space_for, track};
use crate::app::{App, lock};
use crate::util::{log, now_ms};
use crate::spaces::Space;
use nixa_protocol::{CreateSpec, Reply, Request, TermId, TermInfo};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub const LABEL: &str = "resume";
pub const PID_LABEL: &str = "resume.pid";
pub const ENDED: &str = "ended";
pub const OFFER_MS: u64 = 20_000;
pub const SKIP_WORDS: &str = "permission checks off";
pub const AUTO_KEY: &str = "auto-resume";
pub const AUTO_WINDOW_MS: u64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub settle_ms: u64,
    pub gap_ms: u64,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing { settle_ms: 3000, gap_ms: 2000 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub terminal: String,
    pub title: String,
    pub body: String,
    pub command: String,
    pub space: String,
    pub folder: String,
    pub name: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Nothing,
    Mark(String),
    Forget,
    Look { pid: u32, command: String },
    Ended { command: String },
}

pub fn safe_command(command: &str) -> bool {
    nixa_store::agents::safe_resume_command(command)
}

pub fn skips_checks(command: &str) -> bool {
    nixa_store::agents::resume_skips_checks(command)
}

pub fn command_of(info: &TermInfo) -> Option<String> {
    let command = info.labels.get(LABEL)?.trim();
    safe_command(command).then(|| command.to_string())
}

pub fn tidy(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string()
}

pub fn plan(info: &TermInfo, exit: Option<Option<i32>>) -> Plan {
    if info.labels.get(crate::deep::ROLE_LABEL).is_some_and(|r| r == crate::deep::ROLE_VALUE) {
        return Plan::Nothing;
    }
    let Some(command) = command_of(info) else { return Plan::Nothing };
    let marker = info.labels.get(PID_LABEL).map(|m| m.trim().to_string()).unwrap_or_default();
    let code = exit.unwrap_or(info.exit_code);
    if info.running {
        let Some(pid) = info.pid else { return Plan::Nothing };
        if marker.is_empty() {
            return Plan::Mark(pid.to_string());
        }
        if marker == pid.to_string() {
            return Plan::Nothing;
        }
        if exit.is_some() && code == Some(0) {
            return Plan::Forget;
        }
        return Plan::Look { pid, command };
    }
    if marker == ENDED {
        return Plan::Nothing;
    }
    if code == Some(0) {
        return Plan::Forget;
    }
    Plan::Ended { command }
}

pub fn steady(info: &TermInfo, pid: u32, command: &str) -> bool {
    info.running && info.pid == Some(pid) && !info.draft && command_of(info).as_deref() == Some(command)
}

async fn label(app: &App, id: &str, key: &str, value: Option<&str>) -> bool {
    let Ok(term) = crate::prompts::link::term_id(id) else { return false };
    let request = Request::LabelSet { id: term, key: key.to_string(), value: value.map(str::to_string) };
    match app.announce.link().call(request, Duration::from_secs(10)).await {
        Ok(Reply::Error { .. }) | Err(_) => false,
        Ok(_) => true,
    }
}

async fn looks_unused(app: &App, id: &str, pid: u32, command: &str) -> Option<bool> {
    let timing = *lock(&app.announce.timing);
    let link = app.announce.link();
    tokio::time::sleep(Duration::from_millis(timing.settle_ms)).await;
    let first = link.get(id).await.ok()?;
    if !steady(&first, pid, command) {
        return None;
    }
    let before = tidy(&link.read(id).await.ok()?);
    tokio::time::sleep(Duration::from_millis(timing.gap_ms)).await;
    let second = link.get(id).await.ok()?;
    if !steady(&second, pid, command) {
        return None;
    }
    let after = tidy(&link.read(id).await.ok()?);
    Some(before == after)
}

pub async fn check(app: Arc<App>, id: String) {
    let Some(found) = track(&app, &id) else { return };
    match plan(&found.info, found.exit) {
        Plan::Nothing => {}
        Plan::Mark(pid) => {
            label(&app, &id, PID_LABEL, Some(&pid)).await;
        }
        Plan::Forget => {
            label(&app, &id, LABEL, None).await;
            label(&app, &id, PID_LABEL, None).await;
        }
        Plan::Ended { command } => {
            let key = format!("{id}:{ENDED}");
            if !lock(&app.announce.sent).insert(key) {
                return;
            }
            label(&app, &id, PID_LABEL, Some(ENDED)).await;
            stopped(&app, &found.info, &found.agent, &command).await;
        }
        Plan::Look { pid, command } => {
            let key = format!("{id}:{pid}");
            if !lock(&app.announce.sent).insert(key.clone()) {
                return;
            }
            match looks_unused(&app, &id, pid, &command).await {
                Some(unused) => {
                    if !label(&app, &id, PID_LABEL, Some(&pid.to_string())).await {
                        lock(&app.announce.sent).remove(&key);
                        return;
                    }
                    if unused {
                        let info = track(&app, &id).map(|t| t.info).unwrap_or(found.info);
                        stopped(&app, &info, &found.agent, &command).await;
                    }
                }
                None => {
                    lock(&app.announce.sent).remove(&key);
                }
            }
        }
    }
}

pub fn offer_for(app: &App, info: &TermInfo, agent: &str, command: &str) -> Offer {
    let configured = nixa_store::agents::configured_resume_in(&app.dir, command);
    let command = configured.as_deref().unwrap_or(command);
    let space = space_for(app, info);
    let numbers = lock(&app.announce.numbers).clone();
    let who = identity(info, agent, &numbers, space.as_ref().map(|s| s.name.as_str()).unwrap_or_default());
    let job = if who.job.is_empty() { String::new() } else { format!(": {}", who.job) };
    let place = if who.space.is_empty() { String::new() } else { format!("{}: ", who.space) };
    Offer {
        id: format!("resume-{}-{}", info.id, now_ms()),
        terminal: info.id.to_string(),
        title: format!("Resume {}{job}?{}", who.label.to_lowercase(), if skips_checks(command) { format!(" ({SKIP_WORDS})") } else { String::new() }),
        body: format!("{place}it stopped. Click to resume it in a new tab: {command}"),
        command: command.to_string(),
        space: space.map(|s| s.id).unwrap_or_default(),
        folder: info.cwd.clone(),
        name: if who.job.is_empty() { who.label.clone() } else { who.job.clone() },
        timeout_ms: OFFER_MS,
    }
}

pub fn offer(app: &Arc<App>, info: &TermInfo, agent: &str, command: &str) -> Offer {
    let offer = offer_for(app, info, agent, command);
    let track = super::Track { info: info.clone(), agent: agent.to_string(), exit: None };
    let (mut event, _) = event_for(app, &track, "resume", "can be resumed");
    event.offer = Some(offer.clone());
    let words = format!("{}. Click the message to resume it.", event.text);
    add_event(app, event.clone());
    app.send_out(json!({"type": "resume_offer", "offer": offer}));
    log("announce", &format!("offered to resume {}", offer.terminal));
    say(app, &words, &event);
    offer
}

pub fn auto_key(command: &str, now: u64) -> String {
    format!("{AUTO_KEY}:{command}:{}", now / AUTO_WINDOW_MS)
}

pub fn resume_spec(offer: &Offer, space: Option<&Space>) -> CreateSpec {
    let mut spec = CreateSpec {
        cmd: vec!["sh".into(), "-lc".into(), offer.command.clone()],
        cwd: std::path::Path::new(&offer.folder).is_dir().then(|| offer.folder.clone()),
        name: Some(offer.name.clone()).filter(|n| !n.trim().is_empty()),
        ..CreateSpec::default()
    };
    if let Some(space) = space {
        for key in super::SPACE_LABELS {
            spec.labels.insert(key.into(), space.id.clone());
        }
        spec.labels.insert(nixa_protocol::id::SPACE_SLUG_LABEL.into(), space.name.clone());
    }
    spec.labels.insert(LABEL.into(), offer.command.clone());
    spec
}

pub fn again_text(offer: &Offer) -> String {
    format!("Event: {} stopped again after Nixa resumed it once this hour. Do not wait for the user. Decide now: read its screen with nixa_term_read, then start it again with nixa_term_new running {}, start a fresh agent for its work, or ask nixa_deep_ask. Record what you chose in one short line.", offer.terminal, offer.command)
}

pub fn failed_text(offer: &Offer, why: &str) -> String {
    format!("Event: {} stopped and Nixa could not resume it: {why}. Do not wait for the user. Start it again with nixa_term_new running {}, or give its work to another agent. Record what you chose in one short line.", offer.terminal, offer.command)
}

fn tell_brain(app: &Arc<App>, text: &str) {
    if crate::agent::has_brain(app) {
        crate::agent::event(app, text);
    } else {
        crate::modes::effects::carry(app, text.to_string());
    }
}

async fn spawn(app: &App, offer: &Offer, space: Option<&Space>) -> Result<TermId, String> {
    let request = Request::Create { spec: resume_spec(offer, space) };
    match app.announce.link().call(request, Duration::from_secs(10)).await {
        Ok(Reply::Term { term }) => Ok(term.id),
        Ok(Reply::Error { message, .. }) => Err(message),
        Ok(other) => Err(format!("unexpected reply: {other:?}")),
        Err(e) => Err(format!("{e:#}")),
    }
}

pub async fn stopped(app: &Arc<App>, info: &TermInfo, agent: &str, command: &str) {
    let space = space_for(app, info);
    if !crate::settings_api::nixa_on(&app.dir) {
        offer(app, info, agent, command);
        return;
    }
    let made = offer_for(app, info, agent, command);
    let terminal = made.terminal.clone();
    if !lock(&app.announce.sent).insert(auto_key(&made.command, now_ms())) {
        crate::modes::note(app, "resume", &format!("{terminal} stopped again, the brain decides"), Some(terminal.as_str()), Some("resumed once this hour already"));
        tell_brain(app, &again_text(&made));
        return;
    }
    match spawn(app, &made, space.as_ref()).await {
        Ok(id) => {
            crate::modes::note(app, "auto-yes", &format!("resumed {terminal} in {id}: {}", made.command), Some(terminal.as_str()), Some("Nixa resumes a stopped agent"));
            let track = super::Track { info: info.clone(), agent: agent.to_string(), exit: None };
            let (event, _) = event_for(app, &track, "resume", &format!("was resumed in {id}"));
            add_event(app, event.clone());
            log("announce", &format!("resumed {terminal} in {id}"));
            say(app, &event.text, &event);
        }
        Err(why) => {
            log("announce", &format!("could not resume {terminal}: {why}"));
            crate::modes::note(app, "resume", &format!("resume {terminal} by the brain"), Some(terminal.as_str()), Some(&why));
            tell_brain(app, &failed_text(&made, &why));
        }
    }
}
