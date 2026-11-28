use crate::colors::Palette;
use crate::input::{is_image_paste, key_bytes, line_key, paste_bytes};
use crate::link::{self, Job, Link, Msg};
use crate::metrics::{self, FrameClock, Metrics, PaintStats, PaintSummary};
use crate::mouse::{self, MouseAct};
use crate::paint::{Frame, paint_frame};
use crate::pasted;
use crate::selection::{self, Mode, Selection, Spot};
use crate::target::{self, Hit};
use futures::StreamExt;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, Bounds, ClipboardEntry, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable,
    Font, FontFallbacks, InteractiveElement, IntoElement, KeyDownEvent, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, ScrollWheelEvent, Styled, Subscription, Task, Window, WindowId, canvas, div, font, px,
};
use nixa_protocol::{Event, Key, Modes, Snapshot, TermId};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

pub const DEFAULT_FONT: &str = "JetBrainsMono Nerd Font";
pub const FALLBACK_FONTS: [&str; 4] = [
    "JetBrainsMono Nerd Font",
    "Symbols Nerd Font Mono",
    "JetBrains Mono",
    "Menlo",
];
pub const PADDING: f32 = 6.0;
const RESIZE_RETRY: Duration = Duration::from_millis(1000);

pub fn terminal_font(family: &str) -> Font {
    let mut f = font(family.to_string());
    f.fallbacks = Some(FontFallbacks::from_fonts(
        FALLBACK_FONTS.iter().map(|s| s.to_string()).collect(),
    ));
    f
}
pub const DEFAULT_FONT_SIZE: f32 = 14.0;

#[derive(Debug, Clone, PartialEq)]
pub enum TermViewEvent {
    Engine(Box<Event>),
    Problem(String),
}

pub struct TermView {
    id: TermId,
    link: Link,
    focus: FocusHandle,
    palette: Palette,
    font: Font,
    font_size: Pixels,
    snap: Option<Arc<Snapshot>>,
    scroll: usize,
    sent_grid: Option<(u16, u16)>,
    metrics: Option<Metrics>,
    bounds: Option<Bounds<Pixels>>,
    selection: Option<Selection>,
    selecting: bool,
    last_mouse_cell: Option<(u16, u16)>,
    hover: Option<(u16, u16)>,
    cmd: bool,
    link_press: bool,
    scroll_rest: f32,
    problem: Option<String>,
    exited: Option<Option<i32>>,
    removed: bool,
    draft: bool,
    resize_retry: Option<Task<()>>,
    stats: Arc<PaintStats>,
    snapshots: u64,
    clock: FrameClock,
    seen: Option<(WindowId, Instant)>,
    stale: Option<bool>,
    _pump: Task<()>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<TermViewEvent> for TermView {}

impl Focusable for TermView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TermView {
    pub fn new(
        socket: PathBuf,
        id: TermId,
        window: &mut Window,
        cx: &mut Context<TermView>,
    ) -> TermView {
        let (link, mut rx) = link::start(socket, id.clone());
        let pump = cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(msg) = rx.try_recv() {
                    batch.push(msg);
                }
                if this.update(cx, |view, cx| view.on_msgs(batch, cx)).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        let subs = vec![
            cx.on_focus(&focus, window, |view, _, cx| view.focus_changed(true, cx)),
            cx.on_blur(&focus, window, |view, _, cx| view.focus_changed(false, cx)),
        ];
        link.send(Job::Snapshot {
            scroll: 0,
            since: None,
        });
        TermView {
            id,
            link,
            focus,
            palette: Palette::default(),
            font: terminal_font(DEFAULT_FONT),
            font_size: px(DEFAULT_FONT_SIZE),
            snap: None,
            scroll: 0,
            sent_grid: None,
            metrics: None,
            bounds: None,
            selection: None,
            selecting: false,
            last_mouse_cell: None,
            hover: None,
            cmd: false,
            link_press: false,
            scroll_rest: 0.0,
            problem: None,
            exited: None,
            removed: false,
            draft: false,
            resize_retry: None,
            stats: Arc::new(PaintStats::default()),
            snapshots: 0,
            clock: cx.default_global::<FrameClock>().clone(),
            seen: None,
            stale: None,
            _pump: pump,
            _subs: subs,
        }
    }

