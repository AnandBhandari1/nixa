use crate::spaces::{auto_short, home, short_folder};
use anyhow::{Result, bail};
use gpui::prelude::*;
use gpui::{
    AnchoredPositionMode, AnyElement, App, Context, Div, Entity, Focusable, FontWeight, Hsla,
    SharedString, Stateful, Subscription, Window, anchored, deferred, div, point, px,
};
use nixa_store::Connection;
use nixa_store::settings::{get_setting, set_setting};
use nixa_store::spaces::{self as store, MAX_DOING, MAX_DOING_TOP, SpacePatch};
use nixa_ui::popover::{popover, tooltip};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};

pub const PART: &str = "Spaces";
pub const SHORT_KEY: &str = crate::spaces::SHORT_KEY;
pub const MAX_CHOICES: [i64; 5] = [1, 2, 3, 4, 5];
const TICK: Duration = Duration::from_millis(500);
const RELOAD_EVERY: Duration = Duration::from_secs(2);
const INTRO: &str = "Each space: auto-start, open or locked, and how many tasks run at once.";
const NO_STORE: &str = "The store is not open, so the spaces cannot be shown or changed.";

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub short: String,
    pub auto_start: bool,
    pub locked: bool,
    pub max_doing: i64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Data {
    pub cards: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub value: String,
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Short(String),
    AutoStart(bool),
    Locked(bool),
    Max(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Short,
    Max,
}

impl Field {
    fn word(self) -> &'static str {
        match self {
            Field::Short => "short",
            Field::Max => "max",
        }
    }
}

fn pick(value: impl Into<String>, label: impl Into<String>, detail: Option<&str>) -> Pick {
    Pick {
        value: value.into(),
        label: label.into(),
        detail: detail.map(str::to_string),
    }
}

fn shorts(conn: &Connection) -> Result<Map<String, Value>> {
    Ok(get_setting(conn, SHORT_KEY)?
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default())
}

pub fn load(conn: &Connection) -> Result<Data> {
    let shorts = shorts(conn)?;
    let cards = store::list_spaces(conn)?
        .into_iter()
        .map(|s| Card {
            short: shorts
                .get(&s.space_id)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            id: s.space_id,
            name: s.name,
            folder: s.folder,
            auto_start: s.auto_start,
            locked: s.locked,
            max_doing: if s.max_doing > 0 {
                s.max_doing
            } else {
                MAX_DOING
            },
        })
        .collect();
    Ok(Data {
        cards,
    })
}

fn words(name: &str) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut lower = false;
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() {
            if !word.is_empty() {
                list.push(std::mem::take(&mut word));
            }
            lower = false;
            continue;
        }
        if lower && c.is_ascii_uppercase() && !word.is_empty() {
            list.push(std::mem::take(&mut word));
        }
        lower = c.is_ascii_lowercase();
        word.push(c);
    }
    if !word.is_empty() {
        list.push(word);
    }
    list
}

fn head(word: &str) -> String {
    word.chars().take(3).collect::<String>().to_uppercase()
}

