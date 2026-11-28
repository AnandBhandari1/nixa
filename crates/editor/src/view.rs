use crate::area::{self, LINE_H, Layout, TextArea};
use crate::buffer::Buffer;
use crate::doc::{self, DiskAction, Doc};
use crate::files::{self, Read, Save as Saving, Stamp, ViewOnly};
use crate::find::{self, Match};
use crate::keys::*;
use crate::lang::{self, Family, Language, Shape};
use crate::markdown::{self, Block, Link};
use crate::{md_view, mermaid};
use gpui::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, CursorStyle, Div, Entity, EntityInputHandler, EventEmitter,
    ExternalPaths, FocusHandle, Focusable, FontWeight, ImageFormat, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Pixels, Point, Render, ScrollHandle,
    ScrollWheelEvent, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledImage, Subscription, Task,
    UTF16Selection, Window, canvas, div, img, point, prelude::FluentBuilder, px,
};
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::{InputEvent, TextInput, from_utf16, to_utf16};
use nixa_ui::{Theme, theme};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub const WATCH_MS: u64 = 1000;
pub const NOTICE_MS: u64 = 3000;
const NARROW: f32 = 420.0;
const CHIPS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Empty,
    Loading,
    Text,
    Image,
    Missing,
    Folder,
    Binary,
    Large(String),
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Edit,
    Preview,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    Close,
    Edited,
    Opened(String),
    Saved(String),
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub readonly: bool,
}

#[derive(Clone)]
pub enum Source {
    Bytes(Arc<gpui::Image>),
    File(std::path::PathBuf),
}

#[derive(Clone)]
pub struct Pic {
    pub source: Source,
    pub dims: Option<(u32, u32)>,
    pub bytes: u64,
}

fn preview_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("nixa-editor-preview")
}

fn sweep_previews() {
    let Ok(list) = std::fs::read_dir(preview_dir()) else { return };
    let day = Duration::from_secs(24 * 3600);
    for e in list.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if old {
            std::fs::remove_file(e.path()).ok();
        }
    }
}

fn drop_pic(pic: Option<Pic>) {
    if let Some(Pic { source: Source::File(p), .. }) = pic {
        std::fs::remove_file(p).ok();
    }
}