    pub fn id(&self) -> &TermId {
        &self.id
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snap.as_deref()
    }

    pub fn title(&self) -> Option<&str> {
        self.snap.as_deref().and_then(|s| s.title.as_deref())
    }

    pub fn draft(&self) -> bool {
        self.draft
    }

    pub fn exited(&self) -> Option<Option<i32>> {
        self.exited
    }

    pub fn grid(&self) -> Option<(u16, u16)> {
        self.sent_grid
    }

    pub fn cell_position(&self, row: u16, col: u16) -> Option<Point<Pixels>> {
        let (b, m) = (self.bounds?, self.metrics?);
        let o = metrics::cell_origin(&b, &m, row, col);
        Some(Point {
            x: o.x + m.cell_w / 2.0,
            y: o.y + m.line_h / 2.0,
        })
    }

    pub fn paint_stats(&self) -> PaintSummary {
        metrics::paint_summary(&self.stats)
    }

    pub fn snapshots_taken(&self) -> u64 {
        self.snapshots
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<TermView>) {
        self.palette = palette;
        cx.notify();
    }

    pub fn set_font(&mut self, family: &str, size: f32, cx: &mut Context<TermView>) {
        self.font = terminal_font(family);
        self.font_size = px(size);
        cx.notify();
    }

    pub fn selected_text(&self) -> Option<String> {
        let snap = self.snap.as_deref()?;
        let text = self.selection?.text(snap);
        (!text.is_empty()).then_some(text)
    }

    pub fn link_at(&self, pos: Point<Pixels>) -> Option<String> {
        let (row, col) = self.cell_inside(pos)?;
        target::hit_at(self.snap.as_deref()?, row, col).map(|h| h.text)
    }

    pub fn paste_text(&mut self, text: &str, cx: &mut Context<TermView>) {
        let bracketed = self.modes().bracketed_paste;
        self.type_text(paste_bytes(text, bracketed), cx);
    }

    pub fn scroll_to_bottom(&mut self, cx: &mut Context<TermView>) {
        if self.scroll != 0 {
            self.scroll = 0;
            self.ask_snapshot(true);
            cx.notify();
        }
    }

    fn modes(&self) -> Modes {
        self.snap.as_deref().map(|s| s.modes).unwrap_or_default()
    }

    fn ask_snapshot(&self, fresh: bool) {
        let since = match self.snap.as_deref() {
            Some(s) if !fresh && s.scroll == self.scroll => Some(s.seq),
            _ => None,
        };
        self.link.send(Job::Snapshot {
            scroll: self.scroll,
            since,
        });
    }

