use crate::programs;
use crate::screens::menu::switches::{self, Row, RowKind, Switch, SwitchEvent, SwitchesView};
use crate::screens::tabbar::quick::{self, Button, QUICK_AGENTS};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Global, SharedString, Subscription, Window, div,
    px,
};
use nixa_ui::popover::tooltip;
use nixa_ui::toggle::toggle;
use nixa_ui::{Theme, icon, theme};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const PART: &str = "General";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keyword {
    pub word: String,
    pub aliases: Vec<String>,
    pub does: String,
    pub on: bool,
    pub risk: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topic {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub known: bool,
    pub topic: Option<Topic>,
    pub hold: bool,
    pub keywords: Vec<Keyword>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub id: &'static str,
    pub label: String,
    pub tip: String,
    pub command: Value,
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn parse_work(msg: &Value) -> Option<Work> {
    if msg.get("type").and_then(Value::as_str) != Some("work") {
        return None;
    }
    let work = msg.get("work")?;
    let mut keywords: Vec<Keyword> = work
        .get("keywords")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|row| Keyword {
                    word: text(row, "word"),
                    aliases: texts(row.get("aliases")),
                    does: text(row, "does"),
                    on: row.get("on").and_then(Value::as_bool) != Some(false),
                    risk: false,
                })
                .filter(|k| !k.word.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let risk_does = text(work, "riskDoes");
    if let Some(list) = work.get("risk").and_then(Value::as_array) {
        keywords.extend(list.iter().map(|row| Keyword {
            word: text(row, "word"),
            aliases: texts(row.get("aliases")),
            does: risk_does.clone(),
            on: true,
            risk: true,
        }));
    }
    let topic = work.get("topic").map(|t| Topic {
        kind: text(t, "kind"),
        text: text(t, "text"),
    });
    Some(Work {
        known: true,
        topic,
        hold: work.get("hold").is_some_and(|h| !h.is_null()),
        keywords,
        error: work
            .get("error")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
            .map(str::to_string),
    })
}

pub fn keyword_title(k: &Keyword) -> String {
    let tail = if k.risk {
        " · risky"
    } else if !k.on {
        " · off"
    } else {
        ""
    };
    format!("{}{tail}", k.word)
}

pub fn keyword_chip(work: &Work, name: &str) -> Option<Chip> {
    if work.hold {
        return Some(Chip {
            id: "general-hold",
            label: "Hold".into(),
            tip: "Hold is on: nothing runs by itself, no agent question is answered and no check runs. Click to end the hold, or say go on.".into(),
            command: json!({"type": "keyword_clear", "what": "hold"}),
        });
    }
    let topic = work.topic.as_ref()?;
    let word = if topic.kind == "plan" {
        "Plan"
    } else {
        "Discuss"
    };
    let about: String = topic.text.chars().take(40).collect();
    Some(Chip {
        id: "general-topic",
        label: if about.is_empty() {
            word.to_string()
        } else {
            format!("{word}: {about}")
        },
        tip: format!("{word} is on: {name} starts nobody until you say assign. Click to end it."),
        command: json!({"type": "keyword_clear", "what": "topic"}),
    })
}

pub struct GeneralView {
    pub switches: Entity<SwitchesView>,
    pub work: Work,
    pub programs: PathBuf,
    pub dirs: Vec<PathBuf>,
    pub quick: Option<Vec<Button>>,
    pub quick_error: Option<String>,
    _watch: Subscription,
}

pub struct Shared(pub Entity<GeneralView>);

impl Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<GeneralView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn build(switches: Entity<SwitchesView>, cx: &mut Context<GeneralView>) -> GeneralView {
    let watch = cx.observe(&switches, |_, _, cx| cx.notify());
    GeneralView {
        switches,
        work: Work::default(),
        programs: programs::default_path(),
        dirs: programs::default_dirs(),
        quick: None,
        quick_error: None,
        _watch: watch,
    }
}

pub fn new(switches: Entity<SwitchesView>, cx: &mut App) -> Entity<GeneralView> {
    cx.new(|cx| build(switches, cx))
}

pub fn install(switches: Entity<SwitchesView>, cx: &mut App) -> Entity<GeneralView> {
    let view = new(switches, cx);
    cx.set_global(Shared(view.clone()));
    view
}

pub fn feed(view: &Entity<GeneralView>, msg: &Value, cx: &mut App) {
    if let Some(work) = parse_work(msg) {
        view.update(cx, |v, cx| {
            if v.work != work {
                v.work = work;
                cx.notify();
            }
        });
    }
}

pub fn feed_shared(msg: &Value, cx: &mut App) {
    if let Some(view) = shared(cx) {
        feed(&view, msg, cx);
    }
}

pub fn ask_message() -> Value {
    json!({"type": "switches"})
}

pub fn flip(view: &mut GeneralView, switch: Switch, cx: &mut Context<GeneralView>) {
    view.switches.update(cx, |v, cx| {
        if let Some(on) = switches::flip(v, switch) {
            if v.snap.linked {
                cx.emit(SwitchEvent::Send(switches::tell(switch, on)));
            } else {
                v.error = Some(format!(
                    "Saved here. {} is not connected, so it was not told. Change it again after it starts.",
                    v.name
                ));
            }
        }
        cx.notify();
    });
}

pub fn send(view: &mut GeneralView, command: Value, cx: &mut Context<GeneralView>) {
    view.switches.update(cx, |v, cx| {
        if v.snap.linked {
            v.error = None;
            cx.emit(SwitchEvent::Send(command));
        } else {
            v.error = Some(format!(
                "{} is not connected. Change it again after it starts.",
                v.name
            ));
        }
        cx.notify();
    });
}

pub fn quick_rows(view: &mut GeneralView) -> Vec<Button> {
    if view.quick.is_none() {
        let list = quick::buttons_in(&view.programs, &view.dirs);
        view.quick = Some(list);
    }
    view.quick
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| QUICK_AGENTS.get(b.index).is_some_and(quick::can_turn_off))
        .collect()
}

pub fn flip_quick(view: &mut GeneralView, index: usize, cx: &mut Context<GeneralView>) {
    let Some(agent) = QUICK_AGENTS.get(index) else {
        return;
    };
    let old = match programs::read(&view.programs) {
        Ok(text) => text,
        Err(e) => {
            view.quick_error = Some(format!("Cannot read programs.json: {e}"));
            cx.notify();
            return;
        }
    };
    let enable_normal_checks = quick::off_of(old.as_deref(), agent);
    let saved = programs::set_normal_checks(old.as_deref(), agent.provider, enable_normal_checks)
        .and_then(|change| programs::write_if_same(&view.programs, &change.text, old.as_deref()));
    view.quick_error = saved.err();
    view.quick = None;
    cx.notify();
}

pub fn quick_tip(agent: &quick::QuickAgent, off: bool) -> String {
    let flags = quick::off_flags(agent);
    if off {
        format!(
            "{} starts with its checks off ({flags}) when Nixa starts it for you. It can change files and run commands without asking. Turn on normal checks to use its permission prompts.",
            agent.name
        )
    } else {
        format!(
            "{} starts with its normal checks when Nixa starts it for you. Turn off normal checks to start it with {flags}.",
            agent.name
        )
    }
}

fn quick_row(t: &Theme, b: Button, cx: &mut Context<GeneralView>) -> Option<AnyElement> {
    let agent = QUICK_AGENTS.get(b.index)?;
    let hover = t.hover;
    let id = SharedString::from(format!("general-quick-{}", agent.id));
    let selector = id.to_string();
    let index = b.index;
    Some(
        div()
            .id(id.clone())
            .debug_selector(move || selector.clone())
            .h(px(34.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(quick_tip(agent, b.off)))
            .on_click(cx.listener(move |view, _, _, cx| flip_quick(view, index, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.foreground)
                    .child(SharedString::from(format!("{}: use normal checks", agent.name))),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(if b.off { t.yellow } else { t.dim })
                    .child(if b.off { "checks off" } else { "normal checks" }),
            )
            .child(toggle(t, SharedString::from(format!("{id}-toggle")), !b.off))
            .into_any_element(),
    )
}

fn quick_part(view: &mut GeneralView, t: &Theme, cx: &mut Context<GeneralView>) -> AnyElement {
    let rows: Vec<AnyElement> = quick_rows(view)
        .into_iter()
        .filter_map(|b| quick_row(t, b, cx))
        .collect();
    let empty = rows.is_empty();
    let error = view.quick_error.clone();
    section(
        t,
        "general-quick",
        "Quick agent buttons".into(),
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(note(
                t,
                "Agents start with permission checks off by default. Turn on normal checks for an agent to use its permission prompts when Nixa starts or resumes it.".into(),
            ))
            .when(empty, |el| {
                el.child(note(t, "No quick agent is installed yet.".into()))
            })
            .children(rows)
            .when_some(error, |el, error| {
                el.child(
                    div()
                        .id("general-quick-error")
                        .debug_selector(|| "general-quick-error".into())
                        .text_size(px(11.))
                        .text_color(t.red)
                        .child(SharedString::from(error)),
                )
            }),
    )
}

fn pick(view: &mut GeneralView, kind: &RowKind, cx: &mut Context<GeneralView>) {
    match kind {
        RowKind::Switch(switch, _) => flip(view, *switch, cx),
        RowKind::NixaScreen => view
            .switches
            .update(cx, |_, cx| cx.emit(SwitchEvent::NixaScreen)),
        RowKind::ShowCard => view
            .switches
            .update(cx, |_, cx| cx.emit(SwitchEvent::ShowCard)),
    }
}

pub fn section(t: &Theme, id: &'static str, title: String, body: impl IntoElement) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .py(px(10.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.blue)
                .child(SharedString::from(title)),
        )
        .child(body)
        .into_any_element()
}

pub fn note(t: &Theme, text: String) -> AnyElement {
    div()
        .text_size(px(11.5))
        .text_color(t.dim)
        .child(SharedString::from(text))
        .into_any_element()
}

fn row_element(t: &Theme, row: &Row, cx: &mut Context<GeneralView>) -> AnyElement {
    let hover = t.hover;
    let kind = row.kind.clone();
    let id = SharedString::from(format!("general-{}", row.id));
    let selector = id.to_string();
    let toggle_on = match row.kind {
        RowKind::Switch(_, on) => Some(on.unwrap_or(false)),
        _ => None,
    };
    div()
        .id(id.clone())
        .debug_selector(move || selector.clone())
        .h(px(34.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(row.tip.clone()))
        .on_click(cx.listener(move |view, _, _, cx| pick(view, &kind, cx)))
        .child(icon(row.icon).size(px(14.)).text_color(t.muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.foreground)
                .child(SharedString::from(row.label.clone())),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.dim)
                .child(SharedString::from(row.hint.clone())),
        )
        .when_some(toggle_on, |el, on| {
            el.child(toggle(t, SharedString::from(format!("{id}-toggle")), on))
        })
        .into_any_element()
}

fn action(
    t: &Theme,
    id: String,
    label: String,
    selected: bool,
    tip: Option<String>,
    command: Option<Value>,
    cx: &mut Context<GeneralView>,
) -> AnyElement {
    let live = command.is_some();
    let hover = t.hover;
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .flex_none()
        .px(px(9.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(if selected { t.green } else { t.border })
        .text_size(px(12.))
        .text_color(if !live {
            t.dim
        } else if selected {
            t.green
        } else {
            t.foreground
        })
        .when(live, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .when_some(tip, |el, tip| el.tooltip(tooltip(tip)))
        .when_some(command, |el, command| {
            el.on_click(cx.listener(move |view, _, _, cx| send(view, command.clone(), cx)))
        })
        .child(SharedString::from(label))
        .into_any_element()
}

fn keyword_row(t: &Theme, index: usize, k: &Keyword) -> AnyElement {
    div()
        .id(("general-keyword", index))
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(4.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if k.risk { t.red } else { t.foreground })
                .child(SharedString::from(keyword_title(k))),
        )
        .when(!k.aliases.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child(SharedString::from(format!(
                        "Also: {}",
                        k.aliases.join(", ")
                    ))),
            )
        })
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.muted)
                .child(SharedString::from(k.does.clone())),
        )
        .into_any_element()
}

