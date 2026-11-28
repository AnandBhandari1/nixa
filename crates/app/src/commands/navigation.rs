use gpui::{App, Global, KeyBinding, actions};

pub const CONTEXT: &str = "NixaCommandPalette";

actions!(nixa_palette, [FirstCommand, LastCommand]);

struct Bound;
impl Global for Bound {}

pub fn bind(cx: &mut App) {
    if !cx.has_global::<Bound>() {
        let context = Some("NixaCommandPalette > NixaTextInput");
        cx.bind_keys([
            KeyBinding::new("home", FirstCommand, context),
            KeyBinding::new("end", LastCommand, context),
        ]);
        cx.set_global(Bound);
    }
}

pub fn selected(key: &str, current: usize, count: usize, page: usize) -> Option<usize> {
    let last = count.saturating_sub(1);
    let current = current.min(last);
    match key {
        "up" => Some(if current == 0 { last } else { current - 1 }),
        "down" => Some(if current == last { 0 } else { current + 1 }),
        "pageup" => Some(current.saturating_sub(page)),
        "pagedown" => Some(current.saturating_add(page).min(last)),
        "home" => Some(0),
        "end" => Some(last),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::prelude::*;
    use gpui::{Context, Entity, Focusable, TestAppContext, Window, div};
    use nixa_ui::text_input::TextInput;

    struct PaletteKeys {
        input: Entity<TextInput>,
        selected: usize,
    }

    impl gpui::Render for PaletteKeys {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("palette-keys")
                .key_context(CONTEXT)
                .on_action(cx.listener(|view, _: &FirstCommand, _, _| view.selected = 0))
                .on_action(cx.listener(|view, _: &LastCommand, _, _| view.selected = 12))
                .child(self.input.clone())
        }
    }

    #[gpui::test]
    fn home_and_end_reach_the_palette_from_its_text_input(cx: &mut TestAppContext) {
        cx.update(|app| {
            nixa_ui::init(app);
            bind(app);
        });
        let (view, vcx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| TextInput::new(cx, ""));
            window.focus(&input.focus_handle(cx));
            PaletteKeys { input, selected: 4 }
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("home");
        view.read_with(vcx, |view, _| assert_eq!(view.selected, 0));
        vcx.simulate_keystrokes("end");
        view.read_with(vcx, |view, _| assert_eq!(view.selected, 12));
    }

    #[test]
    fn arrows_wrap_and_pages_clamp() {
        assert_eq!(selected("up", 0, 32, 14), Some(31));
        assert_eq!(selected("down", 31, 32, 14), Some(0));
        assert_eq!(selected("pageup", 2, 32, 14), Some(0));
        assert_eq!(selected("pagedown", 20, 32, 14), Some(31));
        assert_eq!(selected("home", 20, 32, 14), Some(0));
        assert_eq!(selected("end", 20, 32, 14), Some(31));
        assert_eq!(selected("end", 0, 0, 14), Some(0));
    }
}
