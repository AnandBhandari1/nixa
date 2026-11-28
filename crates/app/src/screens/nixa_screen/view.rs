use super::map::{self, Agent, DEEP, Event, Flow, Layout, NIXA, Seen, St};
use super::meteor::{self, Cues};
use super::mode::{self, Feed, Mode, Tone};
use super::orb::{self, Core};
use super::palette::{self, Tones};
use super::office::fun;
use super::office::{call, leave, sync};
use super::office::soap::{self, Gate, Models};
use super::office::talk::{self as office_talk, Line, Stage};
use super::office::{self, Rooms, Tab};
use super::prefs::{self, Look, Prefs};
use super::{aura, galaxy, map_draw, now_ms};
use crate::links::brain::{BrainMsg, Link};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, Entity, EventEmitter, FocusHandle, Hsla, Image, ImageFormat,
    KeyDownEvent, MouseButton, SharedString, Stateful, Subscription, Task, Window, canvas, div, img,
    px, rgb,
};
use nixa_store::Connection;
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, Theme, icon};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub const FRAME: Duration = Duration::from_millis(50);
pub const IDLE_FRAME: Duration = Duration::from_millis(150);
pub const LABEL_COLOR: u32 = 0x7d8794;
pub const ICON_COLOR: u32 = 0x8b95a3;
pub const SOAP_POLL: Duration = Duration::from_millis(200);
pub const SOAP_POLLS: u32 = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenEvent {
    Close,
    Settings,
    TurnNixaOn,
    Show,
    Notice(String),
}

pub struct Live {
    pub level: f32,
    pub target: f32,
    pub mic: f32,
    pub talking_at: Option<Instant>,
    pub flows: Vec<Flow>,
    pub events: Vec<Event>,
    pub seen: Option<Seen>,
    pub deep_was: Option<bool>,
    pub deep_answered_at: u64,
    pub glow: f32,
    pub tick: Instant,
    pub cues: Cues,
    pub dirty: bool,
    pub drawn_at: Option<Instant>,
}

impl Live {
    fn new() -> Live {
        Live {
            level: 0.0,
            target: 0.0,
            mic: 0.0,
            talking_at: None,
            flows: Vec::new(),
            events: Vec::new(),
            seen: None,
            deep_was: None,
            deep_answered_at: 0,
            glow: mode::START_GLOW,
            tick: Instant::now(),
            cues: Cues::default(),
            dirty: true,
            drawn_at: None,
        }
    }
}

pub struct NixaScreen {
    pub open: bool,
    pub prefs: Prefs,
    pub feed: Feed,
    pub all: Vec<Agent>,
    pub shown: Vec<Agent>,
    pub deep: Vec<Agent>,
    pub live: Live,
    pub started: Instant,
    pub mascot: Arc<Image>,
    pub error: Option<String>,
    pub sent: Vec<Value>,
    pub rooms: Rooms,
    pub tab: Option<Tab>,
    pub office: Stage,
    pub demo_rooms: bool,
    pub aspect: std::cell::Cell<f32>,
    pub soap_models: Models,
    pub soap_input: Option<Entity<TextInput>>,
    soap_sub: Option<Subscription>,
    soap_wait: Option<Task<()>>,
    store: Option<Connection>,
    link: Option<Link>,
    rx: Option<Receiver<BrainMsg>>,
    pub focus: FocusHandle,
    focus_next: bool,
    ticker: Option<Task<()>>,
    leave_fetch: bool,
}

impl EventEmitter<ScreenEvent> for NixaScreen {}

pub fn mascot() -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Svg, super::MASCOT.to_vec()))
}

