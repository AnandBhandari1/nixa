use anyhow::{Context as _, Result, bail};
use gpui::{
    App, AppContext, Application, AsyncApp, Bounds, Entity, Focusable, KeyBinding, Keystroke,
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, ScrollDelta,
    ScrollWheelEvent, TitlebarOptions, TouchPhase, WindowBounds, WindowHandle, WindowOptions,
    actions, point, px, size,
};
use nixa_protocol::{CreateSpec, Reply, Request, TermId, connect, paths};
use nixa_termview::TermView;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

actions!(term_example, [Quit]);

const DEMO: &str = r#"clear; printf '\033[1mbold\033[0m \033[3mitalic\033[0m \033[4munderline\033[0m \033[7minverse\033[0m \033[9mstrike\033[0m \033[2mdim\033[0m \033[1;3;4;31mall red\033[0m\n\n'; for i in 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do printf "\033[48;5;${i}m  "; done; printf '\033[0m\n'; i=16; while [ $i -lt 232 ]; do printf "\033[48;5;${i}m "; i=$((i+1)); if [ $(( (i-16) % 72 )) -eq 0 ]; then printf '\033[0m\n'; fi; done; printf '\033[0m\n'; i=0; while [ $i -lt 72 ]; do r=$((255-i*3)); g=$((i*3)); printf "\033[48;2;${r};${g};180m "; i=$((i+1)); done; printf '\033[0m\n\n'; printf 'wide: \346\227\245\346\234\254\350\252\236  box: \342\224\214\342\224\200\342\224\200\342\224\220\n'; echo "terminal $NIXA_TERM_ID on $TERM_PROGRAM"; ls -la"#;

fn engine_binary() -> Option<PathBuf> {
    let root = paths::dev_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    ["gpui", "engine"]
        .iter()
        .map(|t| {
            root.join("target")
                .join(t)
                .join("debug")
                .join("nixa-engine")
        })
        .find(|p| p.exists())
}

fn ping(socket: &Path) -> bool {
    connect(socket)
        .and_then(|mut c| {
            c.set_timeout(Some(Duration::from_secs(2)))?;
            c.call(&Request::Ping)
        })
        .is_ok_and(|r| matches!(r, Reply::Pong { .. }))
}

fn ensure_engine(socket: &Path) -> Result<bool> {
    if ping(socket) {
        return Ok(false);
    }
    let bin = engine_binary()
        .context("nixa-engine is not running and no nixa-engine binary was found in target")?;
    Command::new(&bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("cannot start {}", bin.display()))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if ping(socket) {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("nixa-engine did not answer on {}", socket.display())
}

fn call(socket: &Path, req: &Request) -> Result<Reply> {
    let mut c = connect(socket)?;
    c.set_timeout(Some(Duration::from_secs(5)))?;
    c.call(req)
}

fn open_term(socket: &Path) -> Result<(TermId, bool)> {
    if let Some(id) = std::env::var("NIXA_TERMVIEW_ID")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return Ok((id, false));
    }
    let spec = CreateSpec {
        name: Some("termview".to_string()),
        cwd: Some(paths::dev_root().unwrap_or_else(|| std::env::current_dir().unwrap_or_default()).display().to_string()),
        rows: Some(36),
        cols: Some(118),
        ..CreateSpec::default()
    };
    match call(socket, &Request::Create { spec })? {
        Reply::Term { term } => Ok((term.id, true)),
        other => bail!("unexpected reply to create: {other:?}"),
    }
}

fn clean_up(socket: &Path, id: &TermId, created: bool, started: bool) {
    if created {
        call(socket, &Request::Remove { id: id.clone() }).ok();
    }
    if started {
        call(socket, &Request::Shutdown).ok();
    }
}