pub fn short_choices(name: &str) -> Vec<String> {
    let parts = words(name);
    let mut list = vec![auto_short(name)];
    list.extend(parts.iter().map(|w| head(w)));
    if parts.len() > 1 {
        list.push(
            parts
                .iter()
                .take(3)
                .filter_map(|w| w.chars().next())
                .collect::<String>()
                .to_uppercase(),
        );
    }
    let mut out: Vec<String> = Vec::new();
    for item in list {
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

fn base_name(card: &Card) -> &str {
    if card.name.trim().is_empty() {
        &card.id
    } else {
        &card.name
    }
}

pub fn short_display(card: &Card) -> String {
    if card.short.is_empty() {
        format!("{} (automatic)", auto_short(base_name(card)))
    } else {
        card.short.clone()
    }
}

pub fn short_picks(card: &Card) -> Vec<Pick> {
    let auto = auto_short(base_name(card));
    let mut values: Vec<String> = short_choices(base_name(card))
        .into_iter()
        .filter(|v| *v != auto)
        .collect();
    if !card.short.is_empty() && card.short != auto && !values.contains(&card.short) {
        values.push(card.short.clone());
    }
    let mut list = vec![pick(
        "",
        format!("{auto} (automatic)"),
        Some("Made from the space name"),
    )];
    list.extend(values.into_iter().map(|v| pick(v.clone(), v, None)));
    list
}

pub fn clean_short(typed: &str) -> Option<String> {
    let text = typed.trim();
    let count = text.chars().count();
    ((1..=3).contains(&count) && text.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| text.to_uppercase())
}

fn tasks_word(n: i64) -> String {
    if n == 1 {
        "1 task".to_string()
    } else {
        format!("{n} tasks")
    }
}

pub fn max_picks(max: i64) -> Vec<Pick> {
    let mut list: Vec<i64> = MAX_CHOICES.to_vec();
    if !list.contains(&max) {
        list.push(max);
    }
    list.into_iter()
        .map(|n| pick(n.to_string(), tasks_word(n), None))
        .collect()
}

pub fn picks_for(card: &Card, field: Field) -> Vec<Pick> {
    match field {
        Field::Short => short_picks(card),
        Field::Max => max_picks(card.max_doing),
    }
}

pub fn current_of(card: &Card, field: Field) -> String {
    match field {
        Field::Short => card.short.clone(),
        Field::Max => card.max_doing.to_string(),
    }
}

pub fn change_of(field: Field, value: &str) -> Option<Change> {
    match field {
        Field::Short => Some(Change::Short(value.to_string())),
        Field::Max => value.parse().ok().map(Change::Max),
    }
}

pub fn save_short(conn: &Connection, id: &str, value: &str) -> Result<()> {
    let mut map = shorts(conn)?;
    let clean = head(value.trim());
    if clean.is_empty() {
        map.remove(id);
    } else {
        map.insert(id.to_string(), Value::String(clean));
    }
    set_setting(conn, SHORT_KEY, &Value::Object(map))
}

pub fn apply(conn: &Connection, id: &str, change: &Change) -> Result<()> {
    if store::get_space(conn, id)?.is_none() {
        bail!("This space is gone. The list was read again.");
    }
    let patch = match change {
        Change::Short(value) => return save_short(conn, id, value),
        Change::AutoStart(on) => SpacePatch {
            auto_start: Some(*on),
            ..SpacePatch::default()
        },
        Change::Locked(on) => SpacePatch {
            locked: Some(*on),
            ..SpacePatch::default()
        },
        Change::Max(n) => {
            if !(1..=MAX_DOING_TOP).contains(n) {
                bail!("Pick 1 to {MAX_DOING_TOP} tasks at once.");
            }
            SpacePatch {
                max_doing: Some(*n),
                ..SpacePatch::default()
            }
        }
    };
    store::set_space(conn, id, &patch)?;
    Ok(())
}

pub struct Typing {
    pub id: String,
    pub input: Entity<TextInput>,
    _events: Subscription,
}

pub struct SpacesPart {
    pub store: Option<Connection>,
    pub data: Data,
    pub loaded: bool,
    pub error: Option<String>,
    pub open: Option<(String, Field)>,
    pub typing: Option<Typing>,
    reload_at: Instant,
}

pub fn build(store: Option<Connection>, cx: &mut Context<SpacesPart>) -> SpacesPart {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            if this.update(cx, tick).is_err() {
                break;
            }
        }
    })
    .detach();
    let mut part = SpacesPart {
        store,
        data: Data::default(),
        loaded: false,
        error: None,
        open: None,
        typing: None,
        reload_at: Instant::now(),
    };
    reload(&mut part);
    part
}

pub fn new(cx: &mut App) -> Entity<SpacesPart> {
    let store = if cfg!(test) {
        None
    } else {
        nixa_store::open_default().ok()
    };
    cx.new(|cx| build(store, cx))
}

pub fn render(view: &Entity<SpacesPart>) -> AnyElement {
    view.clone().into_any_element()
}