fn brain_parts(view: &GeneralView, t: &Theme, cx: &mut Context<GeneralView>) -> Vec<AnyElement> {
    let (snap, name) = {
        let s = view.switches.read(cx);
        (s.snap.clone(), s.name.clone())
    };
    if !snap.linked {
        return vec![note(
            t,
            format!(
                "Connect {name} to change privacy. The on or off switch above is always available."
            ),
        )];
    }
    let p = switches::privacy(&snap, &name);
    let mut modes: Vec<AnyElement> = Vec::new();
    if let Some(chip) = keyword_chip(&view.work, &name) {
        modes.push(action(
            t,
            chip.id.to_string(),
            chip.label,
            true,
            Some(chip.tip),
            Some(chip.command),
            cx,
        ));
    }
    let words: Vec<AnyElement> = view
        .work
        .keywords
        .iter()
        .enumerate()
        .map(|(i, k)| keyword_row(t, i, k))
        .collect();
    let count = view.work.keywords.len();
    let known = view.work.known;
    let error = view.work.error.clone();
    vec![
        section(
            t,
            "general-privacy",
            "Privacy".into(),
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().flex().child(action(
                    t,
                    "general-privacy-switch".into(),
                    p.label.to_string(),
                    p.on == Some(true),
                    Some(p.tip.to_string()),
                    p.command.clone(),
                    cx,
                )))
                .child(note(t, format!("{} {}", p.note, p.tip))),
        ),
        section(
            t,
            "general-work",
            "Hold and topic".into(),
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().flex().flex_wrap().gap(px(6.)).children(modes))
                .child(note(t, format!("Say hold to stop {name} for now, or plan or discuss a topic. Click the chip to end it."))),
        ),
        section(
            t,
            "general-keywords",
            format!("Keywords ({count})"),
            div()
                .id("general-keyword-list")
                .w_full()
                .flex()
                .flex_col()
                .gap(px(6.))
                .when_some(error, |el, error| {
                    el.child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.red)
                            .child(SharedString::from(error)),
                    )
                })
                .when(!known, |el| {
                    el.child(note(
                        t,
                        format!("{name} has not sent the keyword list yet."),
                    ))
                })
                .children(words),
        ),
    ]
}

