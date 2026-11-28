mod view;

#[cfg(test)]
mod tests;

pub use view::glyph;

use crate::links::brain::{self, Link};
use crate::programs::{self, Fields, Program};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    Subscription, Window,
};
use nixa_ui::text_input::{InputEvent, TextInput};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

pub const WIDTH: f32 = 250.0;
pub const SETTINGS_PART: &str = "General";
const TEAM_WAIT: Duration = Duration::from_secs(4);
const START_WAIT: Duration = Duration::from_secs(150);
const ROW: f32 = 31.0;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuEvent {
    Close,
    Terminal,
    Start { name: String, run: Vec<String> },
    Settings(&'static str),
    Notice(String),
}

pub type OnEvent = Rc<dyn Fn(MenuEvent, &mut App)>;

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub program: Program,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Custom {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub state: String,
    pub agent: String,
    pub model: String,
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub key: Option<String>,
    pub title: String,
    pub shown: Fields,
    pub icon: Option<String>,
    pub icons: Vec<String>,
}

pub struct Loaded {
    pub agents: Vec<Agent>,
    pub mine: Vec<String>,
    pub failed: bool,
    pub team: Result<Vec<Custom>, String>,
}

pub struct Setup {
    pub file: PathBuf,
    pub dirs: Vec<PathBuf>,
    pub link: Option<Link>,
    pub space: Option<String>,
    pub on_event: OnEvent,
}

pub struct NewMenu {
    scroll: gpui::ScrollHandle,
    pub agents: Vec<Agent>,
    pub mine: Vec<String>,
    pub customs: Vec<Custom>,
    pub team_error: Option<String>,
    pub loaded: bool,
    pub failed: bool,
    pub form: Option<Form>,
    pub confirm: Option<String>,
    pub error: String,
    pub busy: bool,
    pub name_input: Entity<TextInput>,
    pub cmd_input: Entity<TextInput>,
    pub focus: FocusHandle,
    pub refocus: bool,
    pub file: PathBuf,
    pub dirs: Vec<PathBuf>,
    pub link: Option<Link>,
    pub space: Option<String>,
    on_event: OnEvent,
    _subs: Vec<Subscription>,
}

impl Focusable for NewMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NewMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus);
        }
        view::render(self, cx)
    }
}

pub fn new(setup: Setup, window: &mut Window, cx: &mut App) -> Entity<NewMenu> {
    let view = cx.new(|cx| build(setup, window, cx));
    let handle = view.read(cx).focus.clone();
    window.focus(&handle);
    load(&view, cx);
    view
}

pub fn build(setup: Setup, window: &mut Window, cx: &mut Context<NewMenu>) -> NewMenu {
    let name_input = cx.new(|cx| TextInput::new(cx, "My agent"));
    let cmd_input = cx.new(|cx| TextInput::new(cx, "omp --model zai/glm-5.3"));
    let subs = vec![
        cx.subscribe_in(&name_input, window, on_input),
        cx.subscribe_in(&cmd_input, window, on_input),
    ];
    NewMenu {
        scroll: gpui::ScrollHandle::new(),
        agents: Vec::new(),
        mine: Vec::new(),
        customs: Vec::new(),
        team_error: None,
        loaded: false,
        failed: false,
        form: None,
        confirm: None,
        error: String::new(),
        busy: false,
        name_input,
        cmd_input,
        focus: cx.focus_handle(),
        refocus: false,
        file: setup.file,
        dirs: setup.dirs,
        link: setup.link,
        space: setup.space,
        on_event: setup.on_event,
        _subs: subs,
    }
}

fn on_input(
    menu: &mut NewMenu,
    _: &Entity<TextInput>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<NewMenu>,
) {
    match event {
        InputEvent::Submit(_) => save(menu, cx),
        InputEvent::Cancel => {
            close_form(menu, cx);
            window.focus(&menu.focus);
        }
        InputEvent::Changed(_) => {}
    }
}

pub fn emit(menu: &NewMenu, event: MenuEvent, cx: &mut App) {
    let on_event = menu.on_event.clone();
    on_event(event, cx);
}

