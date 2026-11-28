use crate::layout;
use crate::links::brain::{self, BrainMsg, Link};
use crate::links::procs::{self, Cmd, Config, Kind, Level, Procs, State};
use crate::screens::agents::{self, AgentsEvent, AgentsView};
use crate::screens::chat::model::Link as ChatLink;
use crate::screens::chat::{self, ChatEvent, ChatView, Wire};
use crate::screens::events::{self, Action, EventsEvent, ToastEvent};
use crate::screens::question::{self, QuestionEvent, QuestionView};
use crate::workspace::Workspace;
use gpui::{Context, Entity, Hsla, Subscription};
use nixa_ui::Theme;
use serde_json::{Value, json};
use nixa_protocol::TermId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, Sender};

pub struct Hub {
    pub agents: Entity<AgentsView>,
    pub chat: Entity<ChatView>,
    pub question: Entity<QuestionView>,
    pub events: events::Screens,
    pub procs: Option<Procs>,
    pub procs_cfg: Option<Config>,
    procs_cmd: Option<Sender<Cmd>>,
    procs_rx: Option<Receiver<Procs>>,
    pub link: Option<Link>,
    link_rx: Option<Receiver<BrainMsg>>,
    numbers_sent: Option<BTreeMap<TermId, u32>>,
    _subs: Vec<Subscription>,
}

pub fn new(
    cx: &mut Context<Workspace>,
    notice: &mut Option<String>,
    wake: futures::channel::mpsc::UnboundedSender<()>,
) -> Hub {
    let chat = chat::new(cx);
    let question = question::new(cx);
    chat::set_question(&chat, question.clone(), cx);
    let screens = events::new(cx);
    let agents = agents::new(cx);
    let subs = vec![
        cx.subscribe(&agents, |ws, _, event: &AgentsEvent, cx| {
            agents::on_event(ws, event, cx)
        }),
        cx.subscribe(&screens.events, |ws, _, event: &EventsEvent, cx| {
            on_events(ws, event, cx)
        }),
        cx.subscribe(&screens.toasts, |ws, _, event: &ToastEvent, cx| {
            on_toast(ws, event, cx)
        }),
        cx.subscribe(&chat, |ws, _, event: &ChatEvent, cx| on_chat(ws, event, cx)),
        cx.subscribe(&question, |ws, _, event: &QuestionEvent, cx| {
            on_question(ws, event, cx)
        }),
        cx.on_release(|ws, _| {
            if let Some(link) = &ws.hub.link {
                link.stop();
            }
        }),
    ];
    let mut hub = Hub {
        agents,
        chat,
        question,
        events: screens,
        procs: None,
        procs_cfg: None,
        procs_cmd: None,
        numbers_sent: None,
        procs_rx: None,
        link: None,
        link_rx: None,
        _subs: subs,
    };
    if cfg!(test) {
        return hub;
    }
    match procs::config() {
        Ok(cfg) => {
            if let Err(e) = procs::open_log(&cfg) {
                *notice = Some(format!("Cannot open the log: {e:#}"));
            }
            let (rx, tx) = procs::spawn_wake(cfg.clone(), wake.clone());
            hub.procs_cfg = Some(cfg);
            hub.procs_rx = Some(rx);
            hub.procs_cmd = Some(tx);
        }
        Err(e) => *notice = Some(format!("Cannot start the engine and brain keeper: {e:#}")),
    }
    match brain::start() {
        Ok(link) => {
            hub.link_rx = Some(link.subscribe_wake(&[], wake));
            crate::screens::workflows::set_link(link.clone());
            crate::screens::settings::share_brain_link(link.clone(), cx);
            hub.link = Some(link);
        }
        Err(e) => *notice = Some(format!("Cannot link to the brain: {e:#}")),
    }
    hub
}

const TOKEN_REFUSED_NOTICE: &str =
    "Nixa brain refused the window token. Restart the Nixa brain to reconnect.";

fn token_refused(why: &str) -> bool {
    why.contains("401")
}

pub fn brain_off(hub: &Hub) -> bool {
    hub.procs.as_ref().is_some_and(|p| {
        matches!(
            p.brain,
            State::Stopped | State::Failed(_)
        )
    })
}

