use super::general::{note, section};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Hsla, SharedString, Window, div, px};
use nixa_ui::popover::tooltip;
use nixa_ui::theme::{CHOICE_FILE, Themes, USER_DIR};
use nixa_ui::{Icon, Theme, icon, theme};

pub const PART: &str = "Look";

#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub name: SharedString,
    pub label: SharedString,
    pub background: Hsla,
    pub chrome: Hsla,
    pub accent: Hsla,
    pub foreground: Hsla,
    pub current: bool,
}

pub fn is_light(color: Hsla) -> bool {
    color.l > 0.5
}

pub fn hint(s: &Swatch) -> &'static str {
    if s.current {
        "current"
    } else if is_light(s.background) {
        "light"
    } else {
        "dark"
    }
}

pub fn swatches_of(list: &[Theme], current: &str) -> Vec<Swatch> {
    list.iter()
        .map(|t| Swatch {
            name: t.name.clone(),
            label: t.label.clone(),
            background: t.background,
            chrome: t.chrome,
            accent: t.accent,
            foreground: t.foreground,
            current: t.name.as_ref() == current,
        })
        .collect()
}

pub fn swatches(cx: &App) -> Vec<Swatch> {
    let current = theme(cx).name.to_string();
    cx.try_global::<Themes>()
        .map(|g| swatches_of(&g.list, &current))
        .unwrap_or_default()
}

pub fn folder(cx: &App) -> Option<String> {
    cx.try_global::<Themes>()
        .map(|g| g.dir.join(USER_DIR).display().to_string())
}

pub struct LookView {
    pub error: Option<String>,
    pub done: Option<String>,
}

pub fn build() -> LookView {
    LookView {
        error: None,
        done: None,
    }
}

pub fn new(cx: &mut App) -> Entity<LookView> {
    cx.new(|_| build())
}

pub fn choose(view: &mut LookView, name: &str, cx: &mut Context<LookView>) {
    match nixa_ui::theme::choose(cx, name) {
        Ok(true) => {
            view.error = None;
            view.done = Some(format!("Using the {name} theme."));
        }
        Ok(false) => view.error = Some(format!("There is no theme named {name} any more.")),
        Err(e) => {
            view.error = Some(format!(
                "The theme is used now, but {CHOICE_FILE} was not saved: {e}"
            ))
        }
    }
    cx.notify();
}

pub fn reload(view: &mut LookView, cx: &mut Context<LookView>) {
    nixa_ui::theme::reload(cx);
    view.error = None;
    view.done = Some(format!("{} themes loaded.", swatches(cx).len()));
    cx.notify();
}

fn swatch_row(t: &Theme, index: usize, s: &Swatch, cx: &mut Context<LookView>) -> AnyElement {
    let hover = t.hover;
    let name = s.name.to_string();
    let selector = format!("look-theme-{name}");
    div()
        .id(("look-theme", index))
        .debug_selector(move || selector.clone())
        .h(px(34.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .rounded(px(7.))
        .cursor_pointer()
        .when(s.current, |el| el.bg(t.selected))
        .hover(move |st| st.bg(hover))
        .tooltip(tooltip(format!("Use the {} theme", s.label)))
        .on_click(cx.listener(move |view, _, _, cx| choose(view, &name, cx)))
        .child(
            div()
                .w(px(38.))
                .h(px(18.))
                .flex_none()
                .flex()
                .overflow_hidden()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border)
                .child(div().w(px(14.)).h_full().bg(s.chrome))
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .bg(s.background)
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(2.))
                        .child(div().size(px(5.)).rounded_full().bg(s.accent))
                        .child(div().w(px(6.)).h(px(2.)).bg(s.foreground)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.foreground)
                .child(s.label.clone()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(if s.current { t.green } else { t.dim })
                .child(hint(s)),
        )
        .when(s.current, |el| {
            el.child(icon(Icon::Check).size(px(13.)).text_color(t.green))
        })
        .into_any_element()
}

impl Render for LookView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let list = swatches(cx);
        let rows: Vec<AnyElement> = list
            .iter()
            .enumerate()
            .map(|(i, s)| swatch_row(&t, i, s, cx))
            .collect();
        let hover = t.hover;
        let where_note = match folder(cx) {
            Some(dir) => format!(
                "Own theme files go in {dir} as <name>.json. Reload after you add or change one."
            ),
            None => "Own theme files go in the themes folder of the data folder.".to_string(),
        };
        div()
            .id("settings-look")
            .debug_selector(|| "settings-look".into())
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.8))
            .child(section(
                &t,
                "look-themes",
                format!("Theme ({})", list.len()),
                div().flex().flex_col().gap(px(2.)).children(rows),
            ))
            .child(section(
                &t,
                "look-files",
                "Own themes".into(),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div().flex().child(
                            div()
                                .id("look-reload")
                                .debug_selector(|| "look-reload".into())
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .px(px(9.))
                                .py(px(6.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(t.border)
                                .text_size(px(12.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(|view, _, _, cx| reload(view, cx)))
                                .child(icon(Icon::Refresh).size(px(13.)).text_color(t.muted))
                                .child("Reload theme files"),
                        ),
                    )
                    .child(note(&t, where_note)),
            ))
            .when_some(self.done.clone(), |el, done| el.child(note(&t, done)))
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("look-error")
                        .debug_selector(|| "look-error".into())
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

    fn click(vcx: &mut VisualTestContext, selector: &'static str) {
        vcx.run_until_parked();
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
    }

    #[test]
    fn swatches_mark_the_current_theme() {
        let list = nixa_ui::theme::builtin();
        let found = swatches_of(&list, "ink");
        assert_eq!(found.len(), list.len());
        assert_eq!(found.iter().filter(|s| s.current).count(), 1);
        let ink = found.iter().find(|s| s.current).unwrap();
        assert_eq!((ink.name.as_ref(), hint(ink)), ("ink", "current"));
        let other = found.iter().find(|s| !s.current).unwrap();
        assert_eq!(
            hint(other),
            if is_light(other.background) {
                "light"
            } else {
                "dark"
            }
        );
        assert!(is_light(gpui::hsla(0., 0., 0.9, 1.)));
        assert!(!is_light(gpui::hsla(0., 0., 0.1, 1.)));
    }

    #[gpui::test]
    fn a_click_on_a_swatch_uses_and_saves_the_theme(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("msl-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.clone();
        cx.update(move |cx| {
            nixa_ui::init(cx);
            nixa_ui::theme::install_from(cx, d);
        });
        let (view, vcx) = cx.add_window_view(|_, _| build());
        click(vcx, "look-theme-ink");
        assert_eq!(vcx.update(|_, cx| theme(cx).name.to_string()), "ink");
        assert_eq!(nixa_ui::theme::saved_choice(&dir).as_deref(), Some("ink"));
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).done.clone()).as_deref(),
            Some("Using the ink theme.")
        );
        let marked = vcx.update(|_, cx| {
            swatches(cx)
                .into_iter()
                .filter(|s| s.current)
                .map(|s| s.name.to_string())
                .collect::<Vec<_>>()
        });
        assert_eq!(marked, ["ink"]);
        click(vcx, "look-reload");
        let done = vcx.update(|_, cx| view.read(cx).done.clone()).unwrap();
        assert!(done.ends_with("themes loaded."), "{done}");
        assert_eq!(vcx.update(|_, cx| theme(cx).name.to_string()), "ink");
        assert!(vcx.update(|_, cx| folder(cx)).unwrap().ends_with("themes"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