    fn on_msgs(&mut self, batch: Vec<Msg>, cx: &mut Context<TermView>) {
        let mut want = None::<bool>;
        let mut ask = |fresh: bool| want = Some(want.unwrap_or(false) || fresh);
        let mut changed = false;
        for msg in batch {
            match msg {
                Msg::Linked => {
                    self.problem = None;
                    changed = true;
                    self.sent_grid = None;
                    ask(true);
                }
                Msg::Unlinked => {}
                Msg::ResizeFailed(rows, cols) => self.retry_resize((rows, cols), cx),
                Msg::Failed(note) => {
                    cx.emit(TermViewEvent::Problem(note.clone()));
                    self.problem = Some(note);
                    changed = true;
                }
                Msg::Snapshot(s) => {
                    if s.id != self.id {
                        continue;
                    }
                    self.problem = None;
                    if self.scroll > s.history {
                        self.scroll = s.history;
                    }
                    if s.scroll != self.scroll {
                        ask(true);
                    }
                    self.snap = Some(Arc::new(s));
                    self.snapshots += 1;
                    changed = true;
                }
                Msg::Event(ev) => {
                    if ev.term_id() != &self.id {
                        continue;
                    }
                    match &ev {
                        Event::OutputChanged { seq, .. } => {
                            if self.snap.as_deref().map(|s| s.seq) != Some(*seq) {
                                ask(false);
                            }
                        }
                        Event::Resized { .. } | Event::Title { .. } => ask(true),
                        Event::Exit { code, .. } => {
                            self.exited = Some(*code);
                            changed = true;
                            ask(true);
                        }
                        Event::Respawned { .. } => {
                            self.exited = None;
                            changed = true;
                            ask(true);
                        }
                        Event::Removed { .. } => {
                            self.removed = true;
                            changed = true;
                        }
                        Event::Draft { active, .. } => {
                            changed |= self.draft != *active;
                            self.draft = *active;
                        }
                        _ => {}
                    }
                    if !ev.is_output() {
                        cx.emit(TermViewEvent::Engine(Box::new(ev)));
                    }
                }
            }
        }
        if let Some(fresh) = want
            && !self.removed
        {
            if metrics::left_out(&self.clock, self.seen) {
                self.stale = Some(self.stale.unwrap_or(false) || fresh);
            } else {
                self.ask_snapshot(fresh);
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn retry_resize(&mut self, failed: (u16, u16), cx: &mut Context<TermView>) {
        if self.sent_grid != Some(failed) {
            return;
        }
        self.resize_retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESIZE_RETRY).await;
            this.update(cx, |view, cx| {
                if view.sent_grid == Some(failed) {
                    view.sent_grid = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn sync_layout(&mut self, bounds: Bounds<Pixels>, m: Metrics, window: WindowId) {
        let now = Instant::now();
        self.seen = Some((window, now));
        metrics::mark_frame(&self.clock, window, now);
        if let Some(fresh) = self.stale.take()
            && !self.removed
        {
            self.ask_snapshot(fresh);
        }
        self.bounds = Some(bounds);
        self.metrics = Some(m);
        let grid = metrics::grid_size(bounds.size, &m);
        if self.sent_grid != Some(grid) && !self.removed {
            self.sent_grid = Some(grid);
            self.link.send(Job::Resize(grid.0, grid.1));
            self.ask_snapshot(true);
        }
    }

    fn focus_changed(&mut self, focused: bool, cx: &mut Context<TermView>) {
        if self.modes().focus_events {
            let seq = if focused { "\x1b[I" } else { "\x1b[O" };
            self.link.send(Job::Keys(vec![Key::Raw(seq.to_string())]));
        }
        cx.notify();
    }

    fn type_text(&mut self, text: String, cx: &mut Context<TermView>) {
        if self.removed || text.is_empty() {
            return;
        }
        self.selection = None;
        if self.scroll != 0 {
            self.scroll = 0;
            self.ask_snapshot(true);
        }
        self.link.send(Job::Input(text));
        cx.notify();
    }

    fn send_raw(&self, bytes: Vec<u8>) {
        if let Ok(s) = String::from_utf8(bytes) {
            self.link.send(Job::Keys(vec![Key::Raw(s)]));
        }
    }

    fn cell(&self, pos: Point<Pixels>) -> Option<(u16, u16)> {
        let (b, m) = (self.bounds?, self.metrics?);
        let (rows, cols) = match self.snap.as_deref() {
            Some(s) => (s.rows, s.cols),
            None => self.sent_grid?,
        };
        Some(metrics::cell_at(pos, &b, &m, rows, cols))
    }

    fn cell_inside(&self, pos: Point<Pixels>) -> Option<(u16, u16)> {
        if !self.bounds?.contains(&pos) {
            return None;
        }
        self.cell(pos)
    }

    fn hover_link(&self) -> Option<Hit> {
        if !self.cmd {
            return None;
        }
        let (row, col) = self.hover?;
        target::hit_at(self.snap.as_deref()?, row, col)
    }

    fn set_hover(&mut self, cell: Option<(u16, u16)>, cmd: bool, cx: &mut Context<TermView>) {
        let before = self.hover_link();
        self.hover = cell;
        self.cmd = cmd;
        if self.hover_link() != before {
            cx.notify();
        }
    }

    fn on_modifiers(
        &mut self,
        ev: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<TermView>,
    ) {
        let cell = self.cell_inside(window.mouse_position());
        self.set_hover(cell, ev.modifiers.platform, cx);
    }

    fn paste_image(&mut self, cx: &mut Context<TermView>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(image) = item.entries().iter().find_map(|e| match e {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) else {
            return false;
        };
        let dir = pasted::folder();
        let job = cx.background_executor().spawn(async move {
            pasted::save(&dir, image.format, &image.bytes, SystemTime::now())
        });
        cx.spawn(async move |this, cx| {
            let saved = job.await;
            this.update(cx, |view, cx| match saved {
                Ok(path) => view.paste_text(&pasted::shell_quote(&path), cx),
                Err(e) => cx.emit(TermViewEvent::Problem(format!(
                    "Cannot save the pasted image: {e}"
                ))),
            })
            .ok();
        })
        .detach();
        true
    }

    fn spot(&self, row: u16, col: u16) -> Spot {
        let scroll = self.snap.as_deref().map(|s| s.scroll).unwrap_or(0);
        Spot {
            line: selection::line_of(row, scroll),
            col,
        }
    }

    fn copy(&self, cx: &mut Context<TermView>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<TermView>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn scroll_by(&mut self, lines: i64, cx: &mut Context<TermView>) {
        let history = self.snap.as_deref().map(|s| s.history).unwrap_or(0) as i64;
        let next = (self.scroll as i64 + lines).clamp(0, history) as usize;
        if next != self.scroll {
            self.scroll = next;
            self.ask_snapshot(true);
            cx.notify();
        }
    }

    pub fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<TermView>) {
        let ks = &ev.keystroke;
        if let Some(seq) = line_key(ks) {
            self.type_text(seq.to_string(), cx);
            cx.stop_propagation();
            return;
        }
        if ks.modifiers.platform {
            let handled = match ks.key.as_str() {
                "c" => {
                    self.copy(cx);
                    true
                }
                "v" => {
                    self.paste(cx);
                    true
                }
                "a" => {
                    self.selection = self.snap.as_deref().map(selection::all);
                    cx.notify();
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
            }
            return;
        }
        if is_image_paste(ks) && self.paste_image(cx) {
            cx.stop_propagation();
            return;
        }
        let modes = self.modes();
        if ks.modifiers.shift && !modes.alt_screen {
            let page = self.snap.as_deref().map(|s| s.rows as i64).unwrap_or(24);
            let lines = match ks.key.as_str() {
                "pageup" => Some(page),
                "pagedown" => Some(-page),
                "home" => Some(i64::MAX / 2),
                "end" => Some(i64::MIN / 2),
                _ => None,
            };
            if let Some(n) = lines {
                self.scroll_by(n, cx);
                cx.stop_propagation();
                return;
            }
        }
        if let Some(bytes) = key_bytes(ks, modes.app_cursor) {
            self.type_text(String::from_utf8_lossy(&bytes).into_owned(), cx);
            cx.stop_propagation();
        }
    }

    pub fn on_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<TermView>,
    ) {
        window.focus(&self.focus);
        self.link_press = ev.button == MouseButton::Left
            && ev.modifiers.platform
            && self.link_at(ev.position).is_some();
        if self.link_press {
            return;
        }
        let Some((row, col)) = self.cell(ev.position) else {
            return;
        };
        let modes = self.modes();
        if mouse::wants_mouse(&modes) && !ev.modifiers.shift {
            if let Some(b) =
                mouse::report(MouseAct::Press(ev.button), row, col, &ev.modifiers, &modes)
            {
                self.send_raw(b);
            }
            self.last_mouse_cell = Some((row, col));
            return;
        }
        if ev.button != MouseButton::Left {
            return;
        }
        let at = self.spot(row, col);
        let mode = match ev.click_count {
            0 | 1 => Mode::Simple,
            2 => Mode::Word,
            _ => Mode::Line,
        };
        match (&mut self.selection, ev.modifiers.shift) {
            (Some(sel), true) => sel.head = at,
            _ => self.selection = Some(Selection::start(at, mode)),
        }
        self.selecting = true;
        cx.notify();
    }

    pub fn on_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<TermView>,
    ) {
        let inside = self.cell_inside(ev.position);
        self.set_hover(inside, ev.modifiers.platform, cx);
        let Some((row, col)) = self.cell(ev.position) else {
            return;
        };
        if self.selecting {
            if ev.pressed_button == Some(MouseButton::Left) {
                let at = self.spot(row, col);
                if let Some(sel) = &mut self.selection
                    && sel.head != at
                {
                    sel.head = at;
                    cx.notify();
                }
            }
            return;
        }
        let modes = self.modes();
        if !mouse::wants_mouse(&modes) || self.last_mouse_cell == Some((row, col)) {
            return;
        }
        self.last_mouse_cell = Some((row, col));
        let act = match ev.pressed_button {
            Some(b) => MouseAct::Drag(b),
            None => MouseAct::Move,
        };
        if let Some(b) = mouse::report(act, row, col, &ev.modifiers, &modes) {
            self.send_raw(b);
        }
    }

    pub fn on_mouse_up(
        &mut self,
        ev: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<TermView>,
    ) {
        if self.link_press && ev.button == MouseButton::Left {
            self.link_press = false;
            return;
        }
        if self.selecting && ev.button == MouseButton::Left {
            self.selecting = false;
            if self.selection.is_some_and(|s| s.is_empty()) {
                self.selection = None;
            }
            cx.notify();
            return;
        }
        let modes = self.modes();
        if !mouse::wants_mouse(&modes) || ev.modifiers.shift {
            return;
        }
        let Some((row, col)) = self.cell(ev.position) else {
            return;
        };
        if let Some(b) = mouse::report(
            MouseAct::Release(ev.button),
            row,
            col,
            &ev.modifiers,
            &modes,
        ) {
            self.send_raw(b);
        }
    }

    pub fn on_scroll(
        &mut self,
        ev: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<TermView>,
    ) {
        let Some(m) = self.metrics else {
            return;
        };
        self.scroll_rest += ev.delta.pixel_delta(m.line_h).y / m.line_h;
        let lines = self.scroll_rest.trunc() as i64;
        self.scroll_rest -= lines as f32;
        if lines == 0 {
            return;
        }
        cx.stop_propagation();
        let modes = self.modes();
        if mouse::wants_mouse(&modes) {
            let Some((row, col)) = self.cell(ev.position) else {
                return;
            };
            let act = if lines > 0 {
                MouseAct::WheelUp
            } else {
                MouseAct::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(20) {
                if let Some(b) = mouse::report(act, row, col, &ev.modifiers, &modes) {
                    self.send_raw(b);
                }
            }
            return;
        }
        if modes.alt_screen {
            let key = if lines > 0 { Key::Up } else { Key::Down };
            let n = lines.unsigned_abs().min(20) as usize;
            self.link.send(Job::Keys(vec![key; n]));
            return;
        }
        self.scroll_by(lines, cx);
    }

    fn note(&self) -> Option<String> {
        if self.removed {
            return Some(format!("terminal {} was removed", self.id));
        }
        if let Some(p) = &self.problem {
            return Some(format!("nixa-engine: {p}"));
        }
        if let Some(code) = self.exited {
            return Some(match code {
                Some(c) => format!("program ended with code {c}"),
                None => "program ended".to_string(),
            });
        }
        if self.scroll > 0 {
            return Some(format!("scrolled up {} lines", self.scroll));
        }
        None
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<TermView>) -> impl IntoElement {
        let entity = cx.entity();
        let snap = self.snap.clone();
        let palette = self.palette;
        let font = self.font.clone();
        let font_size = self.font_size;
        let selection = self.selection;
        let focus = self.focus.clone();
        let note = self.note();
        let stats = self.stats.clone();
        let pointer = window.mouse_position();
        let link = self
            .hover_link()
            .filter(|_| self.bounds.is_some_and(|b| b.contains(&pointer)))
            .map(|h| (h.row, h.start, h.end));
        let surface = canvas(
            move |bounds, window, cx| {
                let m = metrics::measure(window, &font, font_size);
                let id = window.window_handle().window_id();
                entity.update(cx, |view, _| view.sync_layout(bounds, m, id));
                Frame {
                    snap,
                    palette,
                    font,
                    metrics: m,
                    selection,
                    focused: focus.is_focused(window),
                    stats,
                    link,
                }
            },
            |bounds, frame, window, cx| paint_frame(bounds, &frame, window, cx),
        )
        .size_full();
        div()
            .key_context("NixaTerm")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .p(px(PADDING))
            .bg(palette.background)
            .when(link.is_some(), |el| el.cursor_pointer())
            .on_key_down(cx.listener(Self::on_key))
            .on_modifiers_changed(cx.listener(Self::on_modifiers))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(surface)
            .children(note.map(|text| {
                div()
                    .absolute()
                    .top(px(6.0))
                    .right(px(10.0))
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(palette.ansi[0])
                    .text_color(palette.foreground)
                    .text_size(px(12.0))
                    .child(text)
            }))
    }
}
