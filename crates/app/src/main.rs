mod actions;
mod cli_install;
mod commands;
mod demo;
mod font_zoom;
mod hub;
mod keys;
mod last_tab;
mod layout;
mod links;
mod named_layouts;
mod panel;
mod pane_moves;
mod panes;
mod programs;
mod quit;
mod rail;
mod screens;
mod sidebar;
mod soak;
mod spaces;
mod statusbar;
mod tabbar;
mod tabs;
mod terms;
mod views;
mod workspace;

use actions::{Hide, HideOthers, OpenAbout, OpenHelp, OpenLayouts, OpenSettings, Quit};
use gpui::{
    App, AppContext, Application, Bounds, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, point, px, size,
};
use std::time::Duration;
use workspace::Workspace;

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let program = std::path::Path::new(&argv[0]).file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let (word, rest) = match (program.as_str(), argv.get(1).map(String::as_str)) {
        ("nixa", _) => ("cli", argv[1..].to_vec()),
        ("nixa-mcp", _) => ("mcp", argv[1..].to_vec()),
        ("nixa-json", _) => ("json", argv[1..].to_vec()),
        ("nixa-engine", _) => ("engine", argv[1..].to_vec()),
        ("nixa-brain", _) => ("brain", argv[1..].to_vec()),
        (_, Some(word @ ("cli" | "mcp" | "json" | "engine" | "brain"))) => (word, argv[2..].to_vec()),
        _ => ("", Vec::new()),
    };
    match word {
        "cli" => nixa_cli::run_main(rest),
        "mcp" => {
            let mut args = vec![argv[0].clone()];
            args.extend(rest);
            nixa_mcp::run_main(args);
            return;
        }
        "json" => std::process::exit(if nixa_jsonedit::run_main(rest) == std::process::ExitCode::SUCCESS { 0 } else { 1 }),
        "engine" => nixa_engine::run_main(rest),
        "brain" => {
            match nixa_brain::start() {
                Ok(handle) => {
                    while !nixa_brain::finished(&handle) {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }
                Err(error) => {
                    eprintln!("nixa-brain: {error:#}");
                    std::process::exit(1);
                }
            }
            return;
        }
        _ => {}
    }
    let quit_after = std::env::var("NIXA_APP_QUIT_AFTER_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    Application::new()
        .with_assets(nixa_ui::Assets)
        .run(move |cx: &mut App| {
            nixa_ui::init(cx);
            quit::watch_quit_event();
            cli_install::init(cx);
            actions::bind(cx);
            screens::git::join::init(cx);
            screens::editor::init(cx);
            cx.on_action(|_: &Quit, cx| {
                quit::mark(quit::Why::Menu);
                cx.quit();
            });
            cx.on_app_quit(|_| {
                eprintln!("{}", quit::line(quit::why()));
                quit::log_sender();
                links::procs::shutdown();
                quit::come_back_after_outside_quit();
                async {}
            })
            .detach();
            cx.on_action(|_: &OpenSettings, cx| screens::settings::open("General", cx));
            cx.set_menus(vec![Menu {
                name: "Nixa".into(),
                items: vec![
                    MenuItem::action("About Nixa", OpenAbout),
                    MenuItem::separator(),
                    MenuItem::action("Settings", OpenSettings),
                    MenuItem::action("Keys", OpenHelp),
                    MenuItem::action("Layouts", OpenLayouts),
                    MenuItem::separator(),
                    MenuItem::action("Hide Nixa", Hide),
                    MenuItem::action("Hide Others", HideOthers),
                    MenuItem::separator(),
                    MenuItem::action("Quit Nixa", Quit),
                ],
            }]);
            let soak = soak::now();
            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(soak::title(soak).into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(16.))),
                }),
                window_min_size: Some(size(px(720.), px(420.))),
                focus: !soak,
                ..WindowOptions::default()
            };
            if let Err(e) = cx.open_window(options, |window, cx| {
                cx.new(|cx| Workspace::new(window, cx))
            }) {
                eprintln!("nixa-app: cannot open the window: {e}");
                quit::mark(quit::Why::NoWindow);
                cx.quit();
                return;
            }
            cx.on_window_closed(|cx| {
                if !cx
                    .windows()
                    .iter()
                    .any(|w| w.downcast::<Workspace>().is_some())
                {
                    quit::mark(quit::Why::WindowClosed);
                    cx.quit();
                }
            })
            .detach();
            if !soak {
                cx.activate(true);
            }
            if let Some(ms) = quit_after {
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(ms))
                        .await;
                    cx.update(|cx| {
                        quit::mark(quit::Why::Timer);
                        cx.quit();
                    })
                    .ok();
                })
                .detach();
            }
        });
}