pub fn installed(menu: &NewMenu) -> Vec<&Agent> {
    menu.agents.iter().filter(|a| a.installed).collect()
}

pub fn choices(menu: &NewMenu) -> Vec<&Agent> {
    let installed = menu.agents.iter().filter(|a| a.installed);
    let mine = menu
        .agents
        .iter()
        .filter(|a| !a.installed && menu.mine.contains(&a.program.key));
    let other = menu
        .agents
        .iter()
        .filter(|a| !a.installed && !menu.mine.contains(&a.program.key))
        .take(2);
    installed.chain(mine).chain(other).collect()
}

pub fn height(menu: &NewMenu) -> f32 {
    if menu.form.is_some() {
        return 340.0;
    }
    if menu.confirm.is_some() {
        return 170.0;
    }
    let team = if menu.team_error.is_some() {
        28.0
    } else if menu.customs.is_empty() {
        0.0
    } else {
        52.0 + menu.customs.len() as f32 * 36.0
    };
    let rows = choices(menu).len().max(1) as f32 * ROW;
    (260.0 + rows + team).min(640.0)
}

pub fn taken_keys(agents: &[Agent]) -> Vec<String> {
    let mut keys: Vec<String> = programs::BUILT_IN.iter().map(|b| b.0.to_string()).collect();
    for agent in agents {
        if !keys.contains(&agent.program.key) {
            keys.push(agent.program.key.clone());
        }
    }
    keys
}

