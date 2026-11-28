use super::api::Call;
use super::model::{self, Act, Agent, Form, List, Tone};
use super::place;
use crate::actions::ClosePane;
use crate::screens::model_pick::{self, ModelPick};
use crate::screens::settings::parts::memory::text_area::TextArea;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, ScrollHandle, SharedString, Stateful, Task, Window, div, px,
};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::TextInput;
use nixa_ui::{Theme, theme};
use serde_json::Value;
use std::time::Duration;

pub const POLL: Duration = Duration::from_secs(5);
pub const FOCUS_TRIES: usize = 10;
pub const FOCUS_GAP: Duration = Duration::from_millis(300);
pub const CARD_WIDTH: f32 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Window,
    Settings,
}

pub struct Inputs {
    pub title: Entity<TextInput>,
    pub purpose: Entity<TextInput>,
    pub instructions: Entity<TextArea>,
    pub session: Entity<TextInput>,
    pub model: Entity<ModelPick>,
    pub effort: Entity<TextInput>,
}

pub struct CustomAgentsView {
    pub mode: Mode,
    call: Call,
    pub list: List,
    pub loaded: bool,
    pub link_error: String,
    pub error: String,
    pub note: String,
    pub busy: Option<String>,
    pub loading: bool,
    pub editing: Option<String>,
    pub adding: bool,
    pub forgetting: Option<String>,
    pub picking: Option<String>,
    pub add_provider: String,
    pub add_space: String,
    pub profile_agent: String,
    pub opened: Vec<String>,
    pub inputs: Inputs,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _poll: Task<()>,
}

fn input(cx: &mut Context<CustomAgentsView>, placeholder: &'static str) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, placeholder))
}

fn value(input: &Entity<TextInput>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn set(input: &Entity<TextInput>, text: &str, cx: &mut App) {
    let text = text.to_string();
    input.update(cx, |i, cx| i.set_value(text, cx));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn state_color(t: &Theme, state: &str) -> gpui::Hsla {
    match model::tone_of_state(state) {
        "green" => t.green,
        "blue" => t.blue,
        "yellow" => t.yellow,
        _ => t.dim,
    }
}

fn small(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.0))
        .text_color(t.dim)
        .child(text.into())
}

fn badge(id: &str, label: &'static str, color: gpui::Hsla, tip: &'static str) -> Stateful<Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_none()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(color)
        .text_color(color)
        .text_size(px(10.5))
        .tooltip(tooltip(tip))
        .child(label)
}

fn chip(t: &Theme, id: String, label: String, on: bool, tip: String) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .px(px(7.0))
        .py(px(3.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .border_1()
        .border_color(if on { t.green } else { t.border })
        .text_color(if on { t.green } else { t.dim })
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(tip))
        .child(label)
}