impl NixaScreen {
    pub fn build(link: Option<Link>, store: Option<Connection>, cx: &mut Context<Self>) -> Self {
        let rx = link
            .as_ref()
            .map(|l| l.subscribe(&["level", "universe_flow", "flow", soap::SCENE_EVENT, soap::MODELS_KIND, fun::FUN_EVENT, fun::BANTER_EVENT, sync::KIND, "chat", leave::LEAVE_EVENT, leave::BACK_EVENT]));
        NixaScreen {
            open: false,
            prefs: prefs::load(store.as_ref()),
            feed: Feed::default(),
            all: Vec::new(),
            shown: Vec::new(),
            deep: Vec::new(),
            live: Live::new(),
            started: Instant::now(),
            mascot: mascot(),
            error: None,
            sent: Vec::new(),
            rooms: Rooms::default(),
            tab: None,
            office: Stage::default(),
            demo_rooms: false,
            aspect: std::cell::Cell::new(1.6),
            soap_models: Models::default(),
            soap_input: None,
            soap_sub: None,
            soap_wait: None,
            store,
            link,
            rx,
            focus: cx.focus_handle(),
            focus_next: false,
            ticker: None,
            leave_fetch: true,
        }
    }

    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.drain_stale(Instant::now(), now_ms());
        self.leave_fetch = true;
        self.open = true;
        self.focus_next = true;
        self.live.dirty = true;
        self.ticking(cx);
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.ticking(cx);
        cx.notify();
    }

    pub fn escape(&mut self, cx: &mut Context<Self>) {
        if !self.prefs.keep_open {
            cx.emit(ScreenEvent::Close);
        }
    }

    pub fn request_show(&mut self, cx: &mut Context<Self>) {
        cx.emit(ScreenEvent::Show);
    }

    pub fn set_prefs(&mut self, next: Prefs, cx: &mut Context<Self>) {
        self.prefs = next;
        self.error = prefs::save(self.store.as_ref(), next)
            .err()
            .map(|e| format!("{e:#}"));
        self.live.dirty = true;
        cx.notify();
    }

    pub fn set_keep(&mut self, on: bool, cx: &mut Context<Self>) {
        let next = Prefs {
            keep_open: on,
            ..self.prefs
        };
        self.set_prefs(next, cx);
    }

    pub fn set_quiet(&mut self, on: bool, cx: &mut Context<Self>) {
        let next = Prefs {
            quiet: on,
            ..self.prefs
        };
        self.set_prefs(next, cx);
    }

    pub fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        if self.prefs.look == look {
            return;
        }
        self.office = Stage::default();
        let next = Prefs { look, ..self.prefs };
        self.set_prefs(next, cx);
    }

    pub fn set_still(&mut self, still: bool, cx: &mut Context<Self>) {
        let next = Prefs { still, ..self.prefs };
        self.set_prefs(next, cx);
    }

    pub fn set_mute(&mut self, mute: bool, cx: &mut Context<Self>) {
        let next = Prefs { mute, ..self.prefs };
        self.set_prefs(next, cx);
    }

    pub fn set_soap(&mut self, on: bool, cx: &mut Context<Self>) {
        if !on {
            self.stop_soap(now_ms());
        }
        let next = Prefs { soap: on, ..self.prefs };
        self.set_prefs(next, cx);
    }

    pub fn stop_soap(&mut self, wall: u64) {
        if self.office.soap.show.is_none() {
            return;
        }
        let plan = self.office_plan_now();
        soap::finish(&mut self.office.soap, &mut self.office.talk_end, &plan, wall);
        self.live.dirty = true;
    }

    pub fn ask_soap_models(&mut self, cx: &mut Context<Self>) {
        let sent = self.post(soap::MODELS_KIND, json!({}));
        self.after_soap_post(sent, cx);
    }

    pub fn pick_soap_model(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > soap::MODEL_MAX {
            self.soap_models.error = Some("Type a model name, like the ones in the list.".into());
            cx.notify();
            return;
        }
        let sent = self.post(soap::MODEL_KIND, json!({"model": name}));
        self.after_soap_post(sent, cx);
    }

    fn after_soap_post(&mut self, sent: bool, cx: &mut Context<Self>) {
        if !sent {
            self.soap_models.waiting = false;
            self.soap_models.error = Some(format!("{} is not connected.", self.feed.name()));
            cx.notify();
            return;
        }
        self.soap_models.waiting = true;
        self.soap_models.error = None;
        cx.notify();
        self.soap_wait = Some(cx.spawn(async move |this, cx| {
            for _ in 0..SOAP_POLLS {
                cx.background_executor().timer(SOAP_POLL).await;
                let done = this.update(cx, |s, cx| {
                    s.drain(Instant::now(), now_ms());
                    let done = !s.soap_models.waiting;
                    if done {
                        cx.notify();
                    }
                    done
                });
                if done.unwrap_or(true) {
                    return;
                }
            }
            this.update(cx, |s, cx| {
                s.soap_models.waiting = false;
                s.soap_models.error = Some("The brain did not answer about the models.".into());
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn soap_field(&mut self, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(input) = &self.soap_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(cx, "Another model name, like llama3.2:3b"));
        self.soap_sub = Some(cx.subscribe(&input, |s, _, e: &InputEvent, cx| {
            if let InputEvent::Submit(name) = e {
                let name = name.clone();
                s.pick_soap_model(&name, cx);
            }
        }));
        self.soap_input = Some(input.clone());
        input
    }

    pub fn set_tab(&mut self, tab: Option<Tab>, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.office.now = None;
            self.stop_soap(now_ms());
            self.office.soap.asked_at = None;
            self.live.dirty = true;
            cx.notify();
        }
    }

    pub fn set_rooms(&mut self, rooms: Rooms) {
        if !self.demo_rooms && rooms != self.rooms {
            self.rooms = rooms;
            self.live.dirty = true;
        }
    }

    pub fn office_plan(&self) -> office::plan::Plan {
        let tab = office::shown_tab(self.tab.as_ref(), &self.rooms);
        let list = office::rooms_of(&self.rooms, &tab);
        office::plan::build(&list, &self.rooms.loose_deep, self.feed.name(), self.aspect.get() < 1.0)
    }

    pub fn office_plan_now(&self) -> office::plan::Plan {
        let mut plan = self.office_plan();
        let cto = office::view::cto_status(self.real_mode(Instant::now()));
        if let Some(p) = plan.people.iter_mut().find(|p| p.id == NIXA) {
            p.status = cto;
        }
        plan
    }

    pub fn add_talk(&mut self, from: &str, to: &str, kind: Option<&str>, text: Option<&str>, at: u64) {
        if self.prefs.look != Look::Office || from.is_empty() || to.is_empty() || from == to {
            return;
        }
        let talk = office_talk::talk_of(from, to, kind, text, &self.all, at);
        fun::drop_banter(&mut self.office);
        office_talk::push(&mut self.office, talk);
        self.stop_soap(now_ms());
        self.ask_fun(now_ms());
        self.live.dirty = true;
    }

    fn ask_fun(&mut self, wall: u64) {
        let gate = self.soap_gate();
        if !fun::on(gate) {
            return;
        }
        let plan = self.office_plan_now();
        for (id, body) in fun::asks(&mut self.office, &plan, &self.all, gate, wall) {
            if !self.post(fun::FUN_KIND, body) {
                fun::unask(&mut self.office, id);
            }
        }
    }

    fn fun_step(&mut self, plan: &office::plan::Plan, wall: u64) {
        let gate = self.soap_gate();
        if let Some(body) = fun::gate_msg(&mut self.office.fun, gate, wall) {
            self.post(fun::GATE_KIND, body);
        }
        fun::schedule(&mut self.office.fun, gate, wall);
        let show = self.office.soap.show.is_some();
        if !fun::banter_due(&self.office.fun, gate, show, wall) {
            return;
        }
        self.office.fun.banter_at = wall + fun::gap(wall);
        let cast = leave::cast_plan(plan, &self.office.leave);
        if let Some(body) = fun::banter_pick(&cast, &self.all, gate, wall)
            && self.post(fun::BANTER_KIND, body)
        {
            self.office.fun.banter_asked = Some(wall);
        }
    }

    fn take_banter(&mut self, msg: &Value, wall: u64) -> bool {
        let asked = self.office.fun.banter_asked.take().is_some();
        let gate = self.soap_gate();
        if !asked || !fun::on(gate) || gate.idle || gate.talk || self.office.soap.show.is_some() {
            return false;
        }
        let plan = leave::cast_plan(&self.office_plan_now(), &self.office.leave);
        match fun::banter_talk(msg, &plan, wall) {
            Some(talk) => {
                office_talk::push(&mut self.office, talk);
                true
            }
            None => false,
        }
    }

    fn take_voice(&mut self, msg: &Value, wall: u64) -> bool {
        let Some(h) = sync::heard(msg) else {
            return false;
        };
        if h.chat {
            return h.who == NIXA && call::voice(&mut self.office.call, &h.text, &h.state, wall);
        }
        sync::on_event(&mut self.office.voice, &h, wall)
    }

    pub fn office_open(&self) -> bool {
        self.open && self.prefs.look == Look::Office
    }

    fn take_chat(&mut self, msg: &Value, wall: u64) -> bool {
        let Some(item) = msg.get("item") else {
            return false;
        };
        let text = item.get("text").and_then(Value::as_str).unwrap_or("");
        match item.get("who").and_then(Value::as_str) {
            Some("user") if self.office_open() => {
                if !call::user(&mut self.office.call, text, wall) {
                    return false;
                }
                if let Some(p) = self.office.now.take() {
                    self.office.queue.insert(0, p.talk);
                }
                fun::drop_banter(&mut self.office);
                self.stop_soap(wall);
                true
            }
            Some("nixa") => call::reply(&mut self.office.call, text, wall),
            _ => false,
        }
    }

    fn take_leave(&mut self, msg: &Value, back: bool, wall: u64) -> bool {
        leave::take(&mut self.office.leave, msg, back, wall)
    }

    pub fn soap_gate(&self) -> Gate {
        let real = self.real_mode(Instant::now());
        Gate {
            office: self.prefs.look == Look::Office,
            open: self.open,
            on: self.prefs.soap,
            up: self.feed.up,
            idle: real.idle() && office::view::cto_status(real) != St::Working,
            talk: self.office.now.is_some() || !self.office.queue.is_empty() || call::on(&self.office.call, now_ms()),
        }
    }

    fn take_scene(&mut self, msg: &Value, wall: u64) -> bool {
        let asked = self.office.soap.asked_at.take().is_some();
        let gate = self.soap_gate();
        let failed = msg.get("error").is_some_and(|e| !e.is_null());
        let busy = !asked || gate.talk || !gate.office || !gate.on || !gate.idle || !gate.open || !gate.up;
        let scene = if failed || busy || self.office.soap.show.is_some() {
            None
        } else {
            let plan = leave::cast_plan(&self.office_plan_now(), &self.office.leave);
            let busy = office::view::busy_of(self, &plan);
            let busy: Vec<&str> = busy.iter().map(String::as_str).collect();
            let life = office::npc::life_after(&plan, wall, &busy, &self.office.talk_end);
            soap::scene_of(msg, &plan).and_then(|sc| soap::start(sc, &plan, &life, wall))
        };
        match scene {
            Some(show) => {
                self.office.soap.show = Some(show);
                self.live.dirty = true;
                true
            }
            None => {
                self.office.soap.ended_at = Some(wall);
                false
            }
        }
    }

    fn take_models(&mut self, msg: &Value) -> bool {
        let text = |k: &str| msg.get(k).and_then(Value::as_str).map(str::to_string);
        let models = msg
            .get("models")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        self.soap_models = Models {
            model: text("model").unwrap_or_default(),
            url: text("url").unwrap_or_default(),
            models,
            error: text("error").filter(|e| !e.trim().is_empty()),
            waiting: false,
            known: true,
        };
        true
    }

    fn soap_step(&mut self, plan: &office::plan::Plan, wall: u64) {
        soap::expire(&mut self.office.soap, wall);
        let gate = self.soap_gate();
        if self.office.soap.show.is_some() {
            let stop = gate.talk || !gate.idle || !gate.on || !gate.office;
            let over = self.office.soap.show.as_ref().is_some_and(|s| soap::is_over(s, wall));
            if stop || over {
                soap::finish(&mut self.office.soap, &mut self.office.talk_end, plan, wall);
                self.live.dirty = true;
                return;
            }
            let close = if self.prefs.still {
                0.0
            } else {
                self.office.soap.show.as_ref().map(|s| soap::dolly(s, wall)).unwrap_or(0.0)
            };
            let line = self.office.soap.show.as_mut().and_then(|s| soap::due(s, wall));
            if let Some((who, text)) = line {
                let voiced = soap::speak_ok(self.prefs.still, self.prefs.mute, self.feed.up, close)
                    && self.post("office_say", json!({"who": who, "text": text}));
                sync::post(&mut self.office.voice, &who, &text, wall, voiced);
            }
            if let (Some(show), Some(line)) = (self.office.soap.show.as_mut(), self.office.voice.line.as_ref()) {
                soap::hold(show, line, wall);
            }
            return;
        }
        if !soap::should_ask(&self.office.soap, gate, wall) {
            return;
        }
        let life = office::npc::life_after(plan, wall, &[], &self.office.talk_end);
        let cast = leave::cast_plan(plan, &self.office.leave);
        if let Some(body) = soap::payload(&cast, &life, &self.all, soap::local_hour(), wall)
            && self.post(soap::KIND, body)
        {
            self.office.soap.asked_at = Some(wall);
        } else {
            self.office.soap.ended_at = Some(wall);
        }
    }

    fn office_step(&mut self, wall: u64) -> bool {
        if self.prefs.look != Look::Office {
            return false;
        }
        let plan = self.office_plan_now();
        office_talk::see(&mut self.office, &plan, wall);
        let on_call = call::on(&self.office.call, wall);
        if !on_call {
            office_talk::advance(&mut self.office, &plan, wall);
        }
        let speak = !self.prefs.still && !self.prefs.mute;
        let line = self
            .office
            .now
            .as_mut()
            .filter(|_| !on_call)
            .and_then(|p| office_talk::due_line(p, wall));
        if let Some(line) = line {
            let (who, text) = match line {
                Line::First(who, text) | Line::Second(who, text) => (who, text),
            };
            let who = office::plan::find(&plan, &who)
                .map(|p| p.id.clone())
                .unwrap_or(who);
            let voiced = speak && self.feed.up && self.post("office_say", json!({"who": who, "text": text}));
            sync::post(&mut self.office.voice, &who, &text, wall, voiced);
        }
        if let (Some(p), Some(line)) = (self.office.now.as_mut(), self.office.voice.line.as_ref()) {
            office_talk::hold(p, line, wall);
        }
        leave::remember(&mut self.office.leave, &plan, wall);
        leave::prune(&mut self.office.leave, &plan, wall);
        let calling = office::call::dolly(&self.office.call, wall) > 0.0;
        let leaving = !self.office.leave.trips.is_empty();
        let typing = plan.people.iter().any(|p| p.status == St::Working);
        let typing = typing || office::npc::life(&plan, wall, &[]).iter().any(Option::is_some);
        let moment = office_talk::fresh(&self.office.seen, wall);
        self.soap_step(&plan, wall);
        self.fun_step(&plan, wall);
        let soap = self.office.soap.show.is_some();
        self.office.now.is_some() || !self.office.queue.is_empty() || typing || moment || soap || calling || leaving
    }

    pub fn wanted(&self) -> bool {
        self.open
    }

    fn ticking(&mut self, cx: &mut Context<Self>) {
        if !self.wanted() {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.live.tick = Instant::now();
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FRAME).await;
                if this.update(cx, |screen, cx| screen.frame(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    pub fn set_feed(&mut self, feed: Feed) {
        if feed != self.feed {
            self.feed = feed;
            self.live.dirty = true;
        }
    }

    pub fn deep_working(&self) -> bool {
        self.deep.iter().any(|a| a.status == St::Working)
    }

    pub fn set_agents(&mut self, all: Vec<Agent>, deep: Vec<Agent>, now: u64) {
        let shown = map::pick_shown(&all);
        let mut events = std::mem::take(&mut self.live.events);
        events.extend(map::agent_events(self.live.seen.as_ref(), &all, now));
        map::live_events(&mut events, now);
        self.live.events = events;
        let (seen, flows) = map::agent_flows(
            self.live.seen.as_ref(),
            &all,
            now,
            self.live.deep_answered_at,
        );
        self.live.seen = Some(seen);
        let talks: Vec<(String, String, u64)> = flows
            .iter()
            .map(|f| (f.from.clone(), f.to.clone(), f.at))
            .collect();
        self.live.flows.extend(flows);
        let working = deep.iter().any(|a| a.status == St::Working);
        if shown != self.shown || deep != self.deep {
            self.live.dirty = true;
        }
        self.all = all;
        self.shown = shown;
        self.deep = deep;
        for (from, to, at) in talks {
            self.add_talk(&from, &to, None, None, at);
        }
        self.note_deep(working, now);
        map::live_flows(&mut self.live.flows, now);
    }

    fn note_deep(&mut self, working: bool, now: u64) {
        if let Some(flow) = map::deep_flow(self.live.deep_was, working, now) {
            if flow.from == DEEP {
                self.live.deep_answered_at = now;
            }
            self.add_talk(&flow.from.clone(), &flow.to.clone(), None, None, now);
            self.live.flows.push(flow);
            self.live.dirty = true;
        }
        self.live.deep_was = Some(working);
    }

    pub fn on_message(&mut self, msg: &Value, now: Instant, wall: u64) -> bool {
        match msg.get("type").and_then(Value::as_str) {
            Some("level") => {
                let mic = msg.get("mic").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                self.live.target = if mic.is_finite() {
                    mic.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if msg.get("talking").and_then(Value::as_bool) == Some(true) {
                    self.live.talking_at = Some(now);
                }
                false
            }
            Some("flow") => {
                let from = msg.get("from").and_then(Value::as_str).map(meteor::node_of);
                let to = msg.get("to").and_then(Value::as_str).map(meteor::node_of);
                let (Some(from), Some(to)) = (from, to) else {
                    return false;
                };
                if from.is_empty() || to.is_empty() || from == to {
                    return false;
                }
                let kind = msg.get("kind").and_then(Value::as_str);
                let text = msg.get("text").and_then(Value::as_str);
                self.add_talk(from, to, kind, text, wall);
                self.live.flows.push(Flow {
                    from: from.into(),
                    to: to.into(),
                    at: wall,
                });
                map::live_flows(&mut self.live.flows, wall);
                true
            }
            Some("universe_flow") => {
                let from = msg
                    .get("from")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty());
                let to = msg
                    .get("to")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty());
                let at = msg.get("at").and_then(Value::as_u64);
                let (Some(from), Some(to), Some(at)) = (from, to, at) else {
                    return false;
                };
                let text = msg.get("text").and_then(Value::as_str);
                self.add_talk(from, to, None, text, at);
                self.live.flows.push(Flow {
                    from: from.into(),
                    to: to.into(),
                    at,
                });
                map::live_flows(&mut self.live.flows, wall);
                true
            }
            Some(soap::SCENE_EVENT) => self.take_scene(msg, wall),
            Some(soap::MODELS_KIND) => self.take_models(msg),
            Some(fun::FUN_EVENT) => fun::apply(&mut self.office, msg),
            Some(fun::BANTER_EVENT) => self.take_banter(msg, wall),
            Some(sync::KIND) => self.take_voice(msg, wall),
            Some("chat") => self.take_chat(msg, wall),
            Some(leave::LEAVE_EVENT) => self.take_leave(msg, false, wall),
            Some(leave::BACK_EVENT) => self.take_leave(msg, true, wall),
            _ => false,
        }
    }

    fn drain(&mut self, now: Instant, wall: u64) {
        let Some(rx) = self.rx.take() else {
            return;
        };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BrainMsg::Event(v) => {
                    if self.on_message(&v, now, wall) {
                        self.live.dirty = true;
                    }
                }
                BrainMsg::Down(_) => {
                    self.live.target = 0.0;
                    self.live.dirty = true;
                }
                BrainMsg::Up(_) => {
                    self.leave_fetch = true;
                    self.live.dirty = true;
                }
            }
        }
        self.rx = Some(rx);
    }

    fn drain_stale(&mut self, now: Instant, wall: u64) {
        let Some(rx) = self.rx.take() else {
            return;
        };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BrainMsg::Event(v) => {
                    self.on_stale(&v, now, wall);
                }
                BrainMsg::Down(_) => self.live.target = 0.0,
                BrainMsg::Up(_) => self.leave_fetch = true,
            }
        }
        self.rx = Some(rx);
        self.live.dirty = true;
    }

    fn on_stale(&mut self, msg: &Value, now: Instant, wall: u64) -> bool {
        match msg.get("type").and_then(Value::as_str) {
            Some("chat") | Some(sync::KIND) => false,
            Some(leave::LEAVE_EVENT) => leave::stale(&mut self.office.leave, msg, false),
            Some(leave::BACK_EVENT) => leave::stale(&mut self.office.leave, msg, true),
            _ => self.on_message(msg, now, wall),
        }
    }

    fn fetch_leave(&mut self, cx: &mut Context<Self>) {
        if !self.leave_fetch || !self.feed.up {
            return;
        }
        let Some(link) = self.link.clone() else {
            self.leave_fetch = false;
            return;
        };
        self.leave_fetch = false;
        let task = cx.background_executor().spawn(async move { link.get("/agents/leave") });
        cx.spawn(async move |this, cx| {
            let Ok(reply) = task.await else {
                return;
            };
            this.update(cx, |screen, cx| {
                if leave::seed(&mut screen.office.leave, &leave::away_of(&reply)) {
                    screen.live.dirty = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn talking(&self, now: Instant) -> bool {
        self.live
            .talking_at
            .is_some_and(|at| now.duration_since(at).as_millis() < mode::TALK_HOLD_MS)
    }

    pub fn real_mode(&self, now: Instant) -> Mode {
        mode::mode_of(&self.feed, self.talking(now))
    }

    pub fn time(&self) -> f32 {
        if self.prefs.quiet {
            0.0
        } else {
            self.started.elapsed().as_secs_f32()
        }
    }

    pub fn step(&mut self, now: Instant, wall: u64) -> bool {
        self.drain(now, wall);
        let real = self.real_mode(now);
        let (shown, _) = mode::shown_of(real, self.deep_working());
        let goal = if real == Mode::You {
            self.live.target
        } else {
            0.0
        };
        self.live.level = mode::ease(self.live.level, goal);
        let heard = if real.hearing() {
            self.live.target
        } else {
            0.0
        };
        self.live.mic = mode::ease(self.live.mic, heard);
        let ids: Vec<String> = self.shown.iter().map(|a| a.id.clone()).collect();
        if let Some(cue) = meteor::cue(&mut self.live.cues, &self.live.flows, wall, &ids)
            && self.feed.up
        {
            self.post(meteor::KIND, cue);
        }
        let dt = now.duration_since(self.live.tick).as_secs_f32().min(0.1);
        self.live.tick = now;
        self.live.glow = mode::next_glow(self.live.glow, shown, dt);
        map::live_flows(&mut self.live.flows, wall);
        map::live_events(&mut self.live.events, wall);
        let busy = self.office_step(wall);
        let calm = !busy
            && shown.idle()
            && self.live.level < 0.05
            && self.live.mic < 0.05
            && self.live.flows.is_empty()
            && self.live.events.is_empty();
        let recent = self
            .live
            .drawn_at
            .is_some_and(|at| now.duration_since(at) < IDLE_FRAME);
        if calm && !self.live.dirty && recent {
            return false;
        }
        self.live.dirty = false;
        self.live.drawn_at = Some(now);
        true
    }

    pub fn frame(&mut self, cx: &mut Context<Self>) {
        if !self.wanted() {
            self.ticker = None;
            return;
        }
        if self.step(Instant::now(), now_ms()) {
            cx.notify();
        }
        self.fetch_leave(cx);
    }

    fn post(&mut self, kind: &str, fields: Value) -> bool {
        let sent = self
            .link
            .as_ref()
            .is_some_and(|l| l.send(kind, fields.clone()).is_ok());
        if sent || self.link.is_none() {
            let mut msg = fields;
            if let Some(map) = msg.as_object_mut() {
                map.insert("type".into(), json!(kind));
            }
            self.sent.push(msg);
            self.sent.truncate(64);
        }
        sent
    }

    pub fn click_core(&mut self, cx: &mut Context<Self>) {
        let (kind, fields) = match self.real_mode(Instant::now()) {
            Mode::Off | Mode::Paused if self.feed.up => ("resume", json!({})),
            Mode::Off | Mode::Paused => {
                cx.emit(ScreenEvent::TurnNixaOn);
                return;
            }
            Mode::MicOff => ("set_mic", json!({"on": true})),
            Mode::Nixa | Mode::Think => ("stop", json!({})),
            _ => return,
        };
        if !self.post(kind, fields) {
            cx.emit(ScreenEvent::Notice(format!(
                "{} is not connected.",
                self.feed.name()
            )));
        }
    }
}

pub struct Scene {
    pub layout: Layout,
    pub real: Mode,
    pub mode: Mode,
    pub tones: Tones,
    pub core: Core,
    pub glow: f32,
    pub time: f32,
    pub now: u64,
    pub quiet: bool,
    pub senders: [Hsla; 3],
    pub flows: Vec<Flow>,
    pub events: Vec<Event>,
    pub name: String,
}

pub fn scene_of(screen: &NixaScreen, w: f32, h: f32, t: &Theme) -> Scene {
    let now = Instant::now();
    let real = screen.real_mode(now);
    let (mode, tone) = mode::shown_of(real, screen.deep_working());
    let tones = palette::palette_of(tone, t);
    let quiet = screen.prefs.quiet;
    let time = screen.time();
    let layout = map::layout(w, h, &screen.shown, &screen.deep);
    let level = match mode {
        Mode::You => screen.live.level,
        Mode::Nixa => 0.6,
        _ => 0.0,
    };
    let core = Core {
        size: layout.nixa.size,
        t: time,
        mode,
        level,
        mic: (real != Mode::Off).then_some(screen.live.mic),
        tones,
        breathing: !quiet && !mode.animated(),
    };
    let glow = if quiet {
        mode::START_GLOW
    } else {
        screen.live.glow + mode::energy_of(mode, time, screen.live.level) * 0.5
    };
    Scene {
        layout,
        real,
        mode,
        tones,
        core,
        glow,
        time,
        now: now_ms(),
        quiet,
        senders: [
            palette::palette_of(Tone::Mode(Mode::Nixa), t).main,
            palette::palette_of(Tone::Deep, t).main,
            palette::palette_of(Tone::Agent, t).main,
        ],
        flows: screen.live.flows.clone(),
        events: screen.live.events.clone(),
        name: screen.feed.name().to_string(),
    }
}

fn brain_core(brain: &map::Brain, time: f32, quiet: bool) -> Core {
    let mode = if brain.agent.status == St::Working {
        Mode::Think
    } else {
        Mode::Standby
    };
    let c = brain.color;
    Core {
        size: brain.spot.size,
        t: time,
        mode,
        level: 0.0,
        mic: None,
        tones: Tones {
            main: c,
            second: c,
            third: c,
        },
        breathing: !quiet && mode != Mode::Think,
    }
}

fn layer(el: gpui::Canvas<()>, w: f32, h: f32) -> gpui::Canvas<()> {
    el.absolute().top_0().left_0().w(px(w)).h(px(h))
}

fn back_layer(s: &Scene, w: f32, h: f32) -> gpui::Canvas<()> {
    let layout = s.layout.clone();
    let core = s.core;
    let tones = s.tones;
    let glow = s.glow;
    let (time, quiet) = (s.time, s.quiet);
    layer(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                galaxy::paint(window, o, w, h);
                let m = layout.nixa;
                aura::paint(window, o, (m.x, m.y, m.size), glow, &tones);
                map_draw::links(window, o, &layout, tones.second);
                for brain in &layout.brains {
                    let s = brain.spot;
                    orb::paint(
                        window,
                        orb::origin(o, s.x, s.y, s.size),
                        &brain_core(brain, time, quiet),
                    );
                }
                orb::paint(window, orb::origin(o, m.x, m.y, m.size), &core);
            },
        ),
        w,
        h,
    )
}

fn front_layer(s: &Scene, w: f32, h: f32, t: &Theme) -> gpui::Canvas<()> {
    let layout = s.layout.clone();
    let theme = t.clone();
    let (time, quiet, now) = (s.time, s.quiet, s.now);
    let senders = s.senders;
    let flows = s.flows.clone();
    let events = s.events.clone();
    layer(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                map_draw::nodes(window, o, &layout, &theme, time, quiet);
                map_draw::events(window, o, &layout, &events, now);
                let color_of = |who: &str| match who {
                    NIXA => senders[0],
                    DEEP => senders[1],
                    _ => senders[2],
                };
                map_draw::flows(window, o, &layout, &flows, now, &color_of);
            },
        ),
        w,
        h,
    )
}

pub fn sway(s: &Scene) -> (f32, f32, f32) {
    let size = s.core.size;
    if s.quiet || s.real == Mode::Off {
        return (size * 0.29, size * 0.27, size * 0.42);
    }
    let t = s.time;
    let think = s.mode == Mode::Think;
    let left = size * (0.29 + (t * 0.65).sin() * 0.003);
    let top = size * 0.27
        + (t * if think { 1.8 } else { 1.1 }).sin() * size * if think { 0.009 } else { 0.005 };
    (left, top, size * (0.42 + (t * 1.1).sin() * 0.002))
}

pub fn core_tip(s: &Scene) -> String {
    if matches!(s.real, Mode::Off | Mode::Paused) {
        format!("{} is off. Click to turn {} on.", s.name, s.name)
    } else if s.mode == Mode::Think {
        format!("{} is thinking", s.name)
    } else {
        format!("{}, your galaxy companion", s.name)
    }
}

pub fn mascot_el(image: Arc<Image>, s: &Scene, (x, y): (f32, f32)) -> impl IntoElement {
    let (left, top, size) = sway(s);
    let half = s.core.size / 2.0;
    img(image)
        .absolute()
        .left(px(x - half + left))
        .top(px(y - half + top))
        .size(px(size))
}

pub fn core_hit(
    entity: &Entity<NixaScreen>,
    s: &Scene,
    (x, y): (f32, f32),
    id: &'static str,
) -> Stateful<Div> {
    let size = s.core.size;
    let screen = entity.clone();
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .absolute()
        .left(px(x - size / 2.0))
        .top(px(y - size / 2.0))
        .size(px(size))
        .rounded_full()
        .cursor_pointer()
        .tooltip(tooltip(core_tip(s)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| screen.update(cx, |v, cx| v.click_core(cx)))
}

pub fn mode_word(s: &Scene, (x, y): (f32, f32)) -> impl IntoElement {
    div()
        .absolute()
        .left(px(x - 120.0))
        .top(px(y + s.core.size / 2.0 + 6.0))
        .w(px(240.0))
        .text_center()
        .font_family("Menlo")
        .text_size(px(12.0))
        .text_color(s.tones.main)
        .opacity(0.85)
        .child(s.mode.word())
}

fn mono_of(agent: &Agent) -> SharedString {
    let m = agent.mono.trim();
    if m.is_empty() {
        agent
            .program
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "·".into())
            .into()
    } else {
        m.to_string().into()
    }
}

pub fn agent_tip(agent: &Agent, placed_parent: bool) -> String {
    let mut tip = format!("{} · {}", agent.name(), agent.status.word());
    if let Some(parent) = &agent.parent {
        tip.push_str(&format!(" · parent {parent}"));
        if !placed_parent {
            tip.push_str(" unavailable");
        }
    }
    tip
}

fn map_elements(s: &Scene) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for brain in &s.layout.brains {
        let sp = brain.spot;
        let size = sp.size * 0.32;
        let state = if brain.agent.status == St::Working {
            "thinking"
        } else {
            "standby"
        };
        out.push(
            div()
                .id(SharedString::from(format!("nixa-deep-{}", brain.agent.id)))
                .debug_selector({
                    let id = format!("nixa-deep-{}", brain.agent.id);
                    move || id
                })
                .absolute()
                .left(px(sp.x - size / 2.0))
                .top(px(sp.y - size / 2.0))
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(aura::BASE))
                .border_1()
                .border_color(brain.color.opacity(0.6))
                .text_size(px((size * 0.5).max(9.0)))
                .text_color(brain.color)
                .tooltip(tooltip(format!(
                    "Deep brain · {} · {state}",
                    brain.agent.label()
                )))
                .child(mono_of(&brain.agent))
                .into_any_element(),
        );
    }
    for placed in &s.layout.agents {
        let kid = placed.parent.is_some();
        let size = if kid { 14.0 } else { 20.0 };
        out.push(
            div()
                .id(SharedString::from(format!(
                    "universe-agent-{}",
                    placed.agent.id
                )))
                .debug_selector({
                    let id = format!("universe-agent-{}", placed.agent.id);
                    move || id
                })
                .absolute()
                .left(px(placed.x - size / 2.0))
                .top(px(placed.y - size / 2.0))
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(if kid { 8.0 } else { 10.0 }))
                .text_color(rgb(0xd8dee9))
                .tooltip(tooltip(agent_tip(&placed.agent, kid)))
                .child(mono_of(&placed.agent))
                .into_any_element(),
        );
        let width = if kid { 96.0 } else { 180.0 };
        out.push(
            div()
                .absolute()
                .left(px(placed.x - width / 2.0))
                .top(px(placed.y + placed.radius + 10.0))
                .w(px(width))
                .flex()
                .justify_center()
                .text_size(px(10.0))
                .text_color(rgb(LABEL_COLOR))
                .child(div().truncate().child(placed.agent.label()))
                .into_any_element(),
        );
    }
    out
}

