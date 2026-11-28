use super::model::{self, Deps, Poll};
use gpui::prelude::*;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, SharedString, Window, div, px,
};
use nixa_git::ai::{AiSettings, Found};
use nixa_ui::text;
use nixa_ui::theme;

pub const TITLE: &str = "AI agent for commit messages and reviews";
pub const LOOKING: &str = "Looking for AI tools…";
pub const NONE_FOUND: &str = "No AI tool was found. Install Claude Code, Codex or another one, then open this again.";
pub const SETTINGS_ROW: &str = "AI settings…";
pub const WIDTH: f32 = 300.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentMenuEvent {
    Picked(String),
    Settings,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Agent(String),
    Settings,
}

pub struct AgentMenu {
    deps: Deps,
    pub agents: Option<Vec<Found>>,
    pub settings: AiSettings,
    pub at: usize,
    pub busy: Option<String>,
    pub error: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<AgentMenuEvent> for AgentMenu {}

impl Focusable for AgentMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn items(agents: Option<&[Found]>) -> Vec<Item> {
    agents
        .unwrap_or_default()
        .iter()
        .map(|a| Item::Agent(a.id.clone()))
        .chain(std::iter::once(Item::Settings))
        .collect()
}

pub fn step(at: usize, count: usize, by: isize) -> usize {
    if count == 0 {
        return 0;
    }
    (at as isize + by).rem_euclid(count as isize) as usize
}

impl AgentMenu {
    pub fn new(deps: Deps, cx: &mut Context<Self>) -> AgentMenu {
        let mut menu = AgentMenu {
            deps,
            agents: None,
            settings: AiSettings::default(),
            at: 0,
            busy: None,
            error: None,
            focus: cx.focus_handle(),
        };
        menu.load(cx);
        menu
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.agents = None;
        let load = self.deps.load.clone();
        let detect = self.deps.detect.clone();
        let rx = model::off_thread(move || (load(), detect()));
        cx.spawn(async move |this, cx| {
            loop {
                match model::poll(&rx) {
                    Poll::Ready((settings, found)) => {
                        this.update(cx, |m, cx| {
                            let list: Vec<Found> = found.into_iter().filter(|a| a.installed).collect();
                            m.at = list.iter().position(|a| a.id == settings.agent).unwrap_or(0);
                            m.settings = settings;
                            m.agents = Some(list);
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    Poll::Waiting => cx.background_executor().timer(model::POLL).await,
                    Poll::Gone => break,
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn items(&self) -> Vec<Item> {
        items(self.agents.as_deref())
    }

    pub fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        match self.items().get(index) {
            Some(Item::Settings) => cx.emit(AgentMenuEvent::Settings),
            Some(Item::Agent(id)) if *id == self.settings.agent => cx.emit(AgentMenuEvent::Close),
            Some(Item::Agent(id)) => self.save(id.clone(), cx),
            None => {}
        }
    }

    fn save(&mut self, id: String, cx: &mut Context<Self>) {
        self.busy = Some(id.clone());
        self.error = None;
        let pick = self.deps.pick.clone();
        let chosen = id.clone();
        let rx = model::off_thread(move || pick(&chosen));
        cx.spawn(async move |this, cx| {
            loop {
                match model::poll(&rx) {
                    Poll::Ready(result) => {
                        this.update(cx, |m, cx| {
                            m.busy = None;
                            match result {
                                Ok(()) => {
                                    m.settings.agent = id.clone();
                                    cx.emit(AgentMenuEvent::Picked(id.clone()));
                                }
                                Err(e) => m.error = Some(format!("Could not save: {e}")),
                            }
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    Poll::Waiting => cx.background_executor().timer(model::POLL).await,
                    Poll::Gone => break,
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn key(&mut self, e: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let count = self.items().len();
        match e.keystroke.key.as_str() {
            "down" => self.at = step(self.at, count, 1),
            "up" => self.at = step(self.at, count, -1),
            "enter" => self.choose(self.at, cx),
            "escape" => cx.emit(AgentMenuEvent::Close),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for AgentMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let items = self.items();
        let settings_at = items.len() - 1;
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        match &self.agents {
            None => rows.push(div().px(px(8.0)).py(px(6.0)).child(text::muted(&t, LOOKING)).into_any_element()),
            Some(list) if list.is_empty() => rows.push(
                div()
                    .id("agent-menu-none")
                    .debug_selector(|| "agent-menu-none".into())
                    .px(px(8.0))
                    .py(px(6.0))
                    .child(text::muted(&t, NONE_FOUND))
                    .into_any_element(),
            ),
            Some(list) => {
                for (index, agent) in list.iter().enumerate() {
                    let current = agent.id == self.settings.agent;
                    let name = if self.busy.as_deref() == Some(agent.id.as_str()) {
                        "Saving…".to_string()
                    } else {
                        agent.label.clone()
                    };
                    let selector = format!("agent-menu-{}", agent.id);
                    let lit = index == self.at || self.busy.as_deref() == Some(agent.id.as_str());
                    let hover = t.hover;
                    rows.push(
                        div()
                            .id(SharedString::from(selector.clone()))
                            .debug_selector(move || selector.clone())
                            .min_h(px(38.0))
                            .flex_none()
                            .px(px(8.0))
                            .py(px(4.0))
                            .gap(px(9.0))
                            .rounded(px(7.0))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .when(lit, |el| el.bg(t.selected))
                            .hover(move |s| s.bg(hover))
                            .on_click(cx.listener(move |m, _, _, cx| m.choose(index, cx)))
                            .child(div().w(px(14.0)).flex_none().text_color(t.accent).child(if current { "✓" } else { "" }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(text::MENU))
                                            .text_color(if current { t.accent } else { t.foreground })
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.0))
                                            .text_color(t.dim)
                                            .child(model::model_text(&self.settings, &agent.id)),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        let hover = t.hover;
        div()
            .id("agent-menu")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .p(px(5.0))
            .rounded(px(8.0))
            .bg(t.popover)
            .border_1()
            .border_color(t.border)
            .shadow(nixa_ui::popover::shadow(&t))
            .text_color(t.foreground)
            .text_size(px(text::MENU))
            .child(div().flex_none().px(px(8.0)).pt(px(4.0)).pb(px(6.0)).text_size(px(text::SM)).text_color(t.muted).child(TITLE))
            .child(div().id("agent-menu-list").flex().flex_col().max_h(px(320.0)).overflow_y_scroll().children(rows))
            .when_some(self.error.clone(), |el, e| {
                el.child(div().px(px(8.0)).py(px(4.0)).text_size(px(text::SM)).text_color(t.red).child(e))
            })
            .child(div().flex_none().mx(px(7.0)).my(px(4.0)).h(px(1.0)).bg(t.border))
            .child(
                div()
                    .id("agent-menu-settings")
                    .debug_selector(|| "agent-menu-settings".into())
                    .h(px(30.0))
                    .flex_none()
                    .px(px(8.0))
                    .gap(px(9.0))
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(self.at == settings_at, |el| el.bg(t.selected))
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |m, _, _, cx| m.choose(settings_at, cx)))
                    .child(div().w(px(14.0)).flex_none().text_color(t.muted).child("⚙"))
                    .child(div().flex_1().min_w_0().truncate().child(SETTINGS_ROW)),
            )
    }
}
