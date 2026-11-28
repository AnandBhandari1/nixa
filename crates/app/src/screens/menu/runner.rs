use anyhow::{Result, bail};
use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowControlArea, WindowHandle, WindowOptions, div, point, px, size,
};
use nixa_protocol::{
    CreateSpec, ReadSource, Reply, Request, TermId, TermInfo, WaitFor, WaitOutcome, connect,
};
use nixa_termview::TermView;
use nixa_ui::metrics::TRAFFIC_LIGHTS_SPACE;
use nixa_ui::popover::tooltip;
use nixa_ui::theme;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const LABEL_POPUP: &str = "popup";
pub const WIDTH: f32 = 760.;
pub const HEIGHT: f32 = 460.;
pub const BAR: f32 = 30.;
pub const WAIT_MS: u64 = 20_000;
const CALL: Duration = Duration::from_secs(10);
const READ_LINES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub title: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    pub title: String,
    pub body: String,
    pub failed: bool,
}

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn script(title: &str, body: &str, cwd: Option<String>) -> Run {
    Run {
        title: title.to_string(),
        argv: vec!["sh".into(), "-c".into(), body.to_string()],
        cwd,
    }
}

pub fn spec(run: &Run) -> CreateSpec {
    let line: Vec<String> = run.argv.iter().map(|a| quote(a)).collect();
    let mut labels = BTreeMap::new();
    labels.insert(LABEL_POPUP.to_string(), run.title.clone());
    CreateSpec {
        cmd: vec![
            "sh".into(),
            "-c".into(),
            format!(
                "TERM=xterm-256color COLORTERM=truecolor exec {}",
                line.join(" ")
            ),
        ],
        cwd: run.cwd.clone(),
        name: Some(run.title.clone()),
        labels,
        ..CreateSpec::default()
    }
}

pub fn last_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

pub fn finish(title: &str, text: &str, code: Option<i32>) -> Finish {
    let failed = code != Some(0);
    let line = last_line(text);
    let body = if !line.is_empty() {
        line
    } else if failed {
        format!(
            "Exit {}",
            code.map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into())
        )
    } else {
        "Finished".to_string()
    };
    Finish {
        title: title.to_string(),
        body,
        failed,
    }
}

pub fn notice(f: &Finish) -> String {
    let word = if f.failed { "failed" } else { "done" };
    format!("{} {word}: {}", f.title, f.body)
}

pub fn spoken(f: &Finish) -> Option<String> {
    f.failed.then(|| format!("{} failed: {}", f.title, f.body))
}

fn call(socket: &Path, req: &Request, wait: Duration) -> Result<Reply> {
    let mut client = connect(socket)?;
    client.set_timeout(Some(wait))?;
    client.call(req)
}

pub fn create(socket: &Path, spec: CreateSpec) -> Result<TermInfo> {
    match call(socket, &Request::Create { spec }, CALL)? {
        Reply::Term { term } => Ok(term),
        other => bail!("unexpected reply to create: {other:?}"),
    }
}

pub fn exit_code(socket: &Path, id: &TermId) -> Result<Option<i32>> {
    match call(socket, &Request::Get { id: id.clone() }, CALL)? {
        Reply::Term { term } => Ok(term.exit_code),
        other => bail!("unexpected reply to get: {other:?}"),
    }
}

pub fn wait_end(socket: &Path, id: &TermId) -> Result<Option<i32>> {
    loop {
        let req = Request::Wait {
            id: id.clone(),
            until: WaitFor::Exit,
            timeout_ms: Some(WAIT_MS),
        };
        match call(socket, &req, Duration::from_millis(WAIT_MS) + CALL)? {
            Reply::Waited { result } if result.outcome == WaitOutcome::Timeout => continue,
            Reply::Waited { .. } => return exit_code(socket, id),
            other => bail!("unexpected reply to wait: {other:?}"),
        }
    }
}

pub fn read_text(socket: &Path, id: &TermId) -> String {
    let req = Request::Read {
        id: id.clone(),
        source: ReadSource::History,
        lines: Some(READ_LINES),
    };
    match call(socket, &req, CALL) {
        Ok(Reply::Text { text, .. }) => text,
        _ => String::new(),
    }
}

pub fn collect(socket: &Path, id: &TermId, title: &str) -> Result<Finish> {
    let code = wait_end(socket, id)?;
    Ok(finish(title, &read_text(socket, id), code))
}

pub fn remove(socket: &Path, id: &TermId) {
    call(socket, &Request::Remove { id: id.clone() }, CALL).ok();
}

pub struct RunnerView {
    pub title: String,
    pub view: Entity<TermView>,
    pub closed: Arc<AtomicBool>,
    focus: FocusHandle,
}

impl Focusable for RunnerView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn build(
    title: String,
    term: TermId,
    socket: PathBuf,
    closed: Arc<AtomicBool>,
    window: &mut Window,
    cx: &mut Context<RunnerView>,
) -> RunnerView {
    let palette = crate::views::palette(theme(cx));
    let view = cx.new(|cx| TermView::new(socket.clone(), term.clone(), window, cx));
    view.update(cx, |v, cx| v.set_palette(palette, cx));
    window.focus(&view.focus_handle(cx));
    let gone = term.clone();
    let flag = closed.clone();
    cx.on_release(move |_, _| {
        flag.store(true, Ordering::SeqCst);
        std::thread::spawn(move || remove(&socket, &gone));
    })
    .detach();
    RunnerView {
        title,
        view,
        closed,
        focus: cx.focus_handle(),
    }
}