impl Render for GeneralView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let (rows, error) = {
            let s = self.switches.read(cx);
            (switches::rows(&s.state, &s.snap, &s.name), s.error.clone())
        };
        let rows: Vec<AnyElement> = rows.iter().map(|row| row_element(&t, row, cx)).collect();
        let parts = brain_parts(self, &t, cx);
        let quick = quick_part(self, &t, cx);
        div()
            .id("settings-general")
            .debug_selector(|| "settings-general".into())
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.8))
            .children(rows)
            .child(quick)
            .children(parts)
            .when_some(error, |el, error| {
                el.child(
                    div()
                        .id("general-error")
                        .debug_selector(|| "general-error".into())
                        .text_size(px(11.))
                        .text_color(t.red)
                        .child(SharedString::from(error)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    type Seen = Rc<RefCell<Vec<SwitchEvent>>>;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("msg-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn work_msg() -> Value {
        json!({"type": "work", "work": {
            "hold": {"at": 5},
            "topic": {"kind": "plan", "text": "the login page", "at": 4},
            "keywords": [
                {"word": "discuss", "aliases": ["lets discuss"], "does": "Talks it through.", "on": true, "live": true},
                {"word": "status", "aliases": [], "does": "Says what is going on.", "on": false, "live": false}
            ],
            "risk": [{"word": "push", "aliases": ["push it"]}],
            "riskDoes": "Always waits.",
            "error": "Row 2 is bad."
        }})
    }

    fn click(vcx: &mut VisualTestContext, selector: &'static str) {
        vcx.run_until_parked();
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
    }

    fn sent(seen: &Seen) -> Vec<SwitchEvent> {
        std::mem::take(&mut *seen.borrow_mut())
    }

    #[test]
    fn work_messages_give_the_keyword_list() {
        assert!(parse_work(&json!({"type": "state"})).is_none());
        assert!(parse_work(&json!({"type": "work"})).is_none());
        let work = parse_work(&work_msg()).unwrap();
        assert!(work.known && work.hold);
        assert_eq!(
            work.topic,
            Some(Topic {
                kind: "plan".into(),
                text: "the login page".into()
            })
        );
        let titles: Vec<String> = work.keywords.iter().map(keyword_title).collect();
        assert_eq!(titles, ["discuss", "status · off", "push · risky"]);
        assert_eq!(work.keywords[2].does, "Always waits.");
        assert_eq!(work.keywords[0].aliases, ["lets discuss"]);
        assert_eq!(work.error.as_deref(), Some("Row 2 is bad."));
        let chip = keyword_chip(&work, "Nixa").unwrap();
        assert_eq!(
            (chip.label.as_str(), chip.command.clone()),
            ("Hold", json!({"type": "keyword_clear", "what": "hold"}))
        );
        let topic = Work {
            hold: false,
            ..work.clone()
        };
        let chip = keyword_chip(&topic, "Nova").unwrap();
        assert_eq!(chip.label, "Plan: the login page");
        assert!(chip.tip.starts_with("Plan is on: Nova starts nobody"));
        assert_eq!(
            chip.command,
            json!({"type": "keyword_clear", "what": "topic"})
        );
        let discuss = Work {
            topic: Some(Topic {
                kind: "discuss".into(),
                text: String::new(),
            }),
            ..topic
        };
        assert_eq!(keyword_chip(&discuss, "Nixa").unwrap().label, "Discuss");
        assert!(keyword_chip(&Work::default(), "Nixa").is_none());
        assert_eq!(ask_message(), json!({"type": "switches"}));
    }

    #[gpui::test]
    fn the_general_part_flips_switches_and_sends_brain_settings(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let data = temp("part");
        let store = nixa_store::open_memory().unwrap();
        let sw = cx.update(|cx| switches::new(data.clone(), Some(store), cx));
        let seen: Seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        cx.update(|cx| {
            cx.subscribe(&sw, move |_, e: &SwitchEvent, _| {
                log.borrow_mut().push(e.clone())
            })
            .detach()
        });
        let s = sw.clone();
        let file = data.join("programs.json");
        let (view, vcx) = cx.add_window_view(move |_, cx| {
            let mut v = build(s, cx);
            v.programs = file;
            v.dirs = Vec::new();
            v
        });
        click(vcx, "general-switch-nixa");
        assert!(data.join(switches::OFF_FILE).exists());
        assert!(sent(&seen).is_empty());
        assert!(
            vcx.update(|_, cx| sw.read(cx).error.clone())
                .unwrap()
                .starts_with("Saved here.")
        );
        assert!(vcx.debug_bounds("general-error").is_some());
        click(vcx, "general-switch-nixa-screen");
        assert_eq!(sent(&seen), vec![SwitchEvent::NixaScreen]);
        vcx.update(|_, cx| {
            switches::set_linked(&sw, true, cx);
            switches::feed(&sw, &json!({"type": "hello", "name": "Nixa"}), cx);
            switches::feed(
                &sw,
                &json!({"type": "state", "state": "idle", "privacy": true}),
                cx,
            );
            switches::feed(&sw, &work_msg(), cx);
            feed(&view, &work_msg(), cx);
        });
        click(vcx, "general-switch-nixa");
        click(vcx, "general-switch-phones");
        click(vcx, "general-privacy-switch");
        click(vcx, "general-hold");
        assert_eq!(
            sent(&seen),
            vec![
                SwitchEvent::Send(json!({"type": "set_nixa", "on": true})),
                SwitchEvent::Send(json!({"type": "set_headphones", "on": true})),
                SwitchEvent::Send(json!({"type": "set_privacy", "on": false})),
                SwitchEvent::Send(json!({"type": "keyword_clear", "what": "hold"})),
            ]
        );
        assert!(!data.join(switches::OFF_FILE).exists());
        assert!(data.join(switches::HEADPHONES_FILE).exists());
        assert_eq!(vcx.update(|_, cx| view.read(cx).work.keywords.len()), 3);
        vcx.update(|_, cx| switches::set_linked(&sw, false, cx));
        click(vcx, "general-hold");
        assert!(sent(&seen).is_empty());
        std::fs::remove_dir_all(&data).ok();
    }

    #[gpui::test]
    fn quick_agent_switches_save_normal_checks_per_agent(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        cx.update(nixa_ui::init);
        let data = temp("quick");
        let bins = data.join("bin");
        std::fs::create_dir_all(&bins).unwrap();
        for bin in ["claude", "jcode"] {
            let path = bins.join(bin);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let file = data.join("programs.json");
        let legacy = r#"{"programs":{"mine":{"run":["m"]}},"checksOff":["claude","codex"]}"#;
        std::fs::write(&file, legacy).unwrap();
        let store = nixa_store::open_memory().unwrap();
        let sw = cx.update(|cx| switches::new(data.clone(), Some(store), cx));
        let (f, d) = (file.clone(), vec![bins.clone()]);
        let (view, vcx) = cx.add_window_view(move |_, cx| {
            let mut v = build(sw, cx);
            v.programs = f;
            v.dirs = d;
            v
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("general-quick-claude").is_some());
        assert!(vcx.debug_bounds("general-quick-jcode").is_none());
        assert!(vcx.debug_bounds("general-quick-codex").is_none());
        let settings = || {
            let text = std::fs::read_to_string(&file).unwrap();
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        };
        let original = settings();
        click(vcx, "general-quick-claude");
        let mut expected = original.clone();
        expected["checksOn"] = serde_json::json!(["claude"]);
        assert_eq!(settings(), expected);
        let jcode = QUICK_AGENTS.iter().position(|a| a.id == "jcode").unwrap();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).quick.clone()),
            Some(vec![Button { index: 0, off: false }, Button { index: jcode, off: false }])
        );
        click(vcx, "general-quick-claude");
        assert_eq!(settings(), original);
        let button = vcx.update(|_, cx| view.read(cx).quick.as_ref().unwrap()[0]);
        assert_eq!(quick::start(button, None).unwrap().1, ["claude", "--dangerously-skip-permissions"]);
        assert!(vcx.update(|_, cx| view.read(cx).quick_error.is_none()));
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn quick_tips_name_the_flags() {
        let claude = &QUICK_AGENTS[0];
        assert!(quick_tip(claude, false).starts_with("Claude Code starts with its normal checks when Nixa starts it for you."));
        assert!(quick_tip(claude, true).contains("(--dangerously-skip-permissions)"));
    }
}