fn truncated(t: &Theme, id: (&'static str, String), text: String) -> Stateful<Div> {
    small(t, text.clone())
        .id(SharedString::from(format!("{}-{}", id.0, id.1)))
        .truncate()
        .tooltip(tooltip(text))
}

fn labeled(t: &Theme, label: &'static str, field: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .flex_1()
        .min_w(px(220.0))
        .child(small(t, label))
        .child(field)
}

fn pick_button(
    t: &Theme,
    id: String,
    label: impl Into<SharedString>,
    tone: Tone,
    tip: String,
) -> Stateful<Div> {
    let kind = if tone == Tone::Danger {
        ButtonKind::Danger
    } else {
        ButtonKind::Secondary
    };
    let selector = id.clone();
    let el = button(t, SharedString::from(id), label, kind)
        .debug_selector(move || selector.clone())
        .tooltip(tooltip(tip));
    if tone == Tone::Good {
        el.text_color(t.green)
    } else {
        el
    }
}

impl CustomAgentsView {
    pub fn new(mode: Mode, call: Call, cx: &mut Context<Self>) -> Self {
        let inputs = Inputs {
            title: input(cx, "Title (name tag)"),
            purpose: input(cx, "Purpose, for example news researcher"),
            instructions: cx.new(|cx| TextArea::new(cx, "Standing instructions").auto_rows(3, 3)),
            session: input(
                cx,
                "Session id, optional: paste one to let Nixa resume a saved session",
            ),
            model: model_pick::new("edit-model", "", "", model_pick::brain_load(), cx),
            effort: input(cx, "Effort, for example high, or empty for default"),
        };
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |view, cx| {
                    if view.idle() {
                        view.load(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut view = CustomAgentsView {
            mode,
            call,
            list: List::default(),
            loaded: false,
            link_error: String::new(),
            error: String::new(),
            note: String::new(),
            busy: None,
            loading: false,
            editing: None,
            adding: false,
            forgetting: None,
            picking: None,
            add_provider: String::new(),
            add_space: String::new(),
            profile_agent: String::new(),
            opened: Vec::new(),
            inputs,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _poll: poll,
        };
        view.load(cx);
        view
    }

    pub fn idle(&self) -> bool {
        self.busy.is_none() && self.editing.is_none() && !self.adding && !self.loading
    }

    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.list.agents.iter().find(|a| a.id == id)
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let call = self.call.clone();
        let job = cx
            .background_executor()
            .spawn(async move { call("list", None) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                view.loading = false;
                view.loaded = true;
                match out {
                    Ok(value) => {
                        view.list = model::parse_list(&value);
                        view.link_error.clear();
                    }
                    Err(e) => view.link_error = e,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn act(&mut self, id: &str, action: &str, body: Value, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(id.to_string());
        self.error.clear();
        self.note.clear();
        cx.notify();
        let call = self.call.clone();
        let action = action.to_string();
        let job = cx
            .background_executor()
            .spawn(async move { call(&action, Some(body)) });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                view.busy = None;
                match out {
                    Ok(value) => {
                        view.note = model::reply_text(&value);
                        if let Some(terminal) = model::used_terminal(&value) {
                            view.open_terminal(terminal, cx);
                        }
                    }
                    Err(e) => view.error = e,
                }
                view.load(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_terminal(&mut self, terminal: String, cx: &mut Context<Self>) {
        self.opened.push(terminal.clone());
        self.error.clear();
        if place::focus(&terminal, cx) {
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            for _ in 0..FOCUS_TRIES {
                cx.background_executor().timer(FOCUS_GAP).await;
                if cx.update(|cx| place::focus(&terminal, cx)).unwrap_or(true) {
                    return;
                }
            }
            this.update(cx, |view, cx| {
                if view.opened.last() == Some(&terminal) {
                    view.error = format!("Could not open terminal {terminal}. It is missing from the workspace tabs.");
                    cx.notify();
                }
            }).ok();
        })
        .detach();
    }

    pub fn press(&mut self, act: Act, id: &str, cx: &mut Context<Self>) {
        let Some(agent) = self.agent(id).cloned() else {
            return;
        };
        match act {
            Act::Open => self.open_terminal(agent.terminal.clone(), cx),
            Act::Edit => self.start_edit(&agent, cx),
            Act::Workflow => {
                self.picking = if self.picking.as_deref() == Some(id) {
                    None
                } else {
                    Some(id.to_string())
                };
                cx.notify();
            }
            Act::Forget => {
                self.forgetting = Some(id.to_string());
                cx.notify();
            }
            other => {
                if other == Act::ForgetYes {
                    self.forgetting = None;
                }
                let space = place::active_space(cx);
                if let Some((action, body)) = model::act_request(other, &agent, space.as_deref()) {
                    self.act(id, action, body, cx);
                }
            }
        }
    }

    pub fn pick_workflow(&mut self, id: &str, name: Option<String>, cx: &mut Context<Self>) {
        let Some(agent) = self.agent(id).cloned() else {
            return;
        };
        self.picking = None;
        let (action, body) = model::workflow_request(&agent, name.as_deref());
        self.act(id, action, body, cx);
    }

    pub fn start_edit(&mut self, agent: &Agent, cx: &mut Context<Self>) {
        self.adding = false;
        self.editing = Some(agent.id.clone());
        let form = model::form_of(agent);
        self.profile_agent = agent
            .start
            .as_ref()
            .map(|p| p.agent.clone())
            .unwrap_or_default();
        self.fill(&form, cx);
        cx.notify();
    }

    pub fn start_add(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.adding = true;
        self.add_provider.clear();
        self.add_space.clear();
        self.fill(&Form::default(), cx);
        cx.notify();
    }

    fn fill(&mut self, form: &Form, cx: &mut Context<Self>) {
        set(&self.inputs.title, &form.title, cx);
        set(&self.inputs.purpose, &form.purpose, cx);
        self.inputs.instructions.update(cx, |input, cx| input.set_value(form.instructions.clone(), cx));
        set(&self.inputs.session, &form.session, cx);
        let (agent, value) = (self.profile_agent.clone(), form.model.clone());
        self.inputs.model.update(cx, |p, cx| p.set_agent(&agent, &value, cx));
        set(&self.inputs.effort, &form.effort, cx);
    }

    pub fn form(&self, cx: &App) -> Form {
        Form {
            title: value(&self.inputs.title, cx),
            purpose: value(&self.inputs.purpose, cx),
            instructions: self.inputs.instructions.read(cx).value().to_string(),
            session: value(&self.inputs.session, cx),
            model: self.inputs.model.read(cx).value.trim().to_string(),
            effort: value(&self.inputs.effort, cx),
        }
    }

    pub fn save_add(&mut self, cx: &mut Context<Self>) {
        match model::add_body(&self.form(cx), &self.add_provider, &self.add_space) {
            Ok(body) => {
                self.adding = false;
                self.act("add", "add", body, cx);
            }
            Err(e) => {
                self.error = e;
                cx.notify();
            }
        }
    }

    pub fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(agent) = self
            .editing
            .as_deref()
            .and_then(|id| self.agent(id))
            .cloned()
        else {
            self.editing = None;
            cx.notify();
            return;
        };
        let body = model::edit_body(&agent, &self.form(cx), &self.profile_agent);
        self.editing = None;
        self.act(&agent.id, "edit", body, cx);
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.adding = false;
        self.editing = None;
        cx.notify();
    }

    fn card(&self, t: &Theme, agent: &Agent, cx: &mut Context<Self>) -> AnyElement {
        let id = agent.id.clone();
        let color = state_color(t, &agent.state);
        let busy = self.busy.as_deref() == Some(id.as_str());
        let mut actions = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(6.0))
            .items_center();
        for act in model::acts(agent, self.forgetting.as_deref() == Some(id.as_str())) {
            let key = format!("{}-{id}", model::act_key(act));
            let tip = model::act_tip(act, agent);
            let agent_id = id.clone();
            actions = actions.child(
                pick_button(
                    t,
                    key,
                    model::act_label(act, agent, busy),
                    model::act_tone(act),
                    tip,
                )
                .on_click(
                    cx.listener(move |view, _: &ClickEvent, _, cx| view.press(act, &agent_id, cx)),
                ),
            );
        }
        let picking = self.picking.as_deref() == Some(id.as_str());
        let mut chips = div().flex().flex_row().flex_wrap().gap(px(5.0));
        if picking {
            let agent_id = id.clone();
            chips = chips.child(
                chip(
                    t,
                    format!("wf-none-{id}"),
                    "None".into(),
                    agent.workflow.is_empty(),
                    "No default workflow: tasks use the normal flow".into(),
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.pick_workflow(&agent_id, None, cx)
                })),
            );
            for w in &self.list.workflows {
                let agent_id = id.clone();
                let name = w.name.clone();
                let tip = if w.title.is_empty() {
                    w.name.clone()
                } else {
                    w.title.clone()
                };
                chips = chips.child(
                    chip(
                        t,
                        format!("wf-{id}-{}", w.name),
                        w.name.clone(),
                        agent.workflow == w.name,
                        tip,
                    )
                    .on_click(cx.listener(
                        move |view, _: &ClickEvent, _, cx| {
                            view.pick_workflow(&agent_id, Some(name.clone()), cx)
                        },
                    )),
                );
            }
        }
        let card_id = format!("agent-{id}");
        div()
            .id(SharedString::from(card_id.clone()))
            .debug_selector(move || card_id.clone())
            .w(px(CARD_WIDTH))
            .flex_none()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .items_center()
                    .child(div().size(px(9.0)).rounded(px(5.0)).flex_none().bg(color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(14.0))
                            .child(agent.title.clone()),
                    )
                    .when(model::reviewer(agent), |el| {
                        el.child(badge(
                            &format!("reviewer-{id}"),
                            "Reviewer",
                            t.magenta,
                            "Review agent of its space: Nixa passes finished code work to it once",
                        ))
                    })
                    .when(agent.off, |el| {
                        el.child(badge(
                            &format!("offmark-{id}"),
                            "Off",
                            t.dim,
                            "Off: Nixa does not pick or start it",
                        ))
                    })
                    .child(small(t, id.clone())),
            )
            .child(div().text_color(t.foreground).child(agent.purpose.clone()))
            .when(!agent.instructions.is_empty(), |el| {
                el.child(small(t, agent.instructions.clone()))
            })
            .when_some(model::job_line(agent), |el, line| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(t.foreground)
                        .child(line),
                )
            })
            .child(truncated(t, ("agent-place", id.clone()), model::place_line(agent)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .text_size(px(11.0))
                    .child(
                        div()
                            .text_color(color)
                            .child(model::state_text(&agent.state)),
                    )
                    .child(div().text_color(t.dim).child(format!(
                        "· {} · used {}",
                        model::where_text(agent),
                        model::ago(now_ms(), agent.last_used)
                    ))),
            )
            .when(picking, |el| el.child(chips))
            .when_some(model::warning(agent), |el, warn| {
                let sel = format!("warn-{id}");
                el.child(
                    div()
                        .debug_selector(move || sel.clone())
                        .text_size(px(11.0))
                        .text_color(t.red)
                        .child(warn),
                )
            })
            .when(!agent.session.is_empty(), |el| {
                el.child(truncated(t, ("agent-session", id.clone()), format!("Session {}", agent.session)))
            })
            .child(actions)
            .into_any_element()
    }

    fn add_form(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut providers = div().flex().flex_row().flex_wrap().gap(px(5.0)).child(
            chip(
                t,
                "add-provider-none".into(),
                "None".into(),
                self.add_provider.is_empty(),
                "No provider saved; Nixa cannot resume this agent until one is set".into(),
            )
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.add_provider.clear();
                cx.notify();
            })),
        );
        for p in model::providers(&self.list) {
            let pick = p.clone();
            providers = providers.child(
                chip(
                    t,
                    format!("add-provider-{p}"),
                    p.clone(),
                    self.add_provider == p,
                    format!("Resume this agent as a {p} session when it is closed"),
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.add_provider = pick.clone();
                    cx.notify();
                })),
            );
        }
        let mut spaces = div().flex().flex_row().flex_wrap().gap(px(5.0)).child(
            chip(
                t,
                "add-space-none".into(),
                "None".into(),
                self.add_space.is_empty(),
                "No space saved; Nixa cannot resume this agent until it has one".into(),
            )
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.add_space.clear();
                cx.notify();
            })),
        );
        for s in &self.list.spaces {
            let pick = s.id.clone();
            let name = if s.name.is_empty() {
                s.id.clone()
            } else {
                s.name.clone()
            };
            spaces = spaces.child(
                chip(
                    t,
                    format!("add-space-{}", s.id),
                    name.clone(),
                    self.add_space == s.id,
                    format!("Resume this agent in space {name}"),
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.add_space = pick.clone();
                    cx.notify();
                })),
            );
        }
        div()
            .px(px(14.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Add a custom agent by hand"),
            )
            .child(small(t, model::ADD_ABOUT))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(labeled(t, "Title (name tag)", self.inputs.title.clone()))
                    .child(labeled(t, "Purpose", self.inputs.purpose.clone())),
            )
            .child(labeled(
                t,
                "Standing instructions (optional)",
                self.inputs.instructions.clone(),
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(labeled(
                        t,
                        "Session id (optional)",
                        self.inputs.session.clone(),
                    ))
                    .child(labeled(t, "Provider (optional, for resume)", providers))
                    .child(labeled(t, "Space (optional, where it resumes)", spaces)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(
                        pick_button(
                            t,
                            "add-save".into(),
                            "Add agent",
                            Tone::Good,
                            "Save this custom agent".into(),
                        )
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.save_add(cx))),
                    )
                    .child(
                        pick_button(
                            t,
                            "add-cancel".into(),
                            "Cancel",
                            Tone::Plain,
                            "Close without saving".into(),
                        )
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.cancel(cx))),
                    ),
            )
            .into_any_element()
    }

    fn edit_form(&self, t: &Theme, agent: &Agent, cx: &mut Context<Self>) -> AnyElement {
        let mut form = div()
            .px(px(14.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Edit {}", agent.id)),
            )
            .child(self.inputs.title.clone())
            .child(self.inputs.purpose.clone())
            .child(self.inputs.instructions.clone())
            .child(self.inputs.session.clone());
        if agent.start.is_some() {
            let mut agents = div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(5.0))
                .items_center()
                .child(small(t, "Agent"));
            for p in model::profile_agents(&self.list) {
                let pick = p.clone();
                agents = agents.child(
                    chip(
                        t,
                        format!("edit-agent-{p}"),
                        p.clone(),
                        self.profile_agent == p,
                        format!(
                            "Start this agent with {p}. A change of agent drops the saved session."
                        ),
                    )
                    .on_click(cx.listener(
                        move |view, _: &ClickEvent, _, cx| {
                            view.profile_agent = pick.clone();
                            view.inputs.model.update(cx, |p, cx| {
                                let keep = p.value.clone();
                                p.set_agent(&pick, &keep, cx);
                            });
                            cx.notify();
                        },
                    )),
                );
            }
            form = form.child(agents).child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(labeled(t, "Model", self.inputs.model.clone()))
                    .child(labeled(t, "Effort", self.inputs.effort.clone())),
            );
            let efforts = self.inputs.model.read(cx).efforts();
            if !efforts.is_empty() {
                let current = value(&self.inputs.effort, cx);
                let mut row = div().flex().flex_row().flex_wrap().gap(px(5.0)).items_center().child(small(t, "Efforts"));
                for e in efforts {
                    let pick = e.clone();
                    row = row.child(
                        chip(t, format!("edit-effort-{e}"), e.clone(), current == e, format!("Start with effort {e}"))
                            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                                set(&view.inputs.effort, &pick, cx);
                                cx.notify();
                            })),
                    );
                }
                form = form.child(row);
            }
        }
        form.child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(
                    pick_button(
                        t,
                        "edit-save".into(),
                        "Save",
                        Tone::Good,
                        "Save the changes".into(),
                    )
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.save_edit(cx))),
                )
                .child(
                    pick_button(
                        t,
                        "edit-cancel".into(),
                        "Cancel",
                        Tone::Plain,
                        "Close without saving".into(),
                    )
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.cancel(cx))),
                ),
        )
        .into_any_element()
    }

    fn teams(&self, t: &Theme) -> AnyElement {
        let mut out = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .pt(px(14.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.0))
                    .child("Teams"),
            )
            .child(small(t, model::TEAMS_ABOUT));
        for team in &self.list.teams {
            out = out.child(
                div()
                    .p(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.0))
                            .items_center()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(team.name.clone()),
                            )
                            .child(small(t, team.id.clone()))
                            .when(team.flow == "pr", |el| el.child(small(t, "PR flow"))),
                    )
                    .when(!team.purpose.is_empty(), |el| {
                        el.child(small(t, team.purpose.clone()))
                    })
                    .child(
                        div()
                            .text_size(px(11.5))
                            .child(model::team_line(team, &self.list.agents)),
                    ),
            );
        }
        out.into_any_element()
    }
}

