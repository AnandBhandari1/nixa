use anyhow::{Result, bail};
use gpui::{
    App, AppContext, Application, AsyncApp, Bounds, Context, Entity, IntoElement, ParentElement,
    Render, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, canvas, div, point, px,
    rgb, size,
};
use nixa_protocol::{CreateSpec, Reply, Request, TermId, connect, paths};
use nixa_termview::TermView;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FLOOD: &str = r#"sleep 1; L=$(printf '\033[31mred\033[0m \033[1;32mbold green\033[0m \033[48;5;236m shaded \033[0m \033[3mitalic\033[0m plain text to fill the line up to the right edge of the pane'); while :; do yes "$L" | head -n 4000; seq 1 4000; done"#;
const WIN_X: f32 = 60.0;
const WIN_Y: f32 = 60.0;
const WIN_W: f32 = 1400.0;
const WIN_H: f32 = 760.0;
const WARM_UP: Duration = Duration::from_millis(2500);
const STUTTER: Duration = Duration::from_millis(50);
const HIDE_AFTER: Duration = Duration::from_millis(1500);

struct Grid {
    views: Vec<Entity<TermView>>,
    hidden: Vec<Entity<TermView>>,
    hide_at: Instant,
    cols: usize,
    stamps: Arc<Mutex<Vec<Instant>>>,
}

impl Render for Grid {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Grid>) -> impl IntoElement {
        let typing = self.hidden.iter().filter(|v| v.read(cx).draft()).count();
        let stamps = self.stamps.clone();
        let mark = canvas(
            |_, _, _| {},
            move |_, _, _, _| {
                if let Ok(mut s) = stamps.lock() {
                    s.push(Instant::now());
                }
            },
        )
        .absolute()
        .size(px(1.0));
        let rows: Vec<_> = self
            .views
            .chunks(self.cols.max(1))
            .map(|chunk| {
                div().flex().flex_row().flex_1().gap(px(2.0)).children(
                    chunk
                        .iter()
                        .map(|v| div().flex_1().h_full().child(v.clone())),
                )
            })
            .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .bg(rgb(0x303030))
            .child(mark)
            .children(rows)
            .children(
                (Instant::now() < self.hide_at && !self.hidden.is_empty()).then(|| {
                    div().flex().flex_row().h(px(120.0)).children(
                        self.hidden
                            .iter()
                            .map(|v| div().flex_1().h_full().child(v.clone())),
                    )
                }),
            )
            .children((typing > 0).then(|| div().child(format!("{typing} hidden drafts"))))
    }
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn call(socket: &Path, req: &Request) -> Result<Reply> {
    let mut c = connect(socket)?;
    c.set_timeout(Some(Duration::from_secs(5)))?;
    c.call(req)
}

fn open_terms(socket: &Path, n: usize) -> Result<Vec<TermId>> {
    let cwd = std::env::temp_dir().display().to_string();
    let mut ids = Vec::new();
    for i in 0..n {
        let spec = CreateSpec {
            name: Some(format!("flood{i}")),
            cmd: vec!["/bin/sh".to_string(), "-c".to_string(), FLOOD.to_string()],
            cwd: Some(cwd.clone()),
            ..CreateSpec::default()
        };
        match call(socket, &Request::Create { spec })? {
            Reply::Term { term } => ids.push(term.id),
            other => bail!("unexpected reply to create: {other:?}"),
        }
    }
    Ok(ids)
}