fn svg_copy(bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = preview_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("svg-{}-{}.svg", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorCommand {
    Save,
    Find,
    Close,
    Preview,
    Source,
    OpenOutside,
    Reveal,
}

struct Loaded {
    read: Option<Read>,
    pic: Option<Result<Pic, State>>,
}

pub struct Editor {
    pub(crate) focus: FocusHandle,
    root_focus: FocusHandle,
    pub(crate) buf: Buffer,
    pub(crate) lang: Language,
    path: Option<String>,
    home: Option<String>,
    shape: Shape,
    state: State,
    doc: Option<Doc>,
    mode: Mode,
    readonly: bool,
    view_only: Option<ViewOnly>,
    pic: Option<Pic>,
    notice: Option<(String, bool)>,
    notice_seq: u64,
    closing: bool,
    saving: bool,
    close_when_saved: bool,
    recheck: bool,
    lid: u64,
    stamp: Option<Stamp>,
    link_ask: Option<(String, bool)>,
    link_ok: bool,
    find_open: bool,
    find_input: Entity<TextInput>,
    query: String,
    pub(crate) found: Vec<Match>,
    pub(crate) found_at: Option<usize>,
    found_rev: u64,
    edited_sent: bool,
    pub(crate) scroll: Point<Pixels>,
    pub(crate) autoscroll: bool,
    pub(crate) page: usize,
    pub(crate) layout: Option<Layout>,
    marked: Option<Range<usize>>,
    goto: Option<usize>,
    dragging: bool,
    scroll_grab: Option<Pixels>,
    width: f32,
    md: Option<(u64, Rc<Vec<Block>>)>,
    diagrams: Option<mermaid::Batch>,
    pub(crate) blocks: Option<(u64, Rc<Vec<bool>>)>,
    md_scroll: ScrollHandle,
    pub(crate) md_selection: md_view::Selection,
    _watch: Task<()>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for Editor {}

impl Drop for Editor {
    fn drop(&mut self) {
        drop_pic(self.pic.take());
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub fn commands() -> Vec<(&'static str, &'static str, EditorCommand)> {
    vec![
        ("Editor: Save", "⌘S", EditorCommand::Save),
        ("Editor: Find in file", "⌘F", EditorCommand::Find),
        ("Editor: Close", "", EditorCommand::Close),
        ("Editor: Show preview", "", EditorCommand::Preview),
        ("Editor: Show source", "", EditorCommand::Source),
        ("Editor: Open in default app", "", EditorCommand::OpenOutside),
        ("Editor: Reveal in Finder", "", EditorCommand::Reveal),
    ]
}

pub fn run_command(ed: &mut Editor, cmd: EditorCommand, window: &mut Window, cx: &mut Context<Editor>) {
    match cmd {
        EditorCommand::Save => ed.save(false, cx),
        EditorCommand::Find => ed.open_find(window, cx),
        EditorCommand::Close => {
            ed.request_close(cx);
        }
        EditorCommand::Preview => ed.set_mode(preview_mode(ed.shape), window, cx),
        EditorCommand::Source => ed.set_mode(Mode::Edit, window, cx),
        EditorCommand::OpenOutside => ed.open_outside(cx),
        EditorCommand::Reveal => ed.reveal(cx),
    }
}

fn default_mode(shape: Shape) -> Mode {
    match shape {
        Shape::Markdown | Shape::Mermaid => Mode::Preview,
        Shape::Image | Shape::Svg => Mode::Image,
        _ => Mode::Edit,
    }
}

fn preview_mode(shape: Shape) -> Mode {
    match shape {
        Shape::Markdown | Shape::Mermaid => Mode::Preview,
        _ => Mode::Image,
    }
}

fn image_format(ext: &str) -> Option<ImageFormat> {
    Some(match ext {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::Webp,
        "gif" => ImageFormat::Gif,
        "svg" => ImageFormat::Svg,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        _ => return None,
    })
}

fn load_pic(path: &str) -> Result<Pic, State> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(State::Missing),
        Err(e) => return Err(State::Error(e.to_string())),
    };
    if meta.is_dir() {
        return Err(State::Folder);
    }
    if meta.len() > files::IMAGE_LIMIT {
        return Err(State::Large("This image is over 16 MB, too large to show here.".into()));
    }
    let ext = lang::ext_of(path);
    let bytes = std::fs::read(path).map_err(|e| State::Error(e.to_string()))?;
    let size = bytes.len() as u64;
    let (format, bytes) = if ext == "ico" {
        let picture = ::image::load_from_memory_with_format(&bytes, ::image::ImageFormat::Ico)
            .map_err(|error| State::Error(error.to_string()))?;
        let mut png = std::io::Cursor::new(Vec::new());
        picture.write_to(&mut png, ::image::ImageFormat::Png)
            .map_err(|error| State::Error(error.to_string()))?;
        (ImageFormat::Png, png.into_inner())
    } else {
        let Some(format) = image_format(&ext) else {
            return Err(State::Error("This kind of image cannot be shown here.".into()));
        };
        (format, bytes)
    };
    let dims = crate::image::dims(&bytes);
    let source = if format == ImageFormat::Svg {
        Source::File(svg_copy(&bytes).map_err(|e| State::Error(e.to_string()))?)
    } else {
        Source::Bytes(Arc::new(gpui::Image::from_bytes(format, bytes)))
    };
    Ok(Pic { source, dims, bytes: size })
}

fn load(path: &str, shape: Shape) -> Loaded {
    match shape {
        Shape::Image => Loaded { read: None, pic: Some(load_pic(path)) },
        Shape::Svg => Loaded { read: Some(files::read(path)), pic: Some(load_pic(path)) },
        _ => Loaded { read: Some(files::read(path)), pic: None },
    }
}

fn state_of(read: Read) -> State {
    match read {
        Read::Text { .. } => State::Text,
        Read::Missing => State::Missing,
        Read::Folder => State::Folder,
        Read::Binary => State::Binary,
        Read::Large(e) => State::Large(e),
        Read::Error(e) => State::Error(e),
    }
}

fn btn(t: &Theme, id: &'static str, label: impl Into<SharedString>, tip: impl Into<SharedString>, active: bool, primary: bool, off: bool) -> Stateful<Div> {
    let primary = primary && !off;
    let (bg, fg, border) = if primary {
        (t.accent, t.accent_foreground, t.accent)
    } else if active {
        (t.selected, t.foreground, t.border_strong)
    } else {
        (t.raised, t.foreground, t.border)
    };
    let hover = if primary { t.accent_hover } else { t.hover };
    div()
        .id(id)
        .flex_none()
        .h(px(24.0))
        .px(px(9.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_size(px(12.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(if off { t.dim } else { fg })
        .tooltip(tooltip(tip))
        .when(off, |el| el.opacity(0.55).cursor_not_allowed())
        .when(!off, move |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .child(label.into())
}

impl Editor {
    pub fn new(path: Option<&str>, opts: Options, window: &mut Window, cx: &mut Context<Editor>) -> Editor {
        let find_input = cx.new(|cx| TextInput::new(cx, "Find in file"));
        let sub = cx.subscribe_in(&find_input, window, |ed: &mut Editor, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed(q) => ed.find_changed(q.clone(), cx),
            InputEvent::Submit(_) => ed.find_step(1, cx),
            InputEvent::Cancel => ed.close_find(window, cx),
        });
        let watch = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(WATCH_MS)).await;
                if this.update(cx, |ed, cx| ed.poll_disk(cx)).is_err() {
                    break;
                }
            }
        });
        let mut ed = Editor {
            focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            buf: Buffer::default(),
            lang: lang::language(""),
            path: None,
            home: std::env::var("HOME").ok().filter(|h| !h.is_empty()),
            shape: Shape::Text,
            state: State::Empty,
            doc: None,
            mode: Mode::Edit,
            readonly: opts.readonly,
            view_only: None,
            pic: None,
            notice: None,
            notice_seq: 0,
            closing: false,
            saving: false,
            close_when_saved: false,
            recheck: false,
            lid: 0,
            stamp: None,
            link_ask: None,
            link_ok: false,
            find_open: false,
            find_input,
            query: String::new(),
            found: Vec::new(),
            found_at: None,
            found_rev: 0,
            edited_sent: false,
            scroll: Point::default(),
            autoscroll: false,
            page: 20,
            layout: None,
            marked: None,
            goto: None,
            dragging: false,
            scroll_grab: None,
            width: 0.0,
            md: None,
            diagrams: None,
            blocks: None,
            md_scroll: ScrollHandle::new(),
            md_selection: md_view::Selection::default(),
            _watch: watch,
            _subs: vec![sub],
        };
        cx.background_executor().spawn(async { sweep_previews() }).detach();
        if let Some(p) = path {
            ed.load_path(p, cx);
        }
        ed
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn title(&self) -> String {
        self.path.as_deref().map(lang::file_name).filter(|n| !n.is_empty()).unwrap_or("No file").to_string()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn text(&self) -> &str {
        self.buf.text()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|(n, _)| n.as_str())
    }

    pub fn closing(&self) -> bool {
        self.closing
    }

    pub fn has_conflict(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.conflict.is_some())
    }

    pub fn finding(&self) -> bool {
        self.find_open
    }

    pub fn find_label(&self) -> String {
        find::count_label(&self.query, &self.found, self.found_at)
    }

    pub fn is_dirty(&self) -> bool {
        self.state == State::Text && self.doc.as_ref().is_some_and(|d| doc::is_dirty(d, self.buf.text()))
    }

    fn reset(&mut self) {
        self.buf = Buffer::default();
        self.doc = None;
        drop_pic(self.pic.take());
        self.view_only = None;
        self.notice = None;
        self.closing = false;
        self.close_when_saved = false;
        self.link_ask = None;
        self.link_ok = false;
        self.saving = false;
        self.recheck = false;
        self.find_open = false;
        self.found.clear();
        self.found_at = None;
        self.edited_sent = false;
        self.scroll = Point::default();
        self.layout = None;
        self.scroll_grab = None;
        self.marked = None;
        self.goto = None;
        self.md = None;
        self.diagrams = None;
        self.blocks = None;
        self.md_scroll = ScrollHandle::new();
        self.md_selection = md_view::Selection::default();
    }

    fn load_path(&mut self, raw: &str, cx: &mut Context<Editor>) {
        self.lid += 1;
        self.reset();
        let path = files::expand(raw, self.home.as_deref());
        self.shape = lang::shape(&path);
        self.lang = lang::language(&path);
        self.path = Some(path.clone());
        if !path.starts_with('/') {
            self.state = State::Error("Give the full path of the file. It must start with / or ~/.".into());
            cx.notify();
            return;
        }
        self.mode = default_mode(self.shape);
        self.state = State::Loading;
        self.stamp = Some(files::stamp(&path));
        let lid = self.lid;
        let shape = self.shape;
        let p = path.clone();
        let job = cx.background_executor().spawn(async move { load(&p, shape) });
        cx.spawn(async move |this, cx| {
            let loaded = job.await;
            this.update(cx, |ed, cx| ed.apply_loaded(lid, loaded, cx)).ok();
        })
        .detach();
        cx.emit(EditorEvent::Opened(path));
        cx.notify();
    }

    fn apply_loaded(&mut self, lid: u64, l: Loaded, cx: &mut Context<Editor>) {
        if lid != self.lid {
            return;
        }
        let mut pic_err = None;
        match l.pic {
            Some(Ok(p)) => drop_pic(self.pic.replace(p)),
            Some(Err(s)) => pic_err = Some(s),
            None => {}
        }
        match l.read {
            Some(Read::Text { doc, view_only }) => {
                self.buf.set_text(&doc.base);
                self.buf.readonly = self.readonly || view_only.is_some();
                if let Some(v) = &view_only {
                    self.notice = Some((files::view_only_notice(v), false));
                }
                self.view_only = view_only;
                self.doc = Some(doc);
                self.state = State::Text;
                if let Some(line) = self.goto.take() {
                    self.jump(line);
                }
                if self.shape == Shape::Svg && self.pic.is_none() {
                    self.mode = Mode::Edit;
                }
            }
            Some(other) => {
                self.state = if self.pic.is_some() { State::Image } else { state_of(other) };
            }
            None => {
                self.state = match (self.pic.is_some(), pic_err) {
                    (true, _) => State::Image,
                    (false, Some(s)) => s,
                    (false, None) => State::Error("Could not read this image.".into()),
                };
            }
        }
        cx.notify();
    }

    pub fn open(&mut self, path: &str, cx: &mut Context<Editor>) -> bool {
        if self.saving {
            self.say("Wait for the save to finish before opening another file.", true, cx);
            return false;
        }
        if self.is_dirty() {
            self.say("Save or undo your changes before opening another file.", true, cx);
            return false;
        }
        self.load_path(path, cx);
        true
    }

    pub fn go_to_line(&mut self, line: usize, cx: &mut Context<Editor>) {
        if self.state != State::Text {
            self.goto = Some(line);
            return;
        }
        self.jump(line);
        cx.notify();
    }

    fn jump(&mut self, line: usize) {
        let at = self.buf.line_start(line.saturating_sub(1));
        self.buf.move_to(at, false);
        self.marked = None;
        self.autoscroll = true;
    }

    fn refresh(&mut self, cx: &mut Context<Editor>) {
        let Some(path) = self.path.clone() else { return };
        let mode = self.mode;
        self.load_path(&path, cx);
        self.mode = mode;
    }

    fn reload_pic(&mut self, cx: &mut Context<Editor>) {
        if self.shape != Shape::Svg {
            return;
        }
        let Some(path) = self.path.clone() else { return };
        let lid = self.lid;
        let job = cx.background_executor().spawn(async move { load_pic(&path) });
        cx.spawn(async move |this, cx| {
            let pic = job.await;
            this.update(cx, |ed, cx| {
                match pic {
                    Ok(p) if ed.lid == lid => {
                        drop_pic(ed.pic.replace(p));
                        cx.notify();
                    }
                    Ok(p) => drop_pic(Some(p)),
                    Err(_) => {}
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn say(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Editor>) {
        self.notice = Some((text.into(), error));
        self.notice_seq += 1;
        let seq = self.notice_seq;
        if !error {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(NOTICE_MS)).await;
                this.update(cx, |ed, cx| {
                    if ed.notice_seq == seq {
                        ed.notice = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn after_edit(&mut self, cx: &mut Context<Editor>) {
        self.autoscroll = true;
        if !self.edited_sent && self.is_dirty() {
            self.edited_sent = true;
            cx.emit(EditorEvent::Edited);
        }
        cx.notify();
    }

    fn edit(&mut self, cx: &mut Context<Editor>, f: impl FnOnce(&mut Buffer) -> bool) {
        if self.state != State::Text {
            return;
        }
        if f(&mut self.buf) {
            self.marked = None;
            self.after_edit(cx);
        }
    }

    fn motion(&mut self, cx: &mut Context<Editor>, f: impl FnOnce(&mut Buffer)) {
        f(&mut self.buf);
        self.marked = None;
        self.autoscroll = true;
        cx.notify();
    }

    pub(crate) fn poll_disk(&mut self, cx: &mut Context<Editor>) {
        let Some(path) = self.path.clone() else { return };
        if matches!(self.state, State::Loading | State::Empty) || !path.starts_with('/') {
            return;
        }
        let now = files::stamp(&path);
        if self.stamp == Some(now) {
            return;
        }
        self.stamp = Some(now);
        self.check_disk(cx);
    }

    fn check_disk(&mut self, cx: &mut Context<Editor>) {
        if self.view_only.is_some() {
            return;
        }
        if self.saving {
            self.recheck = true;
            return;
        }
        if self.state != State::Text {
            self.refresh(cx);
            return;
        }
        let Some(path) = self.path.clone() else { return };
        let lid = self.lid;
        let job = cx.background_executor().spawn(async move { files::read_raw(&path) });
        cx.spawn(async move |this, cx| {
            let raw = job.await;
            this.update(cx, |ed, cx| ed.disk_read(lid, raw, cx)).ok();
        })
        .detach();
    }

    fn disk_read(&mut self, lid: u64, raw: std::io::Result<Option<String>>, cx: &mut Context<Editor>) {
        if lid != self.lid || self.saving {
            return;
        }
        let Some(d) = self.doc.clone() else { return };
        let raw = match raw {
            Ok(r) => r,
            Err(_) => {
                if self.is_dirty() {
                    self.say("The file on disk can no longer be shown as text. Your edits are still here.", true, cx);
                } else {
                    self.refresh(cx);
                }
                return;
            }
        };
        let (next, action) = doc::on_disk(&d, raw.as_deref(), self.buf.text());
        let base = next.base.clone();
        self.doc = Some(next);
        match action {
            DiskAction::Reload => {
                self.buf.replace_all_keep_place(&base);
                self.edited_sent = false;
                self.say("Reloaded. The file changed on disk.", false, cx);
                self.reload_pic(cx);
            }
            DiskAction::Adopt => self.reload_pic(cx),
            _ => {}
        }
        cx.notify();
    }

    pub fn save(&mut self, then_close: bool, cx: &mut Context<Editor>) {
        let (Some(d), Some(path)) = (self.doc.clone(), self.path.clone()) else { return };
        if self.buf.readonly || self.saving || self.state != State::Text {
            return;
        }
        if d.conflict.is_some() {
            self.say("Choose Reload or Keep mine first.", true, cx);
            return;
        }
        if !self.is_dirty() {
            if then_close {
                cx.emit(EditorEvent::Close);
            }
            return;
        }
        if !self.link_ok
            && let Some(real) = files::link_outside(&path)
        {
            self.link_ask = Some((real.to_string_lossy().into_owned(), then_close));
            cx.notify();
            return;
        }
        self.link_ask = None;
        self.saving = true;
        self.notice = Some(("Saving…".into(), false));
        cx.notify();
        let value = self.buf.text().to_string();
        let lid = self.lid;
        let p = path.clone();
        let job = cx.background_executor().spawn(async move { files::save(&p, &d, &value) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |ed, cx| ed.saved(lid, path, result, then_close, cx)).ok();
        })
        .detach();
    }

    fn saved(&mut self, lid: u64, path: String, result: Saving, then_close: bool, cx: &mut Context<Editor>) {
        self.saving = false;
        let deferred = std::mem::take(&mut self.close_when_saved);
        let then_close = then_close || deferred;
        if lid != self.lid {
            return;
        }
        match result {
            Saving::Saved(d) => {
                self.doc = Some(d);
                self.stamp = Some(files::stamp(&path));
                self.edited_sent = self.is_dirty();
                self.say("Saved", false, cx);
                self.reload_pic(cx);
                cx.emit(EditorEvent::Saved(path));
                if then_close && self.is_dirty() {
                    self.say("New changes came in while saving. Save again to keep them.", true, cx);
                } else if then_close {
                    cx.emit(EditorEvent::Close);
                }
            }
            Saving::Conflict(d) => {
                self.doc = Some(d);
                self.notice = None;
            }
            Saving::Failed(e) => self.say(format!("Could not save: {e}"), true, cx),
        }
        if self.recheck {
            self.recheck = false;
            self.check_disk(cx);
        }
        cx.notify();
    }

    pub fn link_prompt(&self) -> Option<&str> {
        self.link_ask.as_ref().map(|(p, _)| p.as_str())
    }

    pub fn save_through_link(&mut self, cx: &mut Context<Editor>) {
        let Some((_, then_close)) = self.link_ask.take() else { return };
        self.link_ok = true;
        self.save(then_close, cx);
    }

    pub fn reload_from_disk(&mut self, cx: &mut Context<Editor>) {
        let Some(d) = self.doc.clone() else { return };
        match doc::reload(&d) {
            None => cx.emit(EditorEvent::Close),
            Some(next) => {
                self.buf.replace_all_keep_place(&next.base);
                self.doc = Some(next);
                self.edited_sent = false;
                self.say("Reloaded from disk.", false, cx);
                self.reload_pic(cx);
            }
        }
    }

    pub fn keep_mine(&mut self, cx: &mut Context<Editor>) {
        let Some(d) = self.doc.clone() else { return };
        let from_save = d.conflict.as_ref().is_some_and(|c| c.from_save);
        self.doc = Some(doc::keep_mine(&d));
        if from_save {
            self.save(false, cx);
        } else {
            self.say("Keeping your version. Save to write it to disk.", false, cx);
        }
    }

    pub fn request_close(&mut self, cx: &mut Context<Editor>) -> bool {
        if self.saving {
            self.close_when_saved = true;
            self.say("Saving… The window closes when the save is done.", false, cx);
            return false;
        }
        if self.is_dirty() {
            self.closing = true;
            cx.notify();
            return false;
        }
        cx.emit(EditorEvent::Close);
        true
    }

    fn launch(&mut self, cmd: String, args: Vec<String>, failure: &'static str, cx: &mut Context<Editor>) {
        let job = cx.background_executor().spawn(async move {
            match std::process::Command::new(&cmd).args(&args).output() {
                Ok(o) if o.status.success() => None,
                Ok(o) => {
                    let msg = String::from_utf8_lossy(if o.stderr.is_empty() { &o.stdout } else { &o.stderr }).trim().to_string();
                    Some(if msg.is_empty() { format!("exit {}", o.status.code().unwrap_or(-1)) } else { msg })
                }
                Err(e) => Some(e.to_string()),
            }
        });
        cx.spawn(async move |this, cx| {
            if let Some(err) = job.await {
                this.update(cx, |ed, cx| ed.say(format!("{failure}: {err}"), true, cx)).ok();
            }
        })
        .detach();
    }

    fn open_outside(&mut self, cx: &mut Context<Editor>) {
        if let Some(p) = self.path.clone().filter(|p| p.starts_with('/')) {
            self.launch("open".into(), vec![p], "Could not open", cx);
        }
    }

    fn reveal(&mut self, cx: &mut Context<Editor>) {
        if let Some(p) = self.path.clone().filter(|p| p.starts_with('/')) {
            self.launch("open".into(), vec!["-R".into(), p], "Could not reveal", cx);
        }
    }

    pub fn follow_link(&mut self, url: &str, cx: &mut Context<Editor>) {
        let folder = self.path.as_deref().map(lang::folder_of).unwrap_or("/").to_string();
        match markdown::link_target(url, &folder) {
            Link::Web(u) => self.launch("open".into(), vec![u], "Could not open link", cx),
            Link::File(p) => {
                self.open(&p, cx);
            }
            Link::Outside => self.say("This link points outside the document folder.", true, cx),
            Link::Ignore => {}
        }
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Editor>) {
        let ok = match mode {
            Mode::Edit => self.state == State::Text,
            Mode::Preview => self.state == State::Text && matches!(self.shape, Shape::Markdown | Shape::Mermaid),
            Mode::Image => self.pic.is_some(),
        };
        if !ok {
            return;
        }
        self.mode = mode;
        if mode == Mode::Edit {
            window.focus(&self.focus);
        } else {
            window.focus(&self.root_focus);
        }
        cx.notify();
    }

    fn modes(&self) -> Vec<(&'static str, &'static str, &'static str, Mode)> {
        if self.state != State::Text {
            return vec![];
        }
        match self.shape {
            Shape::Markdown => vec![
                ("mode-edit", "Edit", "Edit the Markdown text", Mode::Edit),
                ("mode-preview", "Preview", "Show the Markdown drawn", Mode::Preview),
            ],
            Shape::Mermaid => vec![
                ("mode-edit", "Edit", "Edit the Mermaid source", Mode::Edit),
                ("mode-preview", "Preview", "Draw the Mermaid diagram", Mode::Preview),
            ],
            Shape::Svg if self.pic.is_some() => vec![
                ("mode-image", "Image", "Show the picture", Mode::Image),
                ("mode-source", "Source", "Edit the SVG source", Mode::Edit),
            ],
            _ => vec![],
        }
    }

    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Editor>) {
        if self.state != State::Text {
            return;
        }
        if self.mode != Mode::Edit {
            self.mode = Mode::Edit;
        }
        self.find_open = true;
        let sel = self.buf.selected().to_string();
        if !sel.is_empty() && !sel.contains('\n') {
            self.find_input.update(cx, |i, cx| i.set_value(sel.clone(), cx));
            self.query = sel;
        }
        self.recompute();
        self.found_at = find::nearest(&self.found, self.buf.sel.range().start);
        let fh = self.find_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Editor>) {
        self.find_open = false;
        self.found.clear();
        self.found_at = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Editor>) {
        if self.find_open {
            self.close_find(window, cx);
        } else {
            self.open_find(window, cx);
        }
    }

    fn recompute(&mut self) {
        self.found = find::matches(self.buf.text(), &self.query);
        self.found_rev = self.buf.rev;
        if self.found_at.is_some_and(|i| i >= self.found.len()) {
            self.found_at = self.found.len().checked_sub(1);
        }
    }

    pub fn find_changed(&mut self, query: String, cx: &mut Context<Editor>) {
        self.query = query;
        self.recompute();
        self.found_at = find::nearest(&self.found, self.buf.sel.range().start);
        self.select_found(cx);
    }

    pub fn find_step(&mut self, by: isize, cx: &mut Context<Editor>) {
        if self.found_rev != self.buf.rev {
            self.recompute();
        }
        self.found_at = find::step(self.found_at, self.found.len(), by);
        self.select_found(cx);
    }

    fn find_go(&mut self, i: usize, cx: &mut Context<Editor>) {
        if i < self.found.len() {
            self.found_at = Some(i);
            self.select_found(cx);
        }
    }

    fn select_found(&mut self, cx: &mut Context<Editor>) {
        if let Some(m) = self.found_at.and_then(|i| self.found.get(i)).copied() {
            self.buf.select(m.start..m.end);
            self.autoscroll = true;
        }
        cx.notify();
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Editor>) {
        if self.find_open {
            self.close_find(window, cx);
        } else if self.link_ask.is_some() {
            self.link_ask = None;
            cx.notify();
        } else if self.closing || self.close_when_saved {
            self.closing = false;
            self.close_when_saved = false;
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn ensure_caches(&mut self, cx: &mut Context<Editor>) {
        let rev = self.buf.rev;
        let needs_blocks = matches!(self.lang.family, Family::CLike | Family::Sql | Family::Markup | Family::Lua | Family::Haskell | Family::Markdown);
        if needs_blocks && self.blocks.as_ref().is_none_or(|(r, _)| *r != rev) {
            let states = crate::syntax::block_states(self.buf.text().split('\n'), &self.lang);
            self.blocks = Some((rev, Rc::new(states)));
        }
        if self.mode == Mode::Preview && self.state == State::Text && self.shape == Shape::Markdown && self.md.as_ref().is_none_or(|(r, _)| *r != rev) {
            self.md = Some((rev, Rc::new(markdown::parse(self.buf.text()))));
            self.md_selection = md_view::Selection::default();
        }
        if self.mode == Mode::Preview && self.state == State::Text {
            self.ensure_diagrams(rev, cx);
        }
        if self.find_open && self.found_rev != rev {
            self.recompute();
        }
    }

    fn ensure_diagrams(&mut self, rev: u64, cx: &mut Context<Editor>) {
        if self.diagrams.as_ref().is_some_and(|batch| batch.rev == rev) {
            return;
        }
        let blocks = self.md.as_ref().map_or(&[][..], |(_, blocks)| blocks.as_slice());
        let source = (self.shape == Shape::Mermaid).then(|| self.buf.text());
        let inputs = mermaid::inputs(blocks, source);
        let mut batch = mermaid::Batch::new(rev);
        if inputs.is_empty() {
            batch.result = Some(Default::default());
            self.diagrams = Some(batch);
            return;
        }
        let lid = self.lid;
        let cancel = batch.cancel.clone();
        self.diagrams = Some(batch);
        let work = cx.background_executor().spawn(async move { mermaid::draw(inputs, &cancel) });
        cx.spawn(async move |editor, cx| {
            let result = work.await;
            editor.update(cx, |ed, cx| {
                if ed.lid == lid && let Some(batch) = ed.diagrams.as_mut().filter(|batch| batch.rev == rev) {
                    batch.result = Some(result);
                    cx.notify();
                }
            }).ok();
        }).detach();
    }

    fn offset_at(&self, p: Point<Pixels>) -> usize {
        let Some(l) = &self.layout else { return self.buf.sel.head };
        let row = (f32::from(p.y - l.bounds.top() + self.scroll.y) / f32::from(l.line_h)).floor();
        if row < 0.0 {
            return 0;
        }
        let row = row as usize;
        if row >= self.buf.line_count() {
            return self.buf.len();
        }
        let x = (p.x - l.text_left).max(px(0.0));
        if let Some(ll) = l.lines.iter().find(|ll| ll.line == row) {
            let d = ll.shaped.closest_index_for_x(x);
            return self.buf.line_start(row) + area::to_src(&ll.map, d).min(ll.shown);
        }
        let col = (f32::from(x) / f32::from(l.cw)).round().max(0.0) as usize;
        self.buf.offset_at(row, col)
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Editor>) {
        window.focus(&self.focus);
        if let Some(bar) = self.layout.as_ref().and_then(|layout| layout.scrollbar)
            && bar.track.contains(&e.position)
        {
            let y = e.position.y;
            let grab = if y >= bar.thumb.top() && y <= bar.thumb.bottom() {
                y - bar.thumb.top()
            } else {
                bar.thumb.size.height / 2.0
            };
            self.dragging = false;
            self.scroll_grab = Some(grab);
            self.scroll.y = bar.offset(y, grab);
            self.autoscroll = false;
            cx.notify();
            return;
        }
        self.scroll_grab = None;
        let at = self.offset_at(e.position);
        match e.click_count {
            2 => self.buf.select(self.buf.word_at(at)),
            n if n >= 3 => self.buf.select(self.buf.line_range_at(at)),
            _ => self.buf.move_to(at, e.modifiers.shift),
        }
        self.dragging = true;
        self.marked = None;
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Editor>) {
        if e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(grab) = self.scroll_grab {
            self.drag_scrollbar(e.position.y, grab, cx);
        } else if self.dragging {
            let at = self.offset_at(e.position);
            self.buf.move_to(at, true);
            self.autoscroll = true;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Editor>) {
        if let Some(grab) = self.scroll_grab.take() {
            self.drag_scrollbar(event.position.y, grab, cx);
        }
        self.dragging = false;
    }

    fn drag_scrollbar(&mut self, y: Pixels, grab: Pixels, cx: &mut Context<Editor>) {
        if let Some(bar) = self.layout.as_ref().and_then(|layout| layout.scrollbar) {
            self.scroll.y = bar.offset(y, grab.min(bar.thumb.size.height));
            self.autoscroll = false;
            cx.notify();
        }
    }

    fn wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Editor>) {
        let d = e.delta.pixel_delta(px(LINE_H));
        self.scroll.y = (self.scroll.y - d.y).max(px(0.0));
        self.scroll.x = (self.scroll.x - d.x).max(px(0.0));
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Editor>) {
        if !self.buf.sel.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.selected().to_string()));
        }
    }

    fn cut(&mut self, cx: &mut Context<Editor>) {
        if self.buf.readonly || self.state != State::Text {
            return self.copy(cx);
        }
        if let Some(text) = self.buf.cut() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.after_edit(cx);
        }
    }

    fn paste(&mut self, cx: &mut Context<Editor>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.edit(cx, |b| b.paste(&text));
        }
    }

    fn unit(&self) -> String {
        self.doc.as_ref().map(|d| d.indent.unit()).unwrap_or_else(|| "  ".into())
    }

    fn range8(&self, r: &Range<usize>) -> Range<usize> {
        from_utf16(self.buf.text(), r.start)..from_utf16(self.buf.text(), r.end)
    }

    fn range16(&self, r: &Range<usize>) -> Range<usize> {
        to_utf16(self.buf.text(), r.start)..to_utf16(self.buf.text(), r.end)
    }

    fn header(&self, t: &Theme, dirty: bool, cx: &mut Context<Editor>) -> impl IntoElement {
        let has_path = self.path.as_deref().is_some_and(|p| p.starts_with('/'));
        let narrow = self.width > 0.0 && self.width < NARROW;
        let name = self.title();
        let full = self.path.clone().unwrap_or_else(|| "No file open".into());
        let folder = self.path.as_deref().map(lang::folder_of).unwrap_or("").to_string();
        let pretty = files::pretty_folder(&folder, self.home.as_deref());
        let text = self.state == State::Text;
        let can_save = text && !self.buf.readonly;
        let open_label = if self.shape == Shape::Html { "Open in browser" } else { "Open" };
        let open_tip = if self.shape == Shape::Html { "Open this page in your browser" } else { "Open in the default app" };
        div()
            .id("editor-header")
            .h(px(36.0))
            .flex_none()
            .px(px(10.0))
            .flex()
            .gap(px(6.0))
            .items_center()
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface)
            .when(dirty, |el| {
                el.child(div().id("dirty-dot").size(px(8.0)).flex_none().rounded_full().bg(t.accent).tooltip(tooltip("Unsaved changes")))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(div().id("file-name").min_w(px(0.0)).truncate().font_weight(FontWeight::SEMIBOLD).tooltip(tooltip(full)).child(name))
                    .when(has_path && !narrow, |el| {
                        el.child(
                            div()
                                .id("file-folder")
                                .flex_1()
                                .min_w(px(0.0))
                                .truncate()
                                .text_size(px(12.0))
                                .text_color(t.dim)
                                .tooltip(tooltip(format!("Folder: {folder}")))
                                .child(pretty),
                        )
                    }),
            )
            .children(self.modes().into_iter().map(|(id, label, tip, mode)| {
                btn(t, id, label, tip, self.mode == mode, false, false).on_click(cx.listener(move |ed, _, window, cx| ed.set_mode(mode, window, cx)))
            }))
            .when(text, |el| {
                let tip = if self.find_open { "Close find (⌘F)" } else { "Find in file (⌘F)" };
                el.child(btn(t, "find", "Find", tip, self.find_open, false, false).on_click(cx.listener(|ed, _, window, cx| ed.toggle_find(window, cx))))
            })
            .when(text && self.mode == Mode::Preview && self.shape == Shape::Markdown, |el| {
                el.child(btn(t, "copy-preview", "Copy all", "Copy the Markdown preview as plain text", false, false, false)
                    .on_click(cx.listener(|ed, _, _, cx| {
                        if let Some((_, blocks)) = &ed.md {
                            cx.write_to_clipboard(ClipboardItem::new_string(md_view::plain_text(blocks)));
                            ed.notice = Some(("Preview text copied".into(), false));
                            cx.notify();
                        }
                    })))
            })
            .when(can_save, |el| {
                let label = if self.saving { "Saving…" } else { "Save" };
                let tip = if dirty { "Save to disk (⌘S)" } else { "No unsaved changes" };
                el.child(btn(t, "save", label, tip, false, dirty, !dirty || self.saving).on_click(cx.listener(|ed, _, _, cx| ed.save(false, cx))))
            })
            .when(has_path, |el| {
                el.child(btn(t, "open-external", open_label, open_tip, false, false, false).on_click(cx.listener(|ed, _, _, cx| ed.open_outside(cx))))
                    .child(btn(t, "reveal", "Reveal", "Reveal in Finder", false, false, false).on_click(cx.listener(|ed, _, _, cx| ed.reveal(cx))))
            })
            .child(btn(t, "close", "×", "Close this file", false, false, false).on_click(cx.listener(|ed, _, _, cx| {
                ed.request_close(cx);
            })))
    }

    fn bar(&self, t: &Theme, id: &'static str, text: impl Into<SharedString>, actions: Vec<AnyElement>) -> impl IntoElement {
        div()
            .id(id)
            .flex_none()
            .min_h(px(34.0))
            .px(px(10.0))
            .py(px(5.0))
            .flex()
            .gap(px(8.0))
            .items_center()
            .border_b_1()
            .border_color(t.border)
            .bg(t.raised)
            .child(div().size(px(8.0)).flex_none().rounded_full().bg(t.blocked))
            .child(div().flex_1().min_w(px(0.0)).text_size(px(12.0)).child(text.into()))
            .children(actions)
    }

    fn conflict_bar(&self, t: &Theme, cx: &mut Context<Editor>) -> impl IntoElement {
        let c = self.doc.as_ref().and_then(|d| d.conflict.clone());
        let deleted = c.as_ref().is_some_and(|c| c.disk.is_none());
        let from_save = c.as_ref().is_some_and(|c| c.from_save);
        let first = if deleted {
            btn(t, "conflict-close", "Close", "Close this file and drop your edits", false, false, false)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Close)))
                .into_any_element()
        } else {
            btn(t, "conflict-reload", "Reload", "Load the version on disk and drop your edits", false, false, false)
                .on_click(cx.listener(|ed, _, _, cx| ed.reload_from_disk(cx)))
                .into_any_element()
        };
        let tip = if deleted {
            "Keep your text open. Save makes the file again."
        } else if from_save {
            "Write your version over the one on disk"
        } else {
            "Keep your edits. Save will replace the version on disk."
        };
        let keep = btn(t, "conflict-keep", "Keep mine", tip, false, true, false).on_click(cx.listener(|ed, _, _, cx| ed.keep_mine(cx))).into_any_element();
        self.bar(t, "conflict", if deleted { "Deleted on disk." } else { "Changed on disk." }, vec![first, keep])
    }

    fn close_bar(&self, t: &Theme, cx: &mut Context<Editor>) -> impl IntoElement {
        let save = btn(t, "close-save", "Save and close", "Save to disk, then close", false, true, false)
            .on_click(cx.listener(|ed, _, _, cx| {
                ed.closing = false;
                ed.save(true, cx);
            }))
            .into_any_element();
        let discard = btn(t, "close-discard", "Discard", "Close without saving", false, false, false)
            .on_click(cx.listener(|ed, _, _, cx| {
                ed.closing = false;
                ed.close_when_saved = false;
                cx.emit(EditorEvent::Close);
            }))
            .into_any_element();
        let cancel = btn(t, "close-cancel", "Cancel", "Keep editing", false, false, false)
            .on_click(cx.listener(|ed, _, _, cx| {
                ed.closing = false;
                ed.close_when_saved = false;
                cx.notify();
            }))
            .into_any_element();
        self.bar(t, "closing", "Unsaved changes.", vec![save, discard, cancel])
    }

    fn link_bar(&self, t: &Theme, target: &str, cx: &mut Context<Editor>) -> impl IntoElement {
        let save = btn(t, "link-save", "Save there", "Write your text to the file the link points to", false, true, false)
            .on_click(cx.listener(|ed, _, _, cx| ed.save_through_link(cx)))
            .into_any_element();
        let cancel = btn(t, "link-cancel", "Cancel", "Do not save", false, false, false)
            .on_click(cx.listener(|ed, _, _, cx| {
                ed.link_ask = None;
                cx.notify();
            }))
            .into_any_element();
        let text = format!("This file is a link to {target}, outside its folder. Save writes there.");
        self.bar(t, "link", text, vec![save, cancel])
    }

    fn find_bar(&self, t: &Theme, cx: &mut Context<Editor>) -> impl IntoElement {
        let n = self.found.len();
        let shown: Vec<(usize, Match)> = self.found.iter().copied().enumerate().take(CHIPS).collect();
        let current = self.found_at;
        div()
            .id("find-bar")
            .h(px(38.0))
            .flex_none()
            .px(px(10.0))
            .flex()
            .gap(px(6.0))
            .items_center()
            .border_b_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(div().id("find-box").w(px(240.0)).flex_none().child(self.find_input.clone()))
            .child(btn(t, "find-prev", "‹", "Previous match (⇧⌘G)", false, false, n == 0).on_click(cx.listener(|ed, _, _, cx| ed.find_step(-1, cx))))
            .child(btn(t, "find-next", "›", "Next match (Enter, ⌘G)", false, false, n == 0).on_click(cx.listener(|ed, _, _, cx| ed.find_step(1, cx))))
            .child(div().id("find-count").flex_none().min_w(px(86.0)).text_size(px(11.0)).text_color(t.dim).child(self.find_label()))
            .child(
                div()
                    .id("find-lines")
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(20.0))
                    .flex()
                    .gap(px(4.0))
                    .items_center()
                    .overflow_x_hidden()
                    .text_size(px(11.0))
                    .children(shown.into_iter().map(|(i, m)| {
                        let on = current == Some(i);
                        div()
                            .id(("find-line", i))
                            .flex_none()
                            .px(px(4.0))
                            .h(px(18.0))
                            .flex()
                            .items_center()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_color(if on { t.accent } else { t.dim })
                            .bg(if on { t.selected } else { t.raised })
                            .tooltip(tooltip(format!("Go to the match on line {}", m.line)))
                            .on_click(cx.listener(move |ed, _, _, cx| ed.find_go(i, cx)))
                            .child(m.line.to_string())
                    })),
            )
            .when(n > CHIPS, |el| el.child(div().flex_none().text_size(px(11.0)).text_color(t.dim).child(format!("+{} more", n - CHIPS))))
            .child(btn(t, "find-close", "×", "Close find (Esc)", false, false, false).on_click(cx.listener(|ed, _, window, cx| ed.close_find(window, cx))))
    }

    fn message(&self, t: &Theme, title: &str, detail: Option<String>, actions: Vec<AnyElement>) -> AnyElement {
        div()
            .id("message")
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .p(px(24.0))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.0)).child(title.to_string()))
            .when_some(detail, |el, d| el.child(div().max_w(px(520.0)).text_size(px(12.0)).text_color(t.dim).text_center().child(d)))
            .when(!actions.is_empty(), |el| el.child(div().flex().gap(px(8.0)).pt(px(4.0)).children(actions)))
            .into_any_element()
    }

    fn body(&mut self, t: &Theme, cx: &mut Context<Editor>) -> AnyElement {
        let path = self.path.clone().unwrap_or_default();
        let open = |t: &Theme, cx: &mut Context<Editor>| {
            btn(t, "body-open", "Open in default app", "Open with the app your Mac uses for this file", false, true, false)
                .on_click(cx.listener(|ed, _, _, cx| ed.open_outside(cx)))
                .into_any_element()
        };
        let reveal = |t: &Theme, cx: &mut Context<Editor>| {
            btn(t, "body-reveal", "Reveal in Finder", "Show this item in its folder", false, false, false)
                .on_click(cx.listener(|ed, _, _, cx| ed.reveal(cx)))
                .into_any_element()
        };
        let retry = |t: &Theme, cx: &mut Context<Editor>| {
            btn(t, "body-retry", "Try again", "Read the file again", false, false, false)
                .on_click(cx.listener(|ed, _, _, cx| ed.refresh(cx)))
                .into_any_element()
        };
        match self.state.clone() {
            State::Empty => self.message(t, "No file open", Some("Drop a file here to open it.".into()), vec![]),
            State::Loading => self.message(t, "Loading…", None, vec![]),
            State::Missing => self.message(t, "No file at this path", Some(path), vec![retry(t, cx)]),
            State::Folder => self.message(t, "This is a folder, not a file", Some(path), vec![reveal(t, cx)]),
            State::Binary => self.message(t, "Binary file", Some("This file is not text, so it cannot be shown here.".into()), vec![open(t, cx), reveal(t, cx)]),
            State::Large(e) => self.message(t, "File too large", Some(e), vec![open(t, cx), reveal(t, cx)]),
            State::Error(e) => {
                let actions = if path.starts_with('/') { vec![open(t, cx), retry(t, cx)] } else { vec![] };
                self.message(t, "Cannot show this file", Some(e), actions)
            }
            State::Image => self.image_view(t),
            State::Text => match self.mode {
                Mode::Preview if self.shape == Shape::Markdown => {
                    let blocks = self.md.as_ref().map(|(_, b)| b.clone()).unwrap_or_default();
                    let folder = lang::folder_of(&path).to_string();
                    let previews = self.diagrams.as_ref().and_then(|batch| batch.result.as_ref());
                    let children = md_view::render_with_diagrams(&blocks, &folder, t, previews, &mut self.md_selection, cx);
                    md_view::scroller("md-scroll", t, children, &self.md_scroll)
                        .id("markdown-selection")
                        .key_context(PREVIEW_CONTEXT)
                        .track_focus(&self.focus)
                        .cursor(CursorStyle::IBeam)
                        .on_mouse_down(MouseButton::Left, cx.listener(|ed, event: &MouseDownEvent, window, cx| {
                            window.focus(&ed.focus);
                            ed.md_selection.begin(event.position, event.modifiers.shift);
                            cx.notify();
                        }))
                        .on_mouse_move(cx.listener(|ed, event: &MouseMoveEvent, _, cx| {
                            if event.pressed_button == Some(MouseButton::Left) && ed.md_selection.extend(event.position) {
                                cx.notify();
                            }
                        }))
                        .on_mouse_up(MouseButton::Left, cx.listener(Self::preview_mouse_up))
                        .on_mouse_up_out(MouseButton::Left, cx.listener(Self::preview_mouse_up))
                        .on_action(cx.listener(|ed, _: &Copy, _, cx| {
                            if let Some(text) = ed.md_selection.copied() {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }))
                        .on_action(cx.listener(|ed, _: &SelectAll, _, cx| {
                            ed.md_selection.select_all();
                            cx.notify();
                        }))
                        .into_any_element()
                }
                Mode::Preview if self.shape == Shape::Mermaid => {
                    let previews = self.diagrams.as_ref().and_then(|batch| batch.result.as_ref());
                    let diagram = md_view::diagram(t, self.buf.text(), 0, previews);
                    md_view::scroller("mermaid-scroll", t, vec![diagram.into_any_element()], &self.md_scroll).into_any_element()
                }
                Mode::Image if self.pic.is_some() => self.image_view(t),
                _ => self.text_view(cx),
            },
        }
    }

    fn preview_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Editor>) {
        if self.md_selection.finish(event.position) {
            cx.notify();
        }
    }

    fn image_view(&self, t: &Theme) -> AnyElement {
        let Some(pic) = &self.pic else { return div().flex_1().into_any_element() };
        div()
            .id("image-box")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .p(px(16.0))
            .bg(t.canvas)
            .flex()
            .items_center()
            .justify_center()
            .child(match &pic.source {
                Source::Bytes(data) => img(data.clone()),
                Source::File(path) => img(path.clone()),
            }
            .size_full()
            .when_some(pic.dims, |el, (w, h)| el.max_w(px(w as f32)).max_h(px(h as f32)))
            .object_fit(ObjectFit::ScaleDown))
            .into_any_element()
    }

    fn text_view(&self, cx: &mut Context<Editor>) -> AnyElement {
        div()
            .id("text")
            .key_context(TEXT_CONTEXT)
            .track_focus(&self.focus)
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|e, _: &MoveLeft, _, cx| e.motion(cx, |b| b.left(false))))
            .on_action(cx.listener(|e, _: &MoveRight, _, cx| e.motion(cx, |b| b.right(false))))
            .on_action(cx.listener(|e, _: &MoveUp, _, cx| e.motion(cx, |b| b.vertical(-1, false))))
            .on_action(cx.listener(|e, _: &MoveDown, _, cx| e.motion(cx, |b| b.vertical(1, false))))
            .on_action(cx.listener(|e, _: &SelectLeft, _, cx| e.motion(cx, |b| b.left(true))))
            .on_action(cx.listener(|e, _: &SelectRight, _, cx| e.motion(cx, |b| b.right(true))))
            .on_action(cx.listener(|e, _: &SelectUp, _, cx| e.motion(cx, |b| b.vertical(-1, true))))
            .on_action(cx.listener(|e, _: &SelectDown, _, cx| e.motion(cx, |b| b.vertical(1, true))))
            .on_action(cx.listener(|e, _: &WordLeft, _, cx| e.motion(cx, |b| b.word_left(false))))
            .on_action(cx.listener(|e, _: &WordRight, _, cx| e.motion(cx, |b| b.word_right(false))))
            .on_action(cx.listener(|e, _: &SelectWordLeft, _, cx| e.motion(cx, |b| b.word_left(true))))
            .on_action(cx.listener(|e, _: &SelectWordRight, _, cx| e.motion(cx, |b| b.word_right(true))))
            .on_action(cx.listener(|e, _: &LineStart, _, cx| e.motion(cx, |b| b.home(false))))
            .on_action(cx.listener(|e, _: &LineEnd, _, cx| e.motion(cx, |b| b.end(false))))
            .on_action(cx.listener(|e, _: &SelectLineStart, _, cx| e.motion(cx, |b| b.home(true))))
            .on_action(cx.listener(|e, _: &SelectLineEnd, _, cx| e.motion(cx, |b| b.end(true))))
            .on_action(cx.listener(|e, _: &DocStart, _, cx| e.motion(cx, |b| b.doc_start(false))))
            .on_action(cx.listener(|e, _: &DocEnd, _, cx| e.motion(cx, |b| b.doc_end(false))))
            .on_action(cx.listener(|e, _: &SelectDocStart, _, cx| e.motion(cx, |b| b.doc_start(true))))
            .on_action(cx.listener(|e, _: &SelectDocEnd, _, cx| e.motion(cx, |b| b.doc_end(true))))
            .on_action(cx.listener(|e, _: &PageUp, _, cx| {
                let n = e.page as isize;
                e.motion(cx, |b| b.vertical(-n, false))
            }))
            .on_action(cx.listener(|e, _: &PageDown, _, cx| {
                let n = e.page as isize;
                e.motion(cx, |b| b.vertical(n, false))
            }))
            .on_action(cx.listener(|e, _: &SelectPageUp, _, cx| {
                let n = e.page as isize;
                e.motion(cx, |b| b.vertical(-n, true))
            }))
            .on_action(cx.listener(|e, _: &SelectPageDown, _, cx| {
                let n = e.page as isize;
                e.motion(cx, |b| b.vertical(n, true))
            }))
            .on_action(cx.listener(|e, _: &SelectAll, _, cx| e.motion(cx, |b| b.select_all())))
            .on_action(cx.listener(|e, _: &Backspace, _, cx| e.edit(cx, |b| b.backspace())))
            .on_action(cx.listener(|e, _: &Delete, _, cx| e.edit(cx, |b| b.delete_forward())))
            .on_action(cx.listener(|e, _: &DeleteWordBack, _, cx| e.edit(cx, |b| b.delete_word_back())))
            .on_action(cx.listener(|e, _: &DeleteLineBack, _, cx| e.edit(cx, |b| b.delete_line_back())))
            .on_action(cx.listener(|e, _: &Newline, _, cx| e.edit(cx, |b| b.newline())))
            .on_action(cx.listener(|e, _: &Tab, _, cx| {
                let unit = e.unit();
                e.edit(cx, |b| b.tab(&unit))
            }))
            .on_action(cx.listener(|e, _: &Outdent, _, cx| {
                let unit = e.unit();
                e.edit(cx, |b| b.outdent(&unit))
            }))
            .on_action(cx.listener(|e, _: &Undo, _, cx| e.edit(cx, |b| b.undo())))
            .on_action(cx.listener(|e, _: &Redo, _, cx| e.edit(cx, |b| b.redo())))
            .on_action(cx.listener(|e, _: &Copy, _, cx| e.copy(cx)))
            .on_action(cx.listener(|e, _: &Cut, _, cx| e.cut(cx)))
            .on_action(cx.listener(|e, _: &Paste, _, cx| e.paste(cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(TextArea { editor: cx.entity() })
            .into_any_element()
    }

    fn footer(&self, t: &Theme) -> impl IntoElement {
        let mut parts: Vec<String> = Vec::new();
        if let (State::Text, Some(d)) = (&self.state, &self.doc) {
            parts.push(self.lang.name.clone());
            parts.push(d.eol.label().into());
            parts.push(d.indent.label());
            parts.push(if d.bom { "UTF-8 with BOM".into() } else { "UTF-8".into() });
            if self.mode == Mode::Edit {
                let head = self.buf.sel.head;
                parts.push(format!("Ln {}, Col {}", self.buf.line_of(head) + 1, self.buf.col_of(head) + 1));
            }
            if self.buf.readonly {
                parts.push("Read only".into());
            }
            if doc::is_missing(d) {
                parts.push("Not on disk".into());
            }
        } else if let Some(p) = &self.pic {
            parts.push(self.path.as_deref().map(lang::ext_of).unwrap_or_default().to_uppercase());
            if let Some((w, h)) = p.dims {
                parts.push(format!("{w} × {h}"));
            }
            parts.push(doc::format_bytes(p.bytes));
        }
        let (notice, error) = self.notice.clone().unwrap_or_default();
        div()
            .id("editor-footer")
            .h(px(24.0))
            .flex_none()
            .px(px(10.0))
            .flex()
            .gap(px(10.0))
            .items_center()
            .border_t_1()
            .border_color(t.border)
            .bg(t.surface)
            .text_size(px(11.0))
            .text_color(t.dim)
            .child(div().flex_none().child(parts.join("  ·  ")))
            .child(
                div()
                    .id("notice")
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .justify_end()
                    .text_color(if error { t.blocked } else { t.muted })
                    .when(!notice.is_empty(), |element| element.tooltip(nixa_ui::popover::tooltip(notice.clone())))
                    .child(div().truncate().child(notice)),
            )
    }
}

pub fn show(ed: &mut Editor, what: &str, window: &mut Window, cx: &mut Context<Editor>) {
    if let Some(n) = what.strip_prefix("line:") {
        if let Ok(line) = n.trim().parse::<usize>() {
            ed.go_to_line(line, cx);
        }
        return;
    }
    if let Some(q) = what.strip_prefix("find:") {
        ed.open_find(window, cx);
        let q = q.to_string();
        ed.find_input.update(cx, |i, cx| i.set_value(q.clone(), cx));
        ed.find_changed(q, cx);
        return;
    }
    match what {
        #[cfg(test)]
        "close" => {
            ed.buf.doc_end(false);
            ed.edit(cx, |b| b.insert("\nOne more line, not saved yet.\n"));
            ed.request_close(cx);
        }
        #[cfg(test)]
        "conflict" => {
            ed.edit(cx, |b| b.insert("My edit. "));
            if let Some(d) = ed.doc.as_mut() {
                d.conflict = Some(doc::Conflict { disk: Some("Someone else changed this file.\n".into()), from_save: false });
            }
            cx.notify();
        }
        #[cfg(test)]
        "save" => {
            ed.edit(cx, |b| b.insert("My edit. "));
            ed.save(false, cx);
        }
        "source" => ed.set_mode(Mode::Edit, window, cx),
        "preview" => ed.set_mode(preview_mode(ed.shape), window, cx),
        _ => {}
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        self.ensure_caches(cx);
        let dirty = self.is_dirty();
        let me = cx.entity();
        let show_find = self.find_open && self.state == State::Text;
        let conflict = self.has_conflict();
        let closing = self.closing && dirty;
        let header = self.header(&t, dirty, cx).into_any_element();
        let find = show_find.then(|| self.find_bar(&t, cx).into_any_element());
        let conflict = conflict.then(|| self.conflict_bar(&t, cx).into_any_element());
        let closing = closing.then(|| self.close_bar(&t, cx).into_any_element());
        let link = self.link_ask.clone().map(|(p, _)| self.link_bar(&t, &p, cx).into_any_element());
        let body = self.body(&t, cx);
        let footer = self.footer(&t).into_any_element();
        div()
            .id("editor")
            .key_context(ROOT_CONTEXT)
            .track_focus(&self.root_focus)
            .relative()
            .size_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(13.0))
            .on_action(cx.listener(|e, _: &Save, _, cx| e.save(false, cx)))
            .on_action(cx.listener(|e, _: &Find, window, cx| e.toggle_find(window, cx)))
            .on_action(cx.listener(|e, _: &FindNext, _, cx| e.find_step(1, cx)))
            .on_action(cx.listener(|e, _: &FindPrev, _, cx| e.find_step(-1, cx)))
            .on_action(cx.listener(|e, _: &Escape, window, cx| e.escape(window, cx)))
            .on_drop(cx.listener(|e, paths: &ExternalPaths, _, cx| {
                if let Some(p) = paths.paths().first() {
                    e.open(&p.to_string_lossy(), cx);
                }
            }))
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, cx| {
                        let w = f32::from(bounds.size.width);
                        me.update(cx, |ed, cx| {
                            let was = ed.width > 0.0 && ed.width < NARROW;
                            ed.width = w;
                            if was != (w < NARROW) {
                                cx.notify();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(header)
            .children(find)
            .children(conflict)
            .children(closing)
            .children(link)
            .child(body)
            .child(footer)
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let r = self.range8(&range);
        actual.replace(self.range16(&r));
        Some(self.buf.text()[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range16(&self.buf.sel.range()), reversed: self.buf.sel.head < self.buf.sel.anchor })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range.map(|r| self.range8(&r)).or(self.marked.clone()).unwrap_or(self.buf.sel.range());
        self.edit(cx, |b| b.replace(range, text));
        self.marked = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        new_sel: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state != State::Text || self.buf.readonly {
            return;
        }
        let range = range.map(|r| self.range8(&r)).or(self.marked.clone()).unwrap_or(self.buf.sel.range());
        let start = range.start;
        self.buf.replace(range, text);
        self.marked = (!text.is_empty()).then(|| start..start + text.len());
        if let Some(s) = new_sel {
            let inner = from_utf16(text, s.start)..from_utf16(text, s.end);
            self.buf.select(start + inner.start..start + inner.end);
        }
        self.after_edit(cx);
    }

    fn bounds_for_range(&mut self, range: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let l = self.layout.as_ref()?;
        let r = self.range8(&range);
        let line = self.buf.line_of(r.start);
        let ll = l.lines.iter().find(|ll| ll.line == line)?;
        let ls = self.buf.line_start(line);
        let x = |off: usize| l.text_left + ll.shaped.x_for_index(ll.map[(off - ls).min(ll.shown)]);
        let end = r.end.min(self.buf.line_end(line));
        Some(Bounds::from_corners(point(x(r.start), ll.y), point(x(end), ll.y + l.line_h)))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let at = self.offset_at(p);
        Some(to_utf16(self.buf.text(), at))
    }
}