pub fn reload(part: &mut SpacesPart) {
    part.reload_at = Instant::now();
    let Some(conn) = part.store.as_ref() else {
        part.loaded = false;
        part.error = Some(NO_STORE.into());
        return;
    };
    match load(conn) {
        Ok(data) => {
            part.data = data;
            part.loaded = true;
        }
        Err(e) => part.error = Some(format!("The spaces could not be read: {e:#}")),
    }
}

fn tick(part: &mut SpacesPart, cx: &mut Context<SpacesPart>) {
    if part.open.is_some() || part.typing.is_some() || part.reload_at.elapsed() < RELOAD_EVERY {
        return;
    }
    let before = part.data.clone();
    reload(part);
    if part.data != before {
        cx.notify();
    }
}

pub fn change(part: &mut SpacesPart, id: &str, change: Change) {
    part.open = None;
    let result = match part.store.as_ref() {
        Some(conn) => apply(conn, id, &change),
        None => Err(anyhow::anyhow!(NO_STORE)),
    };
    part.error = result.err().map(|e| format!("{e:#}"));
    let error = part.error.take();
    reload(part);
    if error.is_some() {
        part.error = error;
    }
}

pub fn start_typing(
    part: &mut SpacesPart,
    id: &str,
    window: &mut Window,
    cx: &mut Context<SpacesPart>,
) {
    part.open = None;
    let current = part
        .data
        .cards
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.short.clone())
        .unwrap_or_default();
    let input = cx.new(|cx| {
        let mut input = TextInput::new(cx, "Up to 3 letters or numbers");
        input.set_value(current, cx);
        input
    });
    window.focus(&input.focus_handle(cx));
    let key = id.to_string();
    let events = cx.subscribe(
        &input,
        move |part: &mut SpacesPart, _, event: &InputEvent, cx| {
            match event {
                InputEvent::Submit(text) => match clean_short(text) {
                    Some(value) => {
                        part.typing = None;
                        change(part, &key, Change::Short(value));
                    }
                    None => part.error = Some("Use 1 to 3 letters or numbers.".into()),
                },
                InputEvent::Cancel => {
                    part.typing = None;
                    part.error = None;
                }
                InputEvent::Changed(_) => {}
            }
            cx.notify();
        },
    );
    part.typing = Some(Typing {
        id: id.to_string(),
        input,
        _events: events,
    });
    cx.notify();
}

fn line(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    div()
        .w_full()
        .text_size(px(size))
        .text_color(color)
        .child(text.into())
}

fn caption(t: &Theme, text: &'static str) -> Div {
    div().text_size(px(11.)).text_color(t.dim).child(text)
}

fn boxed(t: &Theme, edge: Hsla) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(edge)
        .bg(t.chrome)
}

