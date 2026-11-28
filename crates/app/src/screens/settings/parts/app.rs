use super::general::section;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, SharedString, Window, div, px};
use nixa_protocol::paths;
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::text_input::TextInput;
use nixa_ui::{Theme, theme};

pub(crate) const FONT_FAMILY_KEY: &str = "app.terminal_font_family";

pub const PART: &str = "App";

pub const AUTHOR: &str = "Anand Bhandari";
pub const LICENCE: &str = "GPL-3.0, free and open source";
pub const LINKS: [(&str, &str, &str); 4] = [
    ("about-github", "GitHub", "https://github.com/AnandBhandari1/nixa"),
    ("about-issue", "Report an issue", "https://github.com/AnandBhandari1/nixa/issues/new"),
    ("about-linkedin", "LinkedIn", "https://www.linkedin.com/in/anand-bhandari-b98900164/"),
    ("about-website", "Website", "https://anandbhandari1.github.io/"),
];
const THIRD_PARTY_FILE: &str = "THIRD_PARTY_LICENSES.html";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub id: &'static str,
    pub label: &'static str,
    pub value: String,
}

pub fn facts() -> Vec<Fact> {
    vec![
        Fact {
            id: "app-version",
            label: "Version",
            value: env!("CARGO_PKG_VERSION").to_string(),
        },
        Fact {
            id: "app-build",
            label: "Build",
            value: env!("NIXA_BUILD").to_string(),
        },
        Fact {
            id: "app-author",
            label: "Made by",
            value: format!("{AUTHOR}. Copyright (c) 2026."),
        },
        Fact {
            id: "app-licence",
            label: "Licence",
            value: LICENCE.to_string(),
        },
        Fact {
            id: "app-data",
            label: "Data folder",
            value: paths::data_dir().display().to_string(),
        },
        Fact {
            id: "app-brain",
            label: "Brain address",
            value: paths::brain_addr(),
        },
        Fact {
            id: "app-engine",
            label: "Engine socket",
            value: paths::socket_path().display().to_string(),
        },
    ]
}

pub struct AppView {
    pub facts: Vec<Fact>,
    family: Option<Entity<TextInput>>,
    font_error: Option<String>,
}

pub fn build() -> AppView {
    AppView { facts: facts(), family: None, font_error: None }
}

pub fn new(cx: &mut App) -> Entity<AppView> {
    let installer = crate::cli_install::installer(cx);
    cx.new(|cx| {
        if let Some(installer) = installer {
            cx.observe(&installer, |_, _, cx| cx.notify()).detach();
        }
        build()
    })
}