pub fn scene(
    screen: &NixaScreen,
    entity: &Entity<NixaScreen>,
    (w, h): (f32, f32),
    t: &Theme,
    gap: f32,
) -> Div {
    if screen.prefs.look == Look::Office {
        return div()
            .relative()
            .w(px(w))
            .h(px(h))
            .child(office::view::scene(screen, entity, (w, h), t, gap));
    }
    let s = scene_of(screen, w, h, t);
    let at = (s.layout.nixa.x, s.layout.nixa.y);
    div()
        .relative()
        .w(px(w))
        .h(px(h))
        .overflow_hidden()
        .bg(rgb(aura::BASE))
        .child(back_layer(&s, w, h))
        .child(mascot_el(screen.mascot.clone(), &s, at))
        .child(mode_word(&s, at))
        .child(front_layer(&s, w, h, t))
        .children(map_elements(&s))
        .child(core_hit(entity, &s, at, "nixa-core"))
        .child(
            div()
                .absolute()
                .top(px(8.0))
                .left(px(8.0 + office::view::WINDOW_BUTTONS))
                .child(office::view::look_switch(entity, Look::Universe, t)),
        )
}

fn icon_button(id: &'static str, which: Icon, tip: String) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .size(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(gpui::rgba(0xffffff14)))
        .tooltip(tooltip(tip))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(which).size(px(16.0)).text_color(rgb(ICON_COLOR)))
}

impl Render for NixaScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_next {
            window.focus(&self.focus);
            self.focus_next = false;
        }
        let t = nixa_ui::theme(cx).clone();
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let entity = cx.entity();
        let name = self.feed.name().to_string();
        div()
            .id("nixa-screen")
            .debug_selector(|| "nixa-screen".into())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgb(aura::BASE))
            .overflow_hidden()
            .occlude()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|screen, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    screen.escape(cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|screen, _, window, cx| {
                    window.focus(&screen.focus);
                    cx.stop_propagation();
                }),
            )
            .child(scene(self, &entity, (w, h), &t, 80.0))
            .child(
                div()
                    .absolute()
                    .top(px(10.0))
                    .right(px(12.0))
                    .flex()
                    .gap(px(4.0))
                    .child(
                        icon_button(
                            "nixa-screen-gear",
                            Icon::Settings,
                            format!("{name} screen settings"),
                        )
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ScreenEvent::Settings))),
                    )
                    .child(
                        icon_button(
                            "nixa-screen-close",
                            Icon::X,
                            format!("Close the {name} screen"),
                        )
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ScreenEvent::Close))),
                    ),
            )
    }
}