fn chip_button(
    t: &Theme,
    id: SharedString,
    label: &str,
    tip: String,
    on: bool,
    color: Hsla,
    enabled: bool,
) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .h(px(24.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(if on { color } else { t.border })
        .when(on, |el| el.bg(color.opacity(0.16)))
        .text_size(px(12.))
        .text_color(if on { color } else { t.foreground })
        .tooltip(tooltip(tip))
        .when(enabled, |el| {
            el.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .when(!enabled, |el| el.opacity(0.5).cursor_not_allowed())
        .child(label.to_string())
}

fn picker(
    t: &Theme,
    card: &Card,
    field: Field,
    picks: Vec<Pick>,
    current: String,
    cx: &mut Context<SpacesPart>,
) -> AnyElement {
    let id = card.id.clone();
    let base = format!("spaces-pick-{}-{}", field.word(), card.id);
    let hover = t.hover;
    let mut panel = popover(t)
        .id(SharedString::from(base.clone()))
        .min_w(px(220.))
        .max_w(px(380.))
        .max_h(px(280.))
        .overflow_y_scroll()
        .on_mouse_down_out(cx.listener(|part, _, _, cx| {
            part.open = None;
            cx.notify();
        }));
    for (index, item) in picks.into_iter().enumerate() {
        let on = item.value == current;
        let row_id = format!("{base}-{index}");
        let selector = row_id.clone();
        let key = id.clone();
        let value = item.value.clone();
        panel = panel.child(
            div()
                .id(SharedString::from(row_id))
                .debug_selector(move || selector)
                .w_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(5.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .when(on, |el| el.bg(t.selected))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(t.foreground)
                                .child(item.label),
                        )
                        .when_some(item.detail, |el, detail| {
                            el.child(div().text_size(px(10.5)).text_color(t.dim).child(detail))
                        }),
                )
                .when(on, |el| {
                    el.child(icon(Icon::Check).size(px(13.)).text_color(t.foreground))
                })
                .on_click(cx.listener(move |part, _, _, cx| {
                    if let Some(next) = change_of(field, &value) {
                        let same = part
                            .data
                            .cards
                            .iter()
                            .find(|c| c.id == key)
                            .is_some_and(|c| current_of(c, field) == value);
                        if same {
                            part.open = None;
                        } else {
                            change(part, &key, next);
                        }
                    }
                    cx.notify();
                })),
        );
    }
    if field == Field::Short {
        let key = id.clone();
        let row_id = format!("{base}-type");
        let selector = row_id.clone();
        panel = panel.child(
            div()
                .id(SharedString::from(row_id))
                .debug_selector(move || selector)
                .mt(px(4.))
                .pt(px(6.))
                .border_t_1()
                .border_color(t.border)
                .px(px(8.))
                .py(px(5.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .text_size(px(12.5))
                .text_color(t.muted)
                .child("Type your own…")
                .on_click(cx.listener(move |part, _, window, cx| {
                    start_typing(part, &key, window, cx);
                })),
        );
    }
    panel.into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn dropdown(
    part: &SpacesPart,
    t: &Theme,
    card: &Card,
    field: Field,
    label: &'static str,
    display: String,
    tip: Option<&'static str>,
    cx: &mut Context<SpacesPart>,
) -> AnyElement {
    let open = part
        .open
        .as_ref()
        .is_some_and(|(id, f)| *id == card.id && *f == field);
    let trigger_id = format!("spaces-{}-{}", field.word(), card.id);
    let selector = trigger_id.clone();
    let key = card.id.clone();
    let hover = t.hover;
    let mut trigger = div()
        .id(SharedString::from(trigger_id))
        .debug_selector(move || selector)
        .w_full()
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(if open { t.accent } else { t.border })
        .bg(t.background)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.5))
                .child(display),
        )
        .child(icon(Icon::ChevronDown).size(px(13.)).text_color(t.dim))
        .on_click(cx.listener(move |part, _, _, cx| {
            cx.stop_propagation();
            let next = (key.clone(), field);
            part.open = if part.open.as_ref() == Some(&next) {
                None
            } else {
                Some(next)
            };
            cx.notify();
        }));
    if let Some(tip) = tip {
        trigger = trigger.tooltip(tooltip(tip));
    }
    let mut holder = div()
        .relative()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(caption(t, label))
        .child(trigger);
    if open {
        let picks = picks_for(card, field);
        let current = current_of(card, field);
        let panel = picker(t, card, field, picks, current, cx);
        holder = holder.child(
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .position(point(px(0.), px(48.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(1),
        );
    }
    holder.into_any_element()
}

fn short_field(
    part: &SpacesPart,
    t: &Theme,
    card: &Card,
    cx: &mut Context<SpacesPart>,
) -> AnyElement {
    let typing = part.typing.as_ref().filter(|ty| ty.id == card.id);
    match typing {
        Some(ty) => div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(caption(t, "Short name on the narrow sidebar"))
            .child(
                div()
                    .w(px(240.))
                    .h(px(28.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.accent)
                    .bg(t.background)
                    .child(ty.input.clone()),
            )
            .child(line("Up to 3 letters or numbers. Enter saves, Esc goes back.", t.dim, 10.5))
            .into_any_element(),
        None => div()
            .w(px(240.))
            .child(dropdown(
                part,
                t,
                card,
                Field::Short,
                "Short name on the narrow sidebar",
                short_display(card),
                Some("The 3 letters shown for this space when the left sidebar is narrow. Pick one, or type up to 3 letters or numbers."),
                cx,
            ))
            .into_any_element(),
    }
}

fn space_card(
    part: &SpacesPart,
    t: &Theme,
    card: &Card,
    cx: &mut Context<SpacesPart>,
) -> AnyElement {
    let name = if card.name.trim().is_empty() {
        card.id.clone()
    } else {
        card.name.clone()
    };
    let folder = short_folder(&card.folder, &home());
    let auto_key = card.id.clone();
    let auto_on = card.auto_start;
    let lock_key = card.id.clone();
    let locked = card.locked;
    let auto_tip = if auto_on {
        "Nixa starts the next task here by itself. Click to turn off."
    } else {
        "Nixa does not start tasks here by itself. Click to turn on."
    };
    let lock_tip = if locked {
        "Locked: Nixa starts no new work here. Click to open it."
    } else {
        "Open: Nixa may start work here. Click to lock it."
    };
    boxed(t, t.border)
        .id(SharedString::from(format!("spaces-card-{}", card.id)))
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(name))
                .when(!card.folder.is_empty(), |el| {
                    el.child(div().w_full().truncate().text_size(px(10.5)).text_color(t.dim).child(folder))
                }),
        )
        .child(short_field(part, t, card, cx))
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(px(16.))
                .child(
                    div().flex_none().flex().flex_col().gap(px(3.)).child(caption(t, "Auto-start")).child(
                        div().h(px(28.)).flex().items_center().child(
                            chip_button(
                                t,
                                SharedString::from(format!("spaces-auto-{}", card.id)),
                                if auto_on { "On" } else { "Off" },
                                auto_tip.into(),
                                auto_on,
                                t.accent,
                                part.store.is_some(),
                            )
                            .on_click(cx.listener(move |part, _, _, cx| {
                                change(part, &auto_key, Change::AutoStart(!auto_on));
                                cx.notify();
                            })),
                        ),
                    ),
                )
                .child(
                    div().flex_none().flex().flex_col().gap(px(3.)).child(caption(t, "Open to work")).child(
                        div().h(px(28.)).flex().items_center().child(
                            chip_button(
                                t,
                                SharedString::from(format!("spaces-lock-{}", card.id)),
                                if locked { "Locked" } else { "Open" },
                                lock_tip.into(),
                                locked,
                                if locked { t.yellow } else { t.green },
                                part.store.is_some(),
                            )
                            .on_click(cx.listener(move |part, _, _, cx| {
                                change(part, &lock_key, Change::Locked(!locked));
                                cx.notify();
                            })),
                        ),
                    ),
                )
                .child(div().w(px(150.)).flex_none().child(dropdown(
                    part,
                    t,
                    card,
                    Field::Max,
                    "Most doing at once",
                    tasks_word(card.max_doing),
                    None,
                    cx,
                ))),
        )
        .into_any_element()
}

impl Render for SpacesPart {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let mut root = div()
            .id("settings-spaces")
            .debug_selector(|| "settings-spaces".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .overflow_y_scroll()
            .pr(px(8.))
            .pb(px(16.))
            .text_color(t.foreground);
        if let Some(error) = self.error.clone() {
            root = root.child(line(error, t.red, 12.));
        }
        if !self.loaded {
            return root;
        }
        root = root.child(line(INTRO, t.dim, 11.5));
        let cards = self.data.cards.clone();
        if cards.is_empty() {
            root = root.child(line("No spaces found.", t.dim, 12.));
        }
        for card in &cards {
            root = root.child(space_card(self, &t, card, cx));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    fn conn_with_spaces() -> Connection {
        let conn = nixa_store::open_memory().unwrap();
        let one = SpacePatch {
            name: Some("nixa-app".into()),
            folder: Some("/p/nixa-app".into()),
            position: Some(0),
            ..SpacePatch::default()
        };
        let two = SpacePatch {
            name: Some("shop web app".into()),
            folder: Some("/p/shop".into()),
            max_doing: Some(8),
            position: Some(1),
            ..SpacePatch::default()
        };
        store::set_space(&conn, "s1", &one).unwrap();
        store::set_space(&conn, "s2", &two).unwrap();
        conn
    }

    fn card(name: &str, short: &str) -> Card {
        Card {
            id: "s1".into(),
            name: name.into(),
            folder: String::new(),
            short: short.into(),
            auto_start: false,
            locked: false,
            max_doing: MAX_DOING,
        }
    }

    #[test]
    fn short_name_choices() {
        assert_eq!(short_choices("nixa-app"), vec!["NIX", "APP", "NA"]);
        assert_eq!(
            short_choices("shop web app"),
            vec!["SWA", "SHO", "WEB", "APP"]
        );
        assert_eq!(short_choices("myWebAgent"), vec!["MWA", "MY", "WEB", "AGE"]);
        assert_eq!(short_choices("nixa"), vec!["NIX"]);
        let picks = short_picks(&card("nixa-app", ""));
        assert_eq!(picks[0].value, "");
        assert_eq!(picks[0].label, "NIX (automatic)");
        assert_eq!(
            picks.iter().map(|p| p.value.as_str()).collect::<Vec<_>>(),
            ["", "APP", "NA"]
        );
        let picks = short_picks(&card("nixa-app", "ZZ"));
        assert_eq!(picks.last().unwrap().value, "ZZ");
        assert_eq!(short_display(&card("nixa-app", "")), "NIX (automatic)");
        assert_eq!(short_display(&card("", "")), "S1 (automatic)");
        assert_eq!(short_display(&card("nixa-app", "NA")), "NA");
    }

    #[test]
    fn typed_short_names_are_checked() {
        assert_eq!(clean_short(" ab1 "), Some("AB1".into()));
        assert_eq!(clean_short("x"), Some("X".into()));
        assert_eq!(clean_short(""), None);
        assert_eq!(clean_short("abcd"), None);
        assert_eq!(clean_short("a-b"), None);
        assert_eq!(clean_short("é"), None);
    }

    #[test]
    fn pick_lists() {
        assert_eq!(max_picks(3).len(), 5);
        let list = max_picks(8);
        assert_eq!(list.last().unwrap().label, "8 tasks");
        assert_eq!(list[0].label, "1 task");
        assert_eq!(change_of(Field::Max, "4"), Some(Change::Max(4)));
        assert_eq!(change_of(Field::Max, "x"), None);
    }

    #[test]
    fn changes_are_saved_in_the_store() {
        let conn = conn_with_spaces();
        apply(&conn, "s1", &Change::AutoStart(true)).unwrap();
        apply(&conn, "s1", &Change::Locked(true)).unwrap();
        apply(&conn, "s1", &Change::Max(3)).unwrap();
        apply(&conn, "s1", &Change::Short("mo".into())).unwrap();
        assert!(apply(&conn, "s1", &Change::Max(0)).is_err());
        assert!(apply(&conn, "s1", &Change::Max(MAX_DOING_TOP + 1)).is_err());
        assert!(apply(&conn, "s9", &Change::Locked(true)).is_err());
        let data = load(&conn).unwrap();
        let one = &data.cards[0];
        assert!(one.auto_start && one.locked);
        assert_eq!(one.max_doing, 3);
        assert_eq!(one.short, "MO");
        assert_eq!(data.cards[1].max_doing, MAX_DOING);
        let sidebar = crate::spaces::load(&conn).unwrap();
        assert_eq!(crate::spaces::short_name(&sidebar.spaces[0]), "MO");
        assert!(sidebar.spaces[0].locked);
        apply(&conn, "s1", &Change::Short(String::new())).unwrap();
        assert_eq!(load(&conn).unwrap().cards[0].short, "");
        assert!(
            get_setting(&conn, SHORT_KEY)
                .unwrap()
                .unwrap()
                .as_object()
                .unwrap()
                .is_empty()
        );
    }

    fn open(
        cx: &mut TestAppContext,
        store: Option<Connection>,
    ) -> (Entity<SpacesPart>, &mut VisualTestContext) {
        cx.update(nixa_ui::init);
        let (view, vcx) = cx.add_window_view(move |_, cx| build(store, cx));
        vcx.run_until_parked();
        (view, vcx)
    }

    fn click(vcx: &mut VisualTestContext, selector: &'static str) {
        vcx.update(|window, app| window.draw(app).clear());
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
    }

    fn space(view: &Entity<SpacesPart>, vcx: &mut VisualTestContext, id: &str) -> store::Space {
        view.read_with(vcx, |part, _| {
            store::get_space(part.store.as_ref().unwrap(), id)
                .unwrap()
                .unwrap()
        })
    }

    #[gpui::test]
    fn clicks_change_the_space(cx: &mut TestAppContext) {
        let (view, vcx) = open(cx, Some(conn_with_spaces()));
        assert!(view.read_with(vcx, |p, _| p.loaded && p.data.cards.len() == 2));
        click(vcx, "spaces-auto-s1");
        assert!(space(&view, vcx, "s1").auto_start);
        click(vcx, "spaces-lock-s2");
        assert!(space(&view, vcx, "s2").locked);
        click(vcx, "spaces-max-s2");
        assert_eq!(
            view.read_with(vcx, |p, _| p.open.clone()),
            Some(("s2".to_string(), Field::Max))
        );
        click(vcx, "spaces-pick-max-s2-1");
        assert_eq!(space(&view, vcx, "s2").max_doing, 2);
        assert_eq!(view.read_with(vcx, |p, _| p.open.clone()), None);
        click(vcx, "spaces-short-s1");
        click(vcx, "spaces-pick-short-s1-1");
        assert_eq!(
            view.read_with(vcx, |p, _| p.data.cards[0].short.clone()),
            "APP"
        );
        assert_eq!(view.read_with(vcx, |p, _| p.error.clone()), None);
    }

    #[gpui::test]
    fn a_typed_short_name_is_saved(cx: &mut TestAppContext) {
        let (view, vcx) = open(cx, Some(conn_with_spaces()));
        click(vcx, "spaces-short-s2");
        click(vcx, "spaces-pick-short-s2-type");
        assert!(view.read_with(vcx, |p, _| p.typing.is_some()));
        let input = view.read_with(vcx, |p, _| p.typing.as_ref().unwrap().input.clone());
        input.update(vcx, |input, cx| input.set_value("toolong", cx));
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert_eq!(
            view.read_with(vcx, |p, _| p.error.clone()).as_deref(),
            Some("Use 1 to 3 letters or numbers.")
        );
        input.update(vcx, |input, cx| input.set_value("s9", cx));
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(view.read_with(vcx, |p, _| p.typing.is_none()));
        assert_eq!(
            view.read_with(vcx, |p, _| p.data.cards[1].short.clone()),
            "S9"
        );
        click(vcx, "spaces-short-s2");
        click(vcx, "spaces-pick-short-s2-type");
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(view.read_with(vcx, |p, _| p.typing.is_none()));
        assert_eq!(
            view.read_with(vcx, |p, _| p.data.cards[1].short.clone()),
            "S9"
        );
    }

    #[gpui::test]
    fn no_pull_box_is_drawn(cx: &mut TestAppContext) {
        let (_view, vcx) = open(cx, Some(conn_with_spaces()));
        vcx.update(|window, app| window.draw(app).clear());
        assert!(vcx.debug_bounds("spaces-decision-s1").is_none());
        assert!(vcx.debug_bounds("spaces-pull-switch").is_none());
    }

    #[gpui::test]
    fn no_store_says_so(cx: &mut TestAppContext) {
        let (view, vcx) = open(cx, None);
        assert!(view.read_with(vcx, |p, _| !p.loaded));
        assert_eq!(
            view.read_with(vcx, |p, _| p.error.clone()).as_deref(),
            Some(NO_STORE)
        );
    }
}
