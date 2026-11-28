use gpui::{App, AppContext, Application, Bounds, Focusable, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};
use nixa_editor::{Editor, Options};
use std::time::Duration;

fn main() {
    let path = std::env::args().nth(1);
    let quit_after = std::env::var("NIXA_EDITOR_QUIT_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok());
    let show = std::env::var("NIXA_EDITOR_SHOW").ok().filter(|v| !v.is_empty());
    let readonly = std::env::var("NIXA_EDITOR_READONLY").is_ok_and(|v| v == "1");
    Application::new().with_assets(nixa_ui::Assets).run(move |cx: &mut App| {
        nixa_ui::init(cx);
        nixa_editor::bind_keys(cx);
        let bounds = Bounds::new(point(px(80.0), px(60.0)), size(px(1000.0), px(680.0)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Nixa editor".into()), appears_transparent: false, traffic_light_position: None }),
            ..WindowOptions::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| Editor::new(path.as_deref(), Options { readonly }, window, cx));
            window.focus(&view.focus_handle(cx));
            view
        });
        let Ok(handle) = opened else {
            cx.quit();
            return;
        };
        cx.activate(true);
        if let Some(what) = show {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(800)).await;
                handle.update(cx, |ed, window, cx| nixa_editor::view::show(ed, &what, window, cx)).ok();
            })
            .detach();
        }
        if let Some(ms) = quit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
}