impl Focusable for CustomAgentsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CustomAgentsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let editing = self
            .editing
            .as_deref()
            .and_then(|id| self.agent(id))
            .cloned();
        let window_mode = self.mode == Mode::Window;
        let agents = self.list.agents.clone();
        let cards: Vec<AnyElement> = agents.iter().map(|a| self.card(&t, a, cx)).collect();
        let empty = if !self.loaded || !self.link_error.is_empty() && self.list.agents.is_empty() {
            model::CONNECTING_TEXT
        } else {
            model::EMPTY_TEXT
        };
        let add = self.adding.then(|| self.add_form(&t, cx));
        let edit = editing.as_ref().map(|a| self.edit_form(&t, a, cx));
        let teams = (!self.list.teams.is_empty()).then(|| self.teams(&t));
        let error = if self.error.is_empty() {
            self.link_error.clone()
        } else {
            self.error.clone()
        };
        div()
            .id("custom-agents")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(12.0))
            .when(window_mode, |el| {
                el.on_action(cx.listener(|_, _: &ClosePane, window, _| window.remove_window()))
                    .on_key_down(cx.listener(|view, e: &KeyDownEvent, window, cx| {
                        if e.keystroke.key == "escape" {
                            if view.adding || view.editing.is_some() {
                                view.cancel(cx);
                            } else {
                                window.remove_window();
                            }
                        }
                    }))
            })
            .child(
                div()
                    .px(px(14.0))
                    .py(px(10.0))
                    .flex()
                    .flex_row()
                    .gap(px(10.0))
                    .items_center()
                    .border_b_1()
                    .border_color(t.border)
                    .when(window_mode, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(16.0))
                                .child("Custom agents"),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(t.dim)
                            .child(model::ABOUT_TEXT),
                    )
                    .child(
                        pick_button(
                            &t,
                            "add".into(),
                            "Add agent",
                            Tone::Good,
                            "Define a custom agent by hand, without an open terminal".into(),
                        )
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.start_add(cx))),
                    )
                    .child(
                        pick_button(
                            &t,
                            "reload".into(),
                            "Reload",
                            Tone::Plain,
                            "Read the list again".into(),
                        )
                        .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.load(cx))),
                    ),
            )
            .when(!error.is_empty(), |el| {
                el.child(
                    div()
                        .debug_selector(|| "agents-error".into())
                        .px(px(14.0))
                        .py(px(6.0))
                        .text_color(t.red)
                        .child(error),
                )
            })
            .when(!self.note.is_empty(), |el| {
                el.child(
                    div()
                        .debug_selector(|| "agents-note".into())
                        .px(px(14.0))
                        .py(px(6.0))
                        .text_color(t.green)
                        .child(self.note.clone()),
                )
            })
            .children(add)
            .children(edit)
            .child(
                div()
                    .id("agents-scroll")
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(14.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(12.0))
                            .items_start()
                            .children(cards)
                            .when(self.list.agents.is_empty(), |el| {
                                el.child(
                                    div()
                                        .debug_selector(|| "agents-empty".into())
                                        .w_full()
                                        .min_w_0()
                                        .whitespace_normal()
                                        .text_color(t.dim)
                                        .child(empty),
                                )
                            }),
                    )
                    .children(teams)
                    .child(div().w_full().h(px(24.0)).flex_none()),
            )
            .child(
                div()
                    .px(px(14.0))
                    .py(px(6.0))
                    .text_color(t.dim)
                    .text_size(px(11.0))
                    .child(model::FOOT_TEXT),
            )
    }
}