pub fn open_window(
    title: String,
    term: TermId,
    socket: PathBuf,
    closed: Arc<AtomicBool>,
    cx: &mut App,
) -> Result<WindowHandle<RunnerView>> {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from(title.clone())),
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(9.))),
        }),
        window_min_size: Some(size(px(360.), px(200.))),
        ..WindowOptions::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| build(title, term, socket, closed, window, cx))
    })
}

pub fn close(view: &mut RunnerView, window: &mut Window) {
    view.closed.store(true, Ordering::SeqCst);
    window.remove_window();
}

impl Render for RunnerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let hover = t.hover;
        let bright = t.bright;
        let bar = div()
            .id("runner-bar")
            .h(px(BAR))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(TRAFFIC_LIGHTS_SPACE))
            .pr(px(10.))
            .bg(t.chrome)
            .border_b_1()
            .border_color(t.border)
            .text_size(px(12.))
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.muted)
                    .child(SharedString::from(self.title.clone())),
            )
            .child(
                div()
                    .id("runner-close")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(t.muted)
                    .hover(move |s| s.bg(hover).text_color(bright))
                    .tooltip(tooltip("Close"))
                    .on_click(cx.listener(|view, _, window, _| close(view, window)))
                    .child("×"),
            );
        div()
            .id("runner")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .child(bar)
            .child(div().flex_1().min_h_0().child(self.view.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};
    use std::time::Instant;

    #[test]
    fn quoting_keeps_every_word_whole() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        let run = script("tasks", "echo 'hi there'; exit 3", Some("/tmp".into()));
        let spec = spec(&run);
        assert_eq!(spec.cmd[..2], ["sh".to_string(), "-c".to_string()]);
        assert_eq!(
            spec.cmd[2],
            "TERM=xterm-256color COLORTERM=truecolor exec 'sh' '-c' 'echo '\\''hi there'\\''; exit 3'"
        );
        assert_eq!(
            spec.labels.get(LABEL_POPUP).map(String::as_str),
            Some("tasks")
        );
        assert_eq!(spec.name.as_deref(), Some("tasks"));
        assert_eq!(spec.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn finish_takes_the_last_line_with_text() {
        let ok = finish("tasks", "one\n two \n\n  \n", Some(0));
        assert_eq!(
            ok,
            Finish {
                title: "tasks".into(),
                body: "two".into(),
                failed: false
            }
        );
        assert_eq!(notice(&ok), "tasks done: two");
        assert_eq!(spoken(&ok), None);
        let bad = finish("commands", "\r\n", Some(2));
        assert_eq!(bad.body, "Exit 2");
        assert!(bad.failed);
        assert_eq!(spoken(&bad).as_deref(), Some("commands failed: Exit 2"));
        assert_eq!(finish("x", "", None).body, "Exit unknown");
        assert_eq!(finish("x", "", Some(0)).body, "Finished");
        assert_eq!(notice(&finish("x", "boom", Some(1))), "x failed: boom");
    }

    fn engine_bin() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let debug = exe.parent().and_then(Path::parent).unwrap().to_path_buf();
        let bin = debug.join("nixa-engine");
        assert!(
            bin.exists(),
            "build the engine first: cargo build -p nixa-engine ({})",
            bin.display()
        );
        bin
    }

    struct Engine {
        child: Child,
        dir: PathBuf,
        socket: PathBuf,
    }

    impl Engine {
        fn start(name: &str) -> Engine {
            let dir = PathBuf::from(format!("/tmp/mmr-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let socket = dir.join("e.sock");
            let child = Command::new(engine_bin())
                .arg("--socket")
                .arg(&socket)
                .env("NIXA_DATA_DIR", &dir)
                .spawn()
                .unwrap();
            let end = Instant::now() + Duration::from_secs(10);
            while call(&socket, &Request::Ping, CALL).is_err() {
                assert!(Instant::now() < end, "engine did not start");
                std::thread::sleep(Duration::from_millis(50));
            }
            Engine { child, dir, socket }
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            call(&self.socket, &Request::Shutdown, CALL).ok();
            std::thread::sleep(Duration::from_millis(100));
            self.child.kill().ok();
            self.child.wait().ok();
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    #[test]
    fn a_real_run_reports_its_end_and_last_line() {
        let engine = Engine::start("run");
        let ok = script("tasks", "echo first; echo all good", Some("/tmp".into()));
        let term = create(&engine.socket, spec(&ok)).unwrap();
        assert_eq!(
            term.labels.get(LABEL_POPUP).map(String::as_str),
            Some("tasks")
        );
        let done = collect(&engine.socket, &term.id, "tasks").unwrap();
        assert_eq!(
            done,
            Finish {
                title: "tasks".into(),
                body: "all good".into(),
                failed: false
            }
        );
        let bad = script("commands", "echo 'not here' >&2; exit 4", None);
        let term = create(&engine.socket, spec(&bad)).unwrap();
        let failed = collect(&engine.socket, &term.id, "commands").unwrap();
        assert!(failed.failed);
        assert_eq!(failed.body, "not here");
        assert_eq!(
            spoken(&failed).as_deref(),
            Some("commands failed: not here")
        );
        remove(&engine.socket, &term.id);
        assert!(exit_code(&engine.socket, &term.id).is_err());
    }
}
