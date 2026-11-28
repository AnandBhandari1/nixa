use crate::workspace::{PANEL_SPLIT, PanelKind, Workspace};
use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, div, px};
use nixa_ui::icon_button::{IconSize, icon_button};
use nixa_ui::metrics::PANE_RADIUS;
use nixa_ui::split::split_handle;
use nixa_ui::{Icon, theme};

pub fn title(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Files => "Files",
        PanelKind::Git => "Git",
        PanelKind::Nixa => "Nixa",
    }
}

pub fn word(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Files => "files",
        PanelKind::Git => "git",
        PanelKind::Nixa => "nixa",
    }
}

pub fn from_word(word: &str) -> Option<PanelKind> {
    [
        PanelKind::Files,
        PanelKind::Git,
        PanelKind::Nixa,
    ]
    .into_iter()
    .find(|k| self::word(*k) == word)
}

pub fn render(
    ws: &Workspace,
    kind: PanelKind,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let t = theme(cx).clone();
    let body = match kind {
        PanelKind::Files => crate::screens::files::host::render(ws),
        PanelKind::Git => crate::screens::git::changes::host::render(ws),
        PanelKind::Nixa => nixa(ws, cx),
    };
    div()
        .id("panel")
        .debug_selector(|| "panel".into())
        .relative()
        .w(px(ws.panel_w))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .rounded(px(PANE_RADIUS))
        .border_1()
        .border_color(t.border)
        .bg(t.background)
        .when(!matches!(kind, PanelKind::Nixa | PanelKind::Files), |el| {
            el.child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.5))
                            .text_color(t.bright)
                            .child(title(kind)),
                    )
                    .child(
                        icon_button(&t, "panel-close", Icon::X, IconSize::Small, false)
                            .on_click(cx.listener(|ws, _, _, cx| ws.set_panel(None, cx))),
                    ),
            )
        })
        .child(div().flex_1().min_h_0().overflow_hidden().child(body))
        .child(
            split_handle(&t, PANEL_SPLIT, gpui::Axis::Horizontal)
                .absolute()
                .top_0()
                .right(px(-7.)),
        )
}

fn nixa(ws: &Workspace, _cx: &mut gpui::App) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(crate::screens::chat::render(&ws.hub.chat))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_words_go_both_ways() {
        for kind in [
            PanelKind::Files,
            PanelKind::Git,
            PanelKind::Nixa,
        ] {
            assert_eq!(from_word(word(kind)), Some(kind));
        }
        assert_eq!(from_word("nope"), None);
        assert_eq!(from_word("blueprint"), None);
    }
}