pub fn drain(ws: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    let mut changed = false;
    let mut states = Vec::new();
    if let Some(rx) = &ws.hub.procs_rx {
        while let Ok(state) = rx.try_recv() {
            states.push(state);
        }
    }
    if let Some(state) = states.pop() {
        ws.hub.procs = Some(state);
        changed = true;
        if !ws.hub.link.as_ref().is_some_and(Link::connected) {
            let link = if brain_off(&ws.hub) {
                ChatLink::Off
            } else {
                ChatLink::Starting
            };
            chat::set_link(&ws.hub.chat, link, cx);
        }
    }
    let mut msgs = Vec::new();
    if let Some(rx) = &ws.hub.link_rx {
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
    }
    push_numbers(ws);
    for msg in msgs {
        changed = true;
        match msg {
            BrainMsg::Up(hello) => {
                if ws.notice.as_deref() == Some(TOKEN_REFUSED_NOTICE) {
                    ws.notice = None;
                }
                on_up(ws, &hello, cx);
            }
            BrainMsg::Event(value) => on_brain_event(ws, &value, cx),
            BrainMsg::Down(why) => {
                if let Some(view) = crate::screens::menu::switches::shared(cx) {
                    crate::screens::menu::switches::set_linked(&view, false, cx);
                }
                let link = if brain_off(&ws.hub) {
                    ChatLink::Off
                } else {
                    if token_refused(&why) {
                        ws.notice = Some(TOKEN_REFUSED_NOTICE.into());
                    }
                    ChatLink::Starting
                };
                chat::set_link(&ws.hub.chat, link, cx);
            }
        }
    }
    changed
}

fn on_up(ws: &mut Workspace, hello: &Value, cx: &mut Context<Workspace>) {
    chat::apply(&ws.hub.chat, hello, cx);
    if let Some(view) = crate::screens::settings::parts::chat_brain::shared(cx) {
        crate::screens::settings::parts::chat_brain::apply(&view, hello, cx);
    }
    chat::set_link(&ws.hub.chat, ChatLink::Up, cx);
    if let Some(view) = crate::screens::menu::switches::shared(cx) {
        crate::screens::menu::switches::set_linked(&view, true, cx);
        crate::screens::menu::switches::feed(&view, hello, cx);
    }
    if let Some(view) = crate::screens::settings::parts::phone::shared(cx) {
        crate::screens::settings::parts::phone::apply(&view, hello, cx);
    }
    send_ws(ws, &crate::screens::settings::parts::general::ask_message());
    send_ws(ws, &crate::screens::quiet_switch::ask_message());
    ws.hub.numbers_sent = None;
    push_numbers(ws);
    crate::screens::settings::feed(&json!({"type": "hello"}), cx);
    crate::screens::workflows::apply(&json!({"type": "hello"}), cx);
    let Some(link) = ws.hub.link.clone() else {
        return;
    };
    ask_prompts(link.clone(), cx);
    fetch(link.clone(), "/health", "health", cx);
    fetch(link, "/announce", "announce", cx);
}

pub fn on_brain_event(ws: &mut Workspace, value: &Value, cx: &mut Context<Workspace>) {
    chat::apply(&ws.hub.chat, value, cx);
    if value["type"] == "spaces_changed" {
        crate::screens::spaces::sync_from_brain(ws, cx);
    }
    if let Some(view) = crate::screens::settings::parts::chat_brain::shared(cx) {
        crate::screens::settings::parts::chat_brain::apply(&view, value, cx);
    }
    if let Some(view) = crate::screens::menu::switches::shared(cx) {
        crate::screens::menu::switches::feed(&view, value, cx);
    }
    crate::screens::settings::parts::general::feed_shared(value, cx);
    crate::screens::phone_chip::feed(value, cx);
    crate::screens::talk_card::feed(value, cx);
    crate::screens::progress::feed(value, cx);
    crate::screens::quiet_switch::feed(value, cx);
    if let Some(view) = crate::screens::settings::parts::privacy::shared(cx) {
        crate::screens::settings::parts::privacy::apply(&view, value, cx);
    }
    crate::screens::tabbar::apply(&ws.tab_bar, value, cx);
    ws.hub.question.update(cx, |view, cx| {
        view.apply(value, cx);
    });
    events::apply(&ws.hub.events, value, cx);
    crate::screens::settings::parts::voices::apply(value, cx);
    if let Some(view) = crate::screens::settings::parts::mic::shared(cx) {
        crate::screens::settings::parts::mic::apply(&view, value, cx);
    }
    crate::screens::settings::feed(value, cx);
    crate::screens::tasks::apply(value, cx);
    crate::screens::models::apply(value, cx);
    crate::screens::workflows::apply(value, cx);
    crate::screens::desk_panels::apply(value, cx);
    if let Some(terminal) = crate::screens::custom_agents::apply(value, cx) {
        go_to_terminal(ws, &terminal, cx);
    }
}