fn fact_row(t: &Theme, fact: &Fact) -> AnyElement {
    let selector = fact.id;
    div()
        .id(fact.id)
        .debug_selector(move || selector.to_string())
        .min_h(px(28.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .w(px(120.))
                .flex_none()
                .text_color(t.muted)
                .child(fact.label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(t.foreground)
                .child(SharedString::from(fact.value.clone())),
        )
        .into_any_element()
}

pub fn third_party_file(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = exe.parent()?;
    let file = macos.parent()?.join("Resources").join(THIRD_PARTY_FILE);
    (macos.file_name()? == "MacOS" && file.is_file()).then_some(file)
}

fn links_row(t: &Theme) -> AnyElement {
    let third_party = std::env::current_exe().ok().and_then(|exe| third_party_file(&exe));
    div()
        .px(px(10.))
        .py(px(6.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .children(LINKS.iter().map(|(id, label, url)| {
            let selector = *id;
            button(t, *id, *label, ButtonKind::Secondary)
                .debug_selector(move || selector.to_string())
                .on_click(move |_, _, cx| cx.open_url(url))
        }))
        .children(third_party.map(|file| {
            button(t, "about-third-party", "Third-party licences", ButtonKind::Secondary)
                .debug_selector(|| "about-third-party".into())
                .on_click(move |_, _, cx| cx.open_with_system(&file))
        }))
        .into_any_element()
}

fn open_setup(cx: &mut App) {
    crate::screens::first_run::host::run(crate::screens::first_run::Action::Open, cx);
    if let Some(ws) = cx.windows().iter().find_map(|w| w.downcast::<crate::workspace::Workspace>()) {
        ws.update(cx, |_, window, _| window.activate_window()).ok();
    }
}

fn setup_row(t: &Theme) -> AnyElement {
    div()
        .min_h(px(36.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(t.muted)
                .child("Open the first run steps again: space, permissions, voice models and chat brain."),
        )
        .child(
            button(t, "app-setup", "Set up Nixa", ButtonKind::Secondary)
                .debug_selector(|| "app-setup".into())
                .on_click(|_, _, cx| open_setup(cx)),
        )
        .into_any_element()
}

fn command_row(t: &Theme, cx: &mut Context<AppView>) -> AnyElement {
    use crate::cli_install::Status;

    let installer = crate::cli_install::installer(cx);
    let message = match installer.as_ref().map(|state| &state.read(cx).status) {
        Some(Status::Installing) => "installing…".into(),
        Some(Status::Installed(path)) => format!("installed at {}", path.display()),
        Some(Status::Unavailable(error)) => error.clone(),
        None => "install status unavailable".into(),
    };
    div()
        .min_h(px(36.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .debug_selector(|| "app-command-status".into())
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .text_color(t.muted)
                .child(format!("nixa command: {message}")),
        )
        .children(installer.map(|installer| {
            button(t, "app-command-install", "Install again", ButtonKind::Secondary)
                .debug_selector(|| "app-command-install".into())
                .on_click(move |_, _, cx| installer.update(cx, crate::cli_install::Installer::install))
        }))
        .into_any_element()
}

pub(crate) fn terminal_family(store: Option<&nixa_store::Connection>) -> String {
    store.and_then(|conn| nixa_store::settings::setting_text(conn, FONT_FAMILY_KEY, nixa_termview::DEFAULT_FONT).ok())
        .unwrap_or_else(|| nixa_termview::DEFAULT_FONT.to_owned())
}

fn save_family(view: &mut AppView, cx: &mut Context<AppView>) {
    let Some(ws) = super::super::workspace(cx) else { return };
    let Some(input) = &view.family else { return };
    let value = input.read(cx).value().trim();
    let family = if value.is_empty() { nixa_termview::DEFAULT_FONT } else { value }.to_owned();
    let saved = ws.update(cx, |ws, _, cx| -> anyhow::Result<()> {
        let store = ws.store.as_ref().ok_or_else(|| anyhow::anyhow!("Settings storage is not available"))?;
        nixa_store::settings::set_setting_text(store, FONT_FAMILY_KEY, &family)?;
        let size = crate::font_zoom::size(Some(store), cx);
        for view in ws.views.values() {
            view.update(cx, |view, cx| view.set_font(&family, size, cx));
        }
        crate::screens::chat::set_font_family(&ws.hub.chat, family.clone(), cx);
        Ok(())
    }).and_then(|result| result);
    view.font_error = saved.err().map(|err| format!("Cannot save the terminal font: {err:#}"));
    if view.font_error.is_none() {
        input.update(cx, |input, cx| input.set_value(family, cx));
    }
    cx.notify();
}

fn terminal_row(view: &mut AppView, t: &Theme, cx: &mut Context<AppView>) -> AnyElement {
    let ws = super::super::workspace(cx);
    let store = ws.and_then(|ws| ws.read(cx).ok()).and_then(|ws| ws.store.as_ref());
    let family = terminal_family(store);
    let size = crate::font_zoom::saved(store);
    let input = view.family.get_or_insert_with(|| cx.new(|cx| {
        let mut input = TextInput::new(cx, nixa_termview::DEFAULT_FONT);
        input.set_value(family, cx);
        input
    })).clone();
    div().flex().flex_col().gap(px(8.))
        .child(div().flex().items_center().flex_wrap().gap(px(8.)).px(px(10.))
            .child("Terminal font family")
            .child(div().w(px(210.)).child(input))
            .child(button(t, "app-font-family", "Save family", ButtonKind::Secondary)
                .debug_selector(|| "app-font-family".into())
                .on_click(cx.listener(|view, _, _, cx| save_family(view, cx)))))
        .children(view.font_error.clone().map(|error| div().px(px(10.)).text_color(t.red).child(error)))
        .child(terminal_size_row(t, ws, size, cx))
        .child(div().px(px(10.)).child(button(t, "app-keys", "Keyboard shortcuts", ButtonKind::Secondary)
            .debug_selector(|| "app-keys".into())
            .on_click(|_, _, cx| crate::keys::help::open(cx))))
        .into_any_element()
}

fn terminal_size_row(
    t: &Theme,
    ws: Option<gpui::WindowHandle<crate::workspace::Workspace>>,
    size: f32,
    cx: &mut Context<AppView>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap(px(8.))
        .px(px(10.))
        .child("Terminal font size")
        .child(button(t, "app-font-smaller", "−", ButtonKind::Secondary)
            .on_click(cx.listener(move |_, _, _, cx| {
                if let Some(ws) = ws {
                    ws.update(cx, |ws, _, cx| crate::font_zoom::step(ws, -1, cx)).ok();
                    cx.notify();
                }
            })))
        .child(format!("{size}"))
        .child(button(t, "app-font-larger", "+", ButtonKind::Secondary)
            .on_click(cx.listener(move |_, _, _, cx| {
                if let Some(ws) = ws {
                    ws.update(cx, |ws, _, cx| crate::font_zoom::step(ws, 1, cx)).ok();
                    cx.notify();
                }
            })))
        .into_any_element()
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let facts: Vec<AnyElement> = self.facts.iter().map(|f| fact_row(&t, f)).collect();
        div()
            .id("settings-app")
            .debug_selector(|| "settings-app".into())
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.8))
            .child(section(
                &t,
                "app-about",
                "About Nixa".into(),
                div().flex().flex_col().children(facts).child(links_row(&t)),
            ))
            .child(section(&t, "app-command", "Command line".into(), command_row(&t, cx)))
            .child(section(&t, "app-terminal", "Terminal and keys".into(), terminal_row(self, &t, cx)))
            .child(section(&t, "app-setup-part", "Setup".into(), setup_row(&t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn the_facts_name_our_own_places() {
        let list = facts();
        let labels: Vec<&str> = list.iter().map(|f| f.label).collect();
        assert_eq!(
            labels,
            ["Version", "Build", "Made by", "Licence", "Data folder", "Brain address", "Engine socket"]
        );
        assert_eq!(list[0].value, env!("CARGO_PKG_VERSION"));
        assert!(!list[1].value.is_empty());
        assert!(list[2].value.contains(AUTHOR));
        assert!(list[3].value.starts_with("GPL-3.0"));
        assert_eq!(list[6].value, paths::socket_path().display().to_string());
    }

    #[test]
    fn third_party_file_is_found_only_inside_an_app() {
        let root = std::env::temp_dir().join(format!("nixa-about-{}", std::process::id()));
        let macos = root.join("Nixa.app/Contents/MacOS");
        let resources = root.join("Nixa.app/Contents/Resources");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        let exe = macos.join("nixa-app");
        assert_eq!(third_party_file(&exe), None);
        std::fs::write(resources.join(THIRD_PARTY_FILE), "x").unwrap();
        assert_eq!(third_party_file(&exe), Some(resources.join(THIRD_PARTY_FILE)));
        assert_eq!(third_party_file(&root.join("target/release/nixa-app")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[gpui::test]
    fn the_app_part_shows_no_machine_row(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let (_view, vcx) = cx.add_window_view(|_, _| build());
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("app-machine").is_none());
        assert!(vcx.debug_bounds("app-data").is_some());
        assert!(vcx.debug_bounds("app-command-status").is_some());
        assert!(vcx.debug_bounds("app-font-family").is_some());
        assert!(vcx.debug_bounds("app-keys").is_some());
        assert!(vcx.debug_bounds("app-author").is_some());
        for (id, _, _) in LINKS {
            assert!(vcx.debug_bounds(id).is_some(), "{id}");
        }
    }
}