pub fn profile_text(c: &Custom) -> String {
    let or = |v: &str, d: &str| {
        if v.trim().is_empty() {
            d.to_string()
        } else {
            v.to_string()
        }
    };
    [
        or(&c.agent, "agent"),
        or(&c.model, "default model"),
        or(&c.effort, "default effort"),
    ]
    .join(" · ")
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn customs_of(data: &Value) -> Vec<Custom> {
    let list = data
        .get("agents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.iter()
        .filter(|a| a.get("start").is_some_and(Value::is_object))
        .filter(|a| a.get("off").and_then(Value::as_bool) != Some(true))
        .filter_map(|a| {
            let start = a.get("start")?;
            let id = text(a, "id");
            if id.is_empty() {
                return None;
            }
            let title = text(a, "title");
            Some(Custom {
                title: if title.is_empty() { id.clone() } else { title },
                id,
                purpose: text(a, "purpose"),
                state: text(a, "state"),
                agent: text(start, "agent"),
                model: text(start, "model"),
                effort: text(start, "effort"),
            })
        })
        .collect()
}

pub fn gather(file: &std::path::Path, dirs: &[PathBuf], link: Option<&Link>) -> Loaded {
    let (text, failed) = match programs::read(file) {
        Ok(text) => (text, false),
        Err(_) => (None, true),
    };
    let agents = programs::known(text.as_deref())
        .into_iter()
        .map(|program| Agent {
            installed: programs::installed(&program.run, dirs),
            program,
        })
        .collect();
    let team = match link {
        Some(link) => link
            .request_for("custom_agents", json!({"action": "list"}), TEAM_WAIT)
            .map(|data| customs_of(&data))
            .map_err(|e| format!("{e:#}")),
        None => Err(brain::NOT_CONNECTED.to_string()),
    };
    Loaded {
        agents,
        mine: programs::user_keys(text.as_deref()),
        failed,
        team,
    }
}

pub fn take(menu: &mut NewMenu, loaded: Loaded) {
    menu.agents = loaded.agents;
    menu.mine = loaded.mine;
    menu.failed = loaded.failed;
    match loaded.team {
        Ok(list) => {
            menu.customs = list;
            menu.team_error = None;
        }
        Err(e) => {
            menu.customs.clear();
            menu.team_error = Some(e);
        }
    }
    menu.loaded = true;
}

pub fn reload(menu: &NewMenu, cx: &mut Context<NewMenu>) {
    let (file, dirs, link) = (menu.file.clone(), menu.dirs.clone(), menu.link.clone());
    let task = cx
        .background_executor()
        .spawn(async move { gather(&file, &dirs, link.as_ref()) });
    cx.spawn(async move |this, cx| {
        let loaded = task.await;
        this.update(cx, |menu, cx| {
            take(menu, loaded);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn load(view: &Entity<NewMenu>, cx: &mut App) {
    view.update(cx, |menu, cx| reload(menu, cx));
}

pub fn open_form(
    menu: &mut NewMenu,
    key: Option<&str>,
    window: &mut Window,
    cx: &mut Context<NewMenu>,
) {
    let agent = key.and_then(|k| menu.agents.iter().find(|a| a.program.key == k).cloned());
    let choices: Vec<String> = programs::ICONS
        .iter()
        .map(|n| programs::icon_path(n))
        .collect();
    let icon = match &agent {
        Some(a) => a.program.icon.clone(),
        None => Some(programs::icon_path("agent")),
    };
    let icons = match &icon {
        Some(i) if !choices.contains(i) => std::iter::once(i.clone()).chain(choices).collect(),
        _ => choices,
    };
    menu.form = Some(Form {
        key: agent.as_ref().map(|a| a.program.key.clone()),
        title: match &agent {
            Some(a) => format!("Edit {}", a.program.name),
            None => "Add agent".into(),
        },
        shown: match &agent {
            Some(a) => Fields {
                name: Some(a.program.name.clone()),
                icon: a.program.icon.clone(),
                run: Some(a.program.run.clone()),
            },
            None => Fields::default(),
        },
        icon,
        icons,
    });
    menu.error.clear();
    menu.busy = false;
    let name = agent
        .as_ref()
        .map(|a| a.program.name.clone())
        .unwrap_or_default();
    let line = agent
        .as_ref()
        .map(|a| programs::join_command(&a.program.run))
        .unwrap_or_default();
    menu.name_input.update(cx, |i, cx| i.set_value(name, cx));
    menu.cmd_input.update(cx, |i, cx| i.set_value(line, cx));
    let handle = menu.name_input.focus_handle(cx);
    window.focus(&handle);
    cx.notify();
}

pub fn close_form(menu: &mut NewMenu, cx: &mut Context<NewMenu>) {
    menu.form = None;
    menu.error.clear();
    menu.busy = false;
    menu.refocus = true;
    cx.notify();
}

pub fn pick_icon(menu: &mut NewMenu, icon: String, cx: &mut Context<NewMenu>) {
    if let Some(form) = menu.form.as_mut() {
        form.icon = Some(icon);
        cx.notify();
    }
}

pub fn save(menu: &mut NewMenu, cx: &mut Context<NewMenu>) {
    let Some(form) = menu.form.clone() else {
        return;
    };
    if menu.busy {
        return;
    }
    let name = menu.name_input.read(cx).value().trim().to_string();
    let run = match programs::split_command(menu.cmd_input.read(cx).value()) {
        Ok(run) => run,
        Err(e) => {
            menu.error = e;
            cx.notify();
            return;
        }
    };
    if name.is_empty() {
        menu.error = "Type a name.".into();
    } else if run.is_empty() {
        menu.error = "Type the command.".into();
    }
    if name.is_empty() || run.is_empty() {
        cx.notify();
        return;
    }
    menu.busy = true;
    menu.error.clear();
    cx.notify();
    let file = menu.file.clone();
    let taken = taken_keys(&menu.agents);
    let task = cx.background_executor().spawn(async move {
        let old = programs::read(&file).map_err(|e| format!("Cannot read programs.json: {e}"))?;
        let change = match &form.key {
            Some(key) => {
                let values = Fields {
                    name: Some(name),
                    icon: form.icon.clone(),
                    run: Some(run),
                };
                programs::edit_entry(old.as_deref(), key, &values, &form.shown)?
            }
            None => programs::add_entry(old.as_deref(), &name, form.icon.as_deref(), &run, &taken)?,
        };
        programs::write_if_same(&file, &change.text, old.as_deref())?;
        Ok::<_, String>(change)
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |menu, cx| match result {
            Ok(change) => {
                menu.mine = programs::user_keys(Some(&change.text));
                close_form(menu, cx);
                reload(menu, cx);
            }
            Err(e) => {
                menu.busy = false;
                menu.error = e;
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

pub fn ask_remove(menu: &mut NewMenu, key: &str, cx: &mut Context<NewMenu>) {
    menu.confirm = Some(key.to_string());
    cx.notify();
}

pub fn cancel_remove(menu: &mut NewMenu, cx: &mut Context<NewMenu>) {
    menu.confirm = None;
    menu.refocus = true;
    cx.notify();
}

pub fn remove(menu: &mut NewMenu, cx: &mut Context<NewMenu>) {
    let Some(key) = menu.confirm.take() else {
        return;
    };
    menu.refocus = true;
    let file = menu.file.clone();
    let gone = key.clone();
    let task = cx.background_executor().spawn(async move {
        let old = programs::read(&file).map_err(|e| format!("Cannot read programs.json: {e}"))?;
        if let Some(change) = programs::remove_entry(old.as_deref(), &gone)? {
            programs::write_if_same(&file, &change.text, old.as_deref())?;
            return Ok::<_, String>(change.text);
        }
        Ok(old.unwrap_or_default())
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |menu, cx| {
            match result {
                Ok(text) => {
                    menu.mine = programs::user_keys(Some(&text));
                    menu.agents.retain(|a| {
                        a.program.key != key || programs::BUILT_IN.iter().any(|b| b.0 == key)
                    });
                }
                Err(e) => emit(
                    menu,
                    MenuEvent::Notice(format!("Could not remove the agent: {e}")),
                    cx,
                ),
            }
            cx.notify();
            reload(menu, cx);
        })
        .ok();
    })
    .detach();
    cx.notify();
}

pub fn start_custom(menu: &mut NewMenu, id: &str, cx: &mut Context<NewMenu>) {
    if menu.busy {
        return;
    }
    let Some(custom) = menu.customs.iter().find(|c| c.id == id).cloned() else {
        return;
    };
    let Some(link) = menu.link.clone() else {
        emit(
            menu,
            MenuEvent::Notice(format!(
                "Could not start {}: {}",
                custom.title,
                brain::NOT_CONNECTED
            )),
            cx,
        );
        return;
    };
    let mut args = json!({"action": "use", "id": custom.id, "show": true});
    if let Some(space) = &menu.space {
        args["space"] = json!(space);
    }
    let task = cx
        .background_executor()
        .spawn(async move { link.request_for("custom_agents", args, START_WAIT) });
    let on_event = menu.on_event.clone();
    cx.spawn(async move |_, cx| {
        if let Err(error) = task.await {
            let _ = cx.update(|cx| {
                on_event(MenuEvent::Notice(format!("Could not start {}: {error:#}", custom.title)), cx);
            });
        }
    })
    .detach();
    emit(menu, MenuEvent::Close, cx);
}

pub fn start(menu: &NewMenu, key: &str, cx: &mut App) {
    if let Some(agent) = menu
        .agents
        .iter()
        .find(|a| a.program.key == key && a.installed)
    {
        let mut run = agent.program.run.clone();
        let settings = programs::read(&menu.file).ok().flatten();
        nixa_store::agents::configure_start(settings.as_deref(), &mut run);
        let event = MenuEvent::Start {
            name: agent.program.name.clone(),
            run,
        };
        emit(menu, event, cx);
    }
}

pub fn key(
    menu: &mut NewMenu,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<NewMenu>,
) {
    let key = event.keystroke.key.as_str();
    if menu.form.is_some() {
        if key == "escape" {
            close_form(menu, cx);
            window.focus(&menu.focus);
            cx.stop_propagation();
        }
        return;
    }
    if menu.confirm.is_some() {
        match key {
            "escape" => cancel_remove(menu, cx),
            "enter" => remove(menu, cx),
            _ => return,
        }
        cx.stop_propagation();
        return;
    }
    let m = &event.keystroke.modifiers;
    if m.control || m.platform || m.alt {
        return;
    }
    if key == "escape" {
        cx.stop_propagation();
        emit(menu, MenuEvent::Close, cx);
        return;
    }
    let Some(n) = key.parse::<usize>().ok().filter(|n| (1..=9).contains(n)) else {
        return;
    };
    let Some(agent) = installed(menu).get(n - 1).map(|a| a.program.key.clone()) else {
        return;
    };
    cx.stop_propagation();
    start(menu, &agent, cx);
}