fn on_events(ws: &mut Workspace, event: &EventsEvent, cx: &mut Context<Workspace>) {
    match event {
        EventsEvent::Focus { terminal } => go_to_terminal(ws, terminal, cx),
        EventsEvent::Resume(offer) => ws.resume_offer(offer.clone(), cx),
        EventsEvent::SetAgentVoice(on) => {
            send_ws(ws, &json!({"type": "set_agent_voice", "on": on}))
        }
        EventsEvent::Close => ws.refocus = true,
    }
    cx.notify();
}

fn on_toast(ws: &mut Workspace, event: &ToastEvent, cx: &mut Context<Workspace>) {
    match event {
        ToastEvent::Act(Action::Resume(offer)) => ws.resume_offer(offer.clone(), cx),
        ToastEvent::Act(Action::Focus(terminal)) => go_to_terminal(ws, terminal, cx),
    }
    cx.notify();
}

fn ask_prompts(link: Link, cx: &mut Context<Workspace>) {
    let task = cx
        .background_executor()
        .spawn(async move { link.request("prompts", json!({})) });
    cx.spawn(async move |this, cx| {
        if let Err(e) = task.await {
            this.update(cx, |ws, cx| {
                ws.notice = Some(format!("Cannot ask the brain for agent questions: {e:#}"));
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
}

fn fetch(link: Link, path: &'static str, kind: &'static str, cx: &mut Context<Workspace>) {
    let task = cx
        .background_executor()
        .spawn(async move { link.get(path) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |ws, cx| match result {
            Ok(value) => {
                let value = with_type(value, kind);
                on_brain_event(ws, &value, cx);
                cx.notify();
            }
            Err(e) => {
                ws.notice = Some(format!("The brain did not answer {path}: {e:#}"));
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

pub fn with_type(value: Value, kind: &str) -> Value {
    match value {
        Value::Object(mut map) => {
            map.insert("type".into(), Value::String(kind.to_string()));
            Value::Object(map)
        }
        other => json!({"type": kind, "data": other}),
    }
}

pub fn split_kind(value: &Value) -> Option<(String, Value)> {
    let mut map = value.as_object()?.clone();
    let kind = map.remove("type")?.as_str()?.to_string();
    Some((kind, Value::Object(map)))
}

pub fn numbers_message(numbers: &BTreeMap<TermId, u32>) -> Value {
    let map: serde_json::Map<String, Value> = numbers
        .iter()
        .map(|(id, n)| (id.to_string(), json!(n)))
        .collect();
    json!({"numbers": map})
}

pub fn numbers_due(sent: Option<&BTreeMap<TermId, u32>>, now: &BTreeMap<TermId, u32>, connected: bool) -> bool {
    connected && sent != Some(now)
}

pub fn push_numbers(ws: &mut Workspace) {
    let connected = ws.hub.link.as_ref().is_some_and(Link::connected);
    if !numbers_due(ws.hub.numbers_sent.as_ref(), &ws.tabs.numbers, connected) {
        return;
    }
    let Some(link) = ws.hub.link.as_ref() else {
        return;
    };
    if link.send("agent_numbers", numbers_message(&ws.tabs.numbers)).is_ok() {
        ws.hub.numbers_sent = Some(ws.tabs.numbers.clone());
    }
}

fn send_ws(ws: &mut Workspace, value: &Value) {
    let Some((kind, fields)) = split_kind(value) else {
        return;
    };
    let result = match &ws.hub.link {
        Some(link) => link.send(&kind, fields),
        None => Err(anyhow::anyhow!(brain::NOT_CONNECTED)),
    };
    if let Err(e) = result {
        ws.notice = Some(format!("Not sent to the brain: {e:#}"));
    }
}

pub fn send_chat(ws: &mut Workspace, cmd: crate::screens::chat::wire::Command, cx: &mut Context<Workspace>) {
    on_chat(ws, &ChatEvent::Send(cmd), cx);
}

fn on_chat(ws: &mut Workspace, event: &ChatEvent, cx: &mut Context<Workspace>) {
    match event {
        ChatEvent::Send(cmd) => match chat::to_wire(cmd) {
            Wire::Ws(value) => send_ws(ws, &value),
            Wire::Request { kind, fields } => ask_brain(ws, kind, fields.clone(), cx),
            Wire::Post { path, body } => {
                let Some(link) = ws.hub.link.clone() else {
                    ws.notice = Some(brain::NOT_CONNECTED.to_string());
                    cx.notify();
                    return;
                };
                let task = cx.background_executor().spawn(async move { link.post(path, &body) });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |ws, cx| {
                        if let Err(e) = result {
                            ws.notice = Some(format!("The brain did not take it: {e:#}"));
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        },
        ChatEvent::OpenSettings(part) => ws.open_settings(part, cx),
        ChatEvent::Hide => ws.set_panel(None, cx),
    }
    cx.notify();
}

fn ask_brain(ws: &mut Workspace, kind: &'static str, fields: Value, cx: &mut Context<Workspace>) {
    let Some(link) = ws.hub.link.clone() else {
        ws.notice = Some(brain::NOT_CONNECTED.to_string());
        return;
    };
    let task = cx
        .background_executor()
        .spawn(async move { link.request(kind, fields) });
    cx.spawn(async move |this, cx| {
        if let Err(e) = task.await {
            this.update(cx, |ws, cx| {
                ws.notice = Some(format!("The brain did not take it: {e:#}"));
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
}

fn on_question(ws: &mut Workspace, event: &QuestionEvent, cx: &mut Context<Workspace>) {
    match event {
        QuestionEvent::GoTo { terminal } => go_to_terminal(ws, terminal, cx),
        _ => {
            if let Some(value) = event.message() {
                send_ws(ws, &value);
            }
        }
    }
    cx.notify();
}

pub fn go_to_terminal(ws: &mut Workspace, terminal: &str, cx: &mut Context<Workspace>) {
    let found = ws.tabs.list.iter().find_map(|tab| {
        layout::leaves(&tab.root)
            .into_iter()
            .find(|id| id.to_string() == terminal)
            .map(|id| (tab.id.clone(), id))
    });
    match found {
        Some((tab, id)) => {
            ws.select_tab(&tab, cx);
            ws.focus_pane(&tab, &id, cx);
        }
        None => ws.notice = Some(format!("Terminal {terminal} has no pane here.")),
    }
}

pub fn command(ws: &mut Workspace, cmd: Cmd) {
    let sent = ws
        .hub
        .procs_cmd
        .as_ref()
        .is_some_and(|tx| tx.send(cmd).is_ok());
    if !sent {
        ws.notice = Some("The engine and brain keeper does not run.".into());
    }
}

pub fn sync(ws: &Workspace, cx: &mut Context<Workspace>) {
    let space = ws.active_space().map(|s| (s.id.clone(), s.name.clone()));
    chat::set_space(&ws.hub.chat, space, cx);
    chat::set_width(&ws.hub.chat, ws.panel_w, cx);
    if ws.hub.chat.read(cx).font_family.is_none() {
        let family = crate::screens::settings::parts::app::terminal_family(ws.store.as_ref());
        chat::set_font_family(&ws.hub.chat, family, cx);
    }
    let live: BTreeSet<String> = ws
        .terms
        .values()
        .filter(|t| t.running)
        .map(|t| t.id.to_string())
        .collect();
    ws.hub
        .events
        .events
        .update(cx, |view, cx| {
            view.set_live(live);
            view.numbers = ws
                .tabs
                .numbers
                .iter()
                .map(|(id, number)| (id.to_string(), *number))
                .collect();
            cx.notify();
        });
    agents::sync(&ws.hub.agents, ws, cx);
}

pub fn status(t: &Theme, hub: &Hub, kind: Kind) -> Option<(Hsla, String, String)> {
    let procs = hub.procs.as_ref()?;
    let cfg = hub.procs_cfg.as_ref()?;
    let state = match kind {
        Kind::Engine => &procs.engine,
        Kind::Brain => &procs.brain,
    };
    let (level, text) = procs::label(kind, state);
    let color = match level {
        Level::Good => t.green,
        Level::Busy => t.dim,
        Level::Bad => t.red,
    };
    let mut detail = procs::detail(cfg, kind, state);
    if kind == Kind::Brain
        && let Some(link) = &hub.link
    {
        let linked = if link.connected() {
            "linked"
        } else {
            "not linked yet"
        };
        detail = format!("{detail} Window link to {}: {linked}.", link.addr());
    }
    Some((color, text, detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_numbers_go_to_the_brain_by_terminal_id() {
        let mut numbers = BTreeMap::new();
        numbers.insert("nixa-sh-357".parse::<TermId>().unwrap(), 357);
        numbers.insert("nixa-sh-482913".parse::<TermId>().unwrap(), 1);
        assert_eq!(
            numbers_message(&numbers),
            json!({"numbers": {"nixa-sh-357": 357, "nixa-sh-482913": 1}})
        );
    }

    #[test]
    fn numbers_are_sent_on_change_and_after_reconnect_only() {
        let a: TermId = "nixa-sh-357".parse().unwrap();
        let b: TermId = "nixa-sh-482913".parse().unwrap();
        let mut now = BTreeMap::new();
        now.insert(a.clone(), 357);
        assert!(!numbers_due(None, &now, false));
        assert!(numbers_due(None, &now, true));
        let sent = now.clone();
        assert!(!numbers_due(Some(&sent), &now, true));
        now.insert(b, 1);
        assert!(numbers_due(Some(&sent), &now, true));
        assert!(!numbers_due(Some(&sent), &now, false));
        now.remove(&a);
        assert!(numbers_due(Some(&sent), &now, true));
        assert!(numbers_due(None, &sent, true));
    }

    #[test]
    fn a_clash_sends_the_sidebar_number_not_the_id_number() {
        let old: TermId = "nixa-sh-357".parse().unwrap();
        let new: TermId = "nixa-claude-357".parse().unwrap();
        let mut before = BTreeMap::new();
        before.insert(old.clone(), 357);
        let numbers = crate::tabs::assign_numbers(&before, &[new.clone(), old.clone()]);
        assert_eq!(numbers[&old], 357);
        assert_ne!(numbers[&new], 357);
        let sent = numbers_message(&numbers);
        assert_eq!(sent["numbers"]["nixa-sh-357"], json!(357));
        assert_eq!(sent["numbers"]["nixa-claude-357"], json!(numbers[&new]));
    }

    #[test]
    fn kinds_are_split_and_added() {
        let (kind, fields) = split_kind(&json!({"type": "typed", "text": "hi"})).unwrap();
        assert_eq!(kind, "typed");
        assert_eq!(fields, json!({"text": "hi"}));
        assert!(split_kind(&json!({"text": "hi"})).is_none());
        assert!(split_kind(&json!("x")).is_none());
        assert_eq!(
            with_type(json!({"ok": true, "mode": "manual"}), "decision"),
            json!({"ok": true, "mode": "manual", "type": "decision"})
        );
        assert_eq!(
            with_type(json!([1]), "x"),
            json!({"type": "x", "data": [1]})
        );
        assert!(token_refused("nixa-brain refused the link: HTTP error: 401 Unauthorized"));
        assert!(!token_refused("cannot reach nixa-brain at 127.0.0.1:7421"));
    }
}
