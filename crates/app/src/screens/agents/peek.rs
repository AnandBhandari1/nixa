use super::{AgentsEvent, AgentsView};
use crate::views;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Focusable, SharedString, Window, anchored, deferred, div, point, px,
};
use nixa_protocol::paths;
use nixa_termview::TermView;
use nixa_ui::Theme;
use nixa_ui::modal::dialog;

pub const PEEK_WIDTH: f32 = 720.;
pub const PEEK_HEIGHT: f32 = 420.;

pub fn close(v: &mut AgentsView, cx: &mut Context<AgentsView>) {
    v.peek = None;
    v.peek_view = None;
    cx.notify();
}

pub fn render(
    v: &mut AgentsView,
    t: &Theme,
    window: &mut Window,
    cx: &mut Context<AgentsView>,
) -> Option<AnyElement> {
    let Some(id) = v.peek.clone() else {
        v.peek_view = None;
        return None;
    };
    let kid = v.input.kids.iter().find(|k| k.term == id).cloned()?;
    if !cfg!(test) && v.peek_view.as_ref().is_none_or(|(on, _)| *on != id) {
        let palette = views::palette(t);
        let term = id.clone();
        let view = cx.new(|cx| TermView::new(paths::socket_path(), term, window, cx));
        view.update(cx, |view, cx| view.set_palette(palette, cx));
        window.focus(&view.focus_handle(cx));
        v.peek_view = Some((id.clone(), view));
    }
    let parent = v
        .input
        .agents
        .iter()
        .find(|a| a.term == kid.parent)
        .map(|a| a.label.clone())
        .unwrap_or_else(|| kid.parent.to_string());
    let body = div()
        .id("agents-peek-term")
        .debug_selector(|| "agents-peek-term".into())
        .w_full()
        .h(px(PEEK_HEIGHT))
        .rounded(px(6.))
        .overflow_hidden()
        .bg(t.background)
        .map(|el| match &v.peek_view {
            Some((_, view)) => el.child(view.clone()),
            None => el.child(div().p(px(12.)).text_color(t.dim).child("Terminal view")),
        });
    let hint = div()
        .text_size(px(11.5))
        .text_color(t.dim)
        .child(SharedString::from(format!(
            "Sub-agent of {parent}. What you type here goes to {id}."
        )));
    let this = cx.entity();
    let shut = this.clone();
    let done = this.clone();
    let pop = this.clone();
    let hover = t.hover;
    let kid_term = kid.term.clone();
    let open = div()
        .id("agents-peek-open")
        .debug_selector(|| "agents-peek-open".into())
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .bg(t.selected)
        .text_size(px(12.))
        .text_color(t.bright)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(move |_, _, cx| {
            let term = kid_term.clone();
            pop.update(cx, |v, cx| {
                close(v, cx);
                cx.emit(AgentsEvent::PopOut { term });
            })
        })
        .child("Open in a pane");
    let footer = div()
        .id("agents-peek-close")
        .debug_selector(|| "agents-peek-close".into())
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(t.border)
        .bg(t.raised)
        .text_size(px(12.))
        .text_color(t.bright)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(move |_, _, cx| done.update(cx, close))
        .child("Close");
    let footer = div().flex().gap(px(8.)).child(footer).child(open).into_any_element();
    let dialog = dialog(
        t,
        "agents-peek-dialog",
        SharedString::from(format!("Peek: {} ({id})", kid.name)),
        PEEK_WIDTH,
        div().flex().flex_col().gap(px(8.)).child(body).child(hint),
        Some(footer),
        move |_, cx| shut.update(cx, close),
    );
    let size = window.viewport_size();
    let scrim = t.scrim;
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("agents-peek")
                    .debug_selector(|| "agents-peek".into())
                    .w(size.width)
                    .h(size.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(scrim)
                    .occlude()
                    .on_click(move |_, _, cx| this.update(cx, close))
                    .child(
                        div()
                            .id("agents-peek-body")
                            .occlude()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(dialog),
                    ),
            ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}