async fn pause(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn mouse(cx: &mut AsyncApp, handle: WindowHandle<TermView>, input: PlatformInput) {
    handle
        .update(cx, |view, window, cx| match &input {
            PlatformInput::MouseDown(ev) => view.on_mouse_down(ev, window, cx),
            PlatformInput::MouseUp(ev) => view.on_mouse_up(ev, window, cx),
            PlatformInput::ScrollWheel(ev) => view.on_scroll(ev, window, cx),
            _ => {}
        })
        .ok();
}

fn type_keys(cx: &mut AsyncApp, handle: WindowHandle<TermView>, keys: &[&str]) {
    for k in keys {
        if let Ok(ks) = Keystroke::parse(k) {
            cx.update_window(handle.into(), |_, window, cx| {
                window.dispatch_keystroke(ks, cx);
            })
            .ok();
        }
    }
}

async fn self_test(cx: &mut AsyncApp, handle: WindowHandle<TermView>, view: Entity<TermView>) {
    pause(cx, 2500).await;
    let mut keys: Vec<String> = "echo typed ok 42"
        .chars()
        .map(|c| {
            if c == ' ' {
                "space".to_string()
            } else {
                c.to_string()
            }
        })
        .collect();
    keys.push("enter".to_string());
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    type_keys(cx, handle, &keys);
    pause(cx, 1500).await;
    let rows = view
        .read_with(cx, |v, _| {
            v.snapshot()
                .map(|s| (0..s.rows).map(|r| s.row_text(r)).collect::<Vec<_>>())
        })
        .ok()
        .flatten()
        .unwrap_or_default();
    let found = rows.iter().position(|r| r.trim() == "typed ok 42");
    println!("selftest: typed line found at row {found:?}");
    if let Some(row) = found {
        let at = view
            .read_with(cx, |v, _| v.cell_position(row as u16, 7))
            .ok()
            .flatten();
        if let Some(position) = at {
            let mods = Modifiers::default();
            mouse(
                cx,
                handle,
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: mods,
                    click_count: 2,
                    first_mouse: false,
                }),
            );
            mouse(
                cx,
                handle,
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: mods,
                    click_count: 2,
                }),
            );
            pause(cx, 200).await;
            let sel = view.read_with(cx, |v, _| v.selected_text()).ok().flatten();
            println!("selftest: double click selected {sel:?}");
            type_keys(cx, handle, &["cmd-c"]);
            let clip = cx
                .update(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
                .ok()
                .flatten();
            println!("selftest: clipboard {clip:?}");
            mouse(
                cx,
                handle,
                PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Lines(point(0.0, 3.0)),
                    modifiers: mods,
                    touch_phase: TouchPhase::Moved,
                }),
            );
            pause(cx, 700).await;
            let scrolled = view
                .read_with(cx, |v, _| (v.scroll(), v.snapshot().map(|s| s.scroll)))
                .ok();
            println!("selftest: after wheel up (view scroll, snapshot scroll) {scrolled:?}");
            type_keys(cx, handle, &["shift-end"]);
            pause(cx, 700).await;
            let back = view
                .read_with(cx, |v, _| (v.scroll(), v.snapshot().map(|s| s.scroll)))
                .ok();
            println!("selftest: after shift-end {back:?}");
        }
    }
}

fn main() -> Result<()> {
    let socket = paths::socket_path();
    let started = ensure_engine(&socket)?;
    let (id, created) = open_term(&socket)?;
    println!(
        "termview: engine {} started_here={started} terminal {id}",
        socket.display()
    );
    if std::env::var("NIXA_TERMVIEW_DEMO").is_ok_and(|v| v == "1") {
        let (socket, id) = (socket.clone(), id.clone());
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1200));
            let req = Request::Send {
                id,
                text: DEMO.to_string(),
                submit: true,
                timeout_ms: None,
                force: true,
            };
            if let Err(e) = call(&socket, &req) {
                eprintln!("termview: demo send failed: {e}");
            }
        });
    }
    let quit_after = std::env::var("NIXA_TERMVIEW_QUIT_AFTER_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let (q_socket, q_id) = (socket.clone(), id.clone());
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        let (a_socket, a_id) = (q_socket.clone(), q_id.clone());
        cx.on_action(move |_: &Quit, cx| {
            clean_up(&a_socket, &a_id, created, started);
            cx.quit();
        });
        let bounds = Bounds::new(point(px(80.0), px(80.0)), size(px(980.0), px(640.0)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(format!("nixa termview {q_id}").into()),
                ..TitlebarOptions::default()
            }),
            ..WindowOptions::default()
        };
        let view_socket = q_socket.clone();
        let view_id = q_id.clone();
        let opened = cx.open_window(options, move |window, cx| {
            let view = cx.new(|cx| TermView::new(view_socket, view_id, window, cx));
            window.focus(&view.focus_handle(cx));
            view
        });
        let Ok(handle) = opened else {
            clean_up(&q_socket, &q_id, created, started);
            cx.quit();
            return;
        };
        cx.activate(true);
        if std::env::var("NIXA_TERMVIEW_SELFTEST").is_ok_and(|v| v == "1")
            && let Ok(view) = handle.entity(cx)
        {
            cx.spawn(async move |cx| self_test(cx, handle, view).await)
                .detach();
        }
        if let Some(ms) = quit_after {
            let (t_socket, t_id) = (q_socket.clone(), q_id.clone());
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(ms))
                    .await;
                clean_up(&t_socket, &t_id, created, started);
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
    Ok(())
}