fn remove_all(socket: &Path, ids: &[TermId]) {
    for id in ids {
        call(socket, &Request::Remove { id: id.clone() }).ok();
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn spread(mut v: Vec<Duration>) -> String {
    if v.is_empty() {
        return "none".to_string();
    }
    v.sort();
    let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    format!(
        "n={} p50={:.1}ms p95={:.1}ms p99={:.1}ms max={:.1}ms",
        v.len(),
        ms(at(0.5)),
        ms(at(0.95)),
        ms(at(0.99)),
        ms(at(1.0))
    )
}

fn report(
    cx: &mut AsyncApp,
    grid: &Entity<Grid>,
    began: Instant,
    flood_from: Instant,
    ended: Instant,
) {
    let Ok((stamps, views, hidden)) = grid.read_with(cx, |g, _| {
        (
            g.stamps.lock().map(|s| s.clone()).unwrap_or_default(),
            g.views.clone(),
            g.hidden.clone(),
        )
    }) else {
        return;
    };
    let first = stamps.first().map(|t| t.duration_since(began));
    let gaps: Vec<Duration> = stamps
        .windows(2)
        .filter(|w| w[0] >= flood_from && w[1] <= ended)
        .map(|w| w[1].duration_since(w[0]))
        .collect();
    let span = ended.duration_since(flood_from).as_secs_f64();
    let stutters = gaps.iter().filter(|g| **g > STUTTER).count();
    println!(
        "panes: first window frame {:.0}ms after start",
        first.map(ms).unwrap_or(-1.0)
    );
    println!(
        "panes: frames during flood {:.1} per second",
        gaps.len() as f64 / span
    );
    println!("panes: frame gaps {}", spread(gaps.clone()));
    println!("panes: gaps over {}ms: {stutters}", STUTTER.as_millis());
    let mut total = Duration::ZERO;
    let mut frames = 0;
    let mut worst = Duration::ZERO;
    for v in &views {
        if let Ok(s) = v.read_with(cx, |v, _| v.paint_stats()) {
            total += s.total;
            frames += s.frames;
            worst = worst.max(s.max);
        }
    }
    println!(
        "panes: pane paint {} paints, {:.2}ms average, {:.1}ms worst, {:.0}% of the main thread",
        frames,
        ms(total) / frames.max(1) as f64,
        ms(worst),
        100.0 * total.as_secs_f64() / ended.duration_since(began).as_secs_f64()
    );
    let snaps: u64 = views
        .iter()
        .filter_map(|v| v.read_with(cx, |v, _| v.snapshots_taken()).ok())
        .sum();
    let hidden_snaps: u64 = hidden
        .iter()
        .filter_map(|v| v.read_with(cx, |v, _| v.snapshots_taken()).ok())
        .sum();
    let secs = ended.duration_since(began).as_secs_f64();
    println!(
        "panes: screen updates {:.0} per second per shown pane, {:.1} per second per hidden pane",
        snaps as f64 / secs / views.len().max(1) as f64,
        hidden_snaps as f64 / secs / hidden.len().max(1) as f64
    );
}

fn main() -> Result<()> {
    let began = Instant::now();
    let socket: PathBuf = paths::socket_path();
    if call(&socket, &Request::Ping).is_err() {
        bail!(
            "no nixa-engine answers on {}; start one with nixa-engine --socket <path> and set NIXA_ENGINE_SOCKET",
            socket.display()
        );
    }
    let n = env_num("NIXA_PANES", 8).clamp(1, 32) as usize;
    let hidden_n = env_num("NIXA_PANES_HIDDEN", 0).clamp(0, 32) as usize;
    let seconds = env_num("NIXA_PANES_SECONDS", 10);
    let ids = open_terms(&socket, n + hidden_n)?;
    println!(
        "panes: {n} shown and {hidden_n} hidden terminals on {}",
        socket.display()
    );
    let (q_socket, q_ids) = (socket.clone(), ids.clone());
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::new(point(px(WIN_X), px(WIN_Y)), size(px(WIN_W), px(WIN_H)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(format!("nixa panes {n}").into()),
                ..TitlebarOptions::default()
            }),
            ..WindowOptions::default()
        };
        let cols = (n as f64).sqrt().ceil() as usize;
        let view_socket = q_socket.clone();
        let view_ids = q_ids.clone();
        let opened = cx.open_window(options, move |window, cx| {
            let mut views: Vec<Entity<TermView>> = view_ids
                .iter()
                .map(|id| cx.new(|cx| TermView::new(view_socket.clone(), id.clone(), window, cx)))
                .collect();
            let hidden = views.split_off(n);
            cx.new(|_| Grid {
                views,
                hidden,
                hide_at: Instant::now() + HIDE_AFTER,
                cols,
                stamps: Arc::new(Mutex::new(Vec::new())),
            })
        });
        let grid = match opened.and_then(|h| h.entity(cx)) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("panes: cannot open the window: {e}");
                remove_all(&q_socket, &q_ids);
                cx.quit();
                return;
            }
        };
        cx.activate(true);
        let (t_socket, t_ids) = (q_socket.clone(), q_ids.clone());
        cx.spawn(async move |cx| {
            cx.background_executor().timer(WARM_UP).await;
            let flood_from = Instant::now();
            cx.background_executor()
                .timer(Duration::from_secs(seconds))
                .await;
            let ended = Instant::now();
            report(cx, &grid, began, flood_from, ended);
            remove_all(&t_socket, &t_ids);
            cx.update(|cx| cx.quit()).ok();
        })
        .detach();
    });
    Ok(())
}
