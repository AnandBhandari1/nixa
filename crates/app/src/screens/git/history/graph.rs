use super::log::{self, CAP, Commit, FILE_CAP, LogQuery, PAGE, RefKind, SEARCH_CAP};
use super::widgets::{hint, id, selector};
use super::words::{now_secs, short_ago_at};
use super::{commit_menu, repo};
use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, Hsla, MouseButton, PathBuilder, Pixels,
    ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle, Window, canvas, div,
    point, px, uniform_list,
};
use nixa_ui::popover::tooltip;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Icon, Theme, theme};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const ROW_H: f32 = 24.0;
pub const LANE_PITCH: f32 = 13.0;
pub const REFRESH: Duration = Duration::from_secs(8);
pub const LOAD_MORE: &str = "load-more";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub only: bool,
}

pub struct Loaded {
    pub commits: Vec<Commit>,
    pub more: bool,
    pub upstream: Option<String>,
    pub unborn: bool,
}

pub struct GraphView {
    pub repo: Option<PathBuf>,
    pub commits: Vec<Commit>,
    pub lanes: usize,
    pub all: bool,
    pub search: Option<Search>,
    pub file: Option<String>,
    pub filter: String,
    pub more: bool,
    pub busy: bool,
    pub note: Option<String>,
    pub selected: Option<String>,
    pub want: Option<String>,
    pub upstream: Option<String>,
    pub unborn: bool,
    pub loaded_at: Option<Instant>,
    pub input: Entity<TextInput>,
    token: u64,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _sub: Subscription,
    _poll: Task<()>,
}

fn x_of(lane: usize) -> f32 {
    8.0 + lane as f32 * LANE_PITCH
}

pub fn rail_width(lanes: usize) -> f32 {
    (16.0 + (lanes.max(1) as f32 - 1.0) * LANE_PITCH).max(24.0)
}

pub fn lane_colors(t: &Theme) -> [Hsla; 6] {
    let a = t.terminal.ansi;
    [a[4], a[5], a[2], a[3], a[6], a[9]]
}

fn ref_color(t: &Theme, r: &log::Ref) -> Hsla {
    let a = t.terminal.ansi;
    if r.head {
        return a[6];
    }
    match r.kind {
        RefKind::Local => a[5],
        RefKind::Remote => a[4],
        RefKind::Tag => a[3],
        RefKind::Head => a[6],
    }
}

fn ref_tip(r: &log::Ref) -> String {
    let what = if r.head {
        "Current branch "
    } else {
        match r.kind {
            RefKind::Remote => "Remote branch ",
            RefKind::Tag => "Tag ",
            RefKind::Head => "",
            RefKind::Local => "Branch ",
        }
    };
    format!("{what}{}", r.name)
}

fn paint_rail(bounds: Bounds<Pixels>, row: &log::Row, colors: &[Hsla; 6], window: &mut Window) {
    let o = bounds.origin;
    let at = |x: f32, y: f32| point(o.x + px(x), o.y + px(y * ROW_H));
    for seg in &row.segments {
        let mut b = PathBuilder::stroke(px(1.5));
        let (ax, bx) = (x_of(seg.from), x_of(seg.to));
        b.move_to(at(ax, seg.from_y));
        if seg.from == seg.to {
            b.line_to(at(bx, seg.to_y));
        } else {
            let mid = (seg.from_y + seg.to_y) / 2.0;
            b.cubic_bezier_to(at(bx, seg.to_y), at(ax, mid), at(bx, mid));
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, colors[seg.color % colors.len()]);
        }
    }
}

impl Focusable for GraphView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GraphView {
    pub fn new(repo: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new(cx, "Filter by message, author or id").with_icon(Icon::Search));
        let sub = cx.subscribe(&input, |view: &mut GraphView, _, event: &InputEvent, cx| match event {
            InputEvent::Changed(text) => {
                view.filter = text.clone();
                cx.notify();
            }
            InputEvent::Submit(text) => {
                let text = text.clone();
                view.search_from_box(&text, cx);
            }
            InputEvent::Cancel => {}
        });
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                let alive = this.update(cx, |view, cx| {
                    let old = view.loaded_at.is_none_or(|at| at.elapsed() >= REFRESH);
                    if old && !view.busy && view.search.is_none() {
                        view.load(false, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut view = GraphView {
            repo,
            commits: Vec::new(),
            lanes: 1,
            all: super::graph_all(cx),
            search: None,
            file: None,
            filter: String::new(),
            more: false,
            busy: false,
            note: None,
            selected: None,
            want: None,
            upstream: None,
            unborn: false,
            loaded_at: None,
            input,
            token: 0,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _sub: sub,
            _poll: poll,
        };
        view.load(true, cx);
        view
    }

    pub fn set_repo(&mut self, repo: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.repo == repo {
            return;
        }
        self.next_token();
        self.repo = repo;
        self.search = None;
        self.file = None;
        self.busy = false;
        self.note = None;
        self.want = None;
        self.upstream = None;
        self.unborn = false;
        self.reset();
        self.load(true, cx);
        cx.notify();
    }

    fn reset(&mut self) {
        self.commits.clear();
        self.selected = None;
        self.more = false;
        self.loaded_at = None;
        self.lanes = 1;
    }

    pub fn visible(&self) -> Vec<&Commit> {
        log::visible(&self.commits, &self.filter, self.search.is_some())
    }

    fn put(&mut self, mut commits: Vec<Commit>, linear: bool) {
        self.lanes = log::set_rows(&mut commits, linear);
        self.commits = commits;
        if self.selected.as_ref().is_some_and(|s| !self.commits.iter().any(|c| &c.sha == s)) {
            self.selected = None;
        }
    }

    fn next_token(&mut self) -> u64 {
        self.token += 1;
        self.token
    }

    pub fn load(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(root) = self.repo.clone() else {
            return;
        };
        if self.busy && !force {
            return;
        }
        if let Some(s) = self.search.clone() {
            if force {
                self.run_search(&s.query, s.only, true, None, cx);
            }
            return;
        }
        let token = self.next_token();
        self.busy = true;
        let file = self.file.clone();
        let all = self.all;
        let have = self.commits.len();
        let job = cx.background_executor().spawn(async move {
            let unborn = repo::head_sha(&root).is_none();
            let upstream = repo::upstream_of(&root);
            if unborn {
                return Ok(Loaded { commits: Vec::new(), more: false, upstream, unborn });
            }
            let (cap, commits) = match &file {
                Some(path) => {
                    let limit = FILE_CAP.min(PAGE.max(have));
                    (FILE_CAP, log::file_history(&root, path, limit).map(|c| (limit, c)))
                }
                None => {
                    let limit = CAP.min(PAGE.max(have));
                    let q = LogQuery { limit, all, upstream: upstream.clone(), ..LogQuery::default() };
                    (CAP, log::read_log(&root, &log::log_args(&q)).map(|c| (limit, c)))
                }
            };
            let (limit, commits) = commits?;
            let more = commits.len() >= limit && limit < cap;
            Ok::<Loaded, String>(Loaded { commits, more, upstream, unborn })
        });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                if view.token != token {
                    return;
                }
                view.busy = false;
                match out {
                    Ok(l) => {
                        view.upstream = l.upstream;
                        view.unborn = l.unborn;
                        view.more = l.more;
                        let linear = view.file.is_some();
                        view.put(l.commits, linear);
                        view.loaded_at = Some(Instant::now());
                        view.note = None;
                        view.after_load(cx);
                    }
                    Err(e) => view.note = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo.clone() else {
            return;
        };
        if self.busy || !self.more || self.search.is_some() {
            return;
        }
        let token = self.next_token();
        self.busy = true;
        let skip = self.commits.len();
        let file = self.file.clone();
        let all = self.all;
        let upstream = self.upstream.clone();
        let have = self.commits.clone();
        let job = cx.background_executor().spawn(async move {
            match &file {
                Some(path) => {
                    let limit = FILE_CAP.min(skip + PAGE);
                    let list = log::file_history(&root, path, limit)?;
                    let more = list.len() >= limit && limit < FILE_CAP;
                    Ok::<(Vec<Commit>, bool), String>((list, more))
                }
                None => {
                    let q = LogQuery { limit: PAGE, skip, all, upstream, ..LogQuery::default() };
                    let page = log::read_log(&root, &log::log_args(&q))?;
                    let full = page.len() >= PAGE;
                    let seen: std::collections::HashSet<String> = have.iter().map(|c| c.sha.clone()).collect();
                    let mut list = have;
                    list.extend(page.into_iter().filter(|c| !seen.contains(&c.sha)));
                    let more = full && list.len() < CAP;
                    Ok((list, more))
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                if view.token != token {
                    return;
                }
                view.busy = false;
                match out {
                    Ok((list, more)) => {
                        let linear = view.file.is_some();
                        view.put(list, linear);
                        view.more = more;
                        view.loaded_at = Some(Instant::now());
                    }
                    Err(e) => view.note = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn search_from_box(&mut self, text: &str, cx: &mut Context<Self>) {
        match log::search_plan(text) {
            Some(plan) => self.run_search(&plan.query, false, false, None, cx),
            None => {
                if self.search.is_some() {
                    self.clear_search(cx);
                }
            }
        }
    }

    pub fn run_search(&mut self, query: &str, only: bool, keep: bool, select: Option<String>, cx: &mut Context<Self>) {
        let Some(root) = self.repo.clone() else {
            return;
        };
        let Some(plan) = log::search_plan(query) else {
            return;
        };
        if !keep {
            self.search = Some(Search { query: plan.query.clone(), only });
            self.commits.clear();
            self.selected = None;
        }
        self.note = None;
        self.more = false;
        let token = self.next_token();
        self.busy = true;
        let path = self.file.clone();
        let all = self.all;
        let upstream = self.upstream.clone();
        let job = cx.background_executor().spawn(async move {
            let by_id = plan.id.as_deref().map(|id| log::find_by_id(&root, id)).unwrap_or_default();
            let lists = if only {
                Vec::new()
            } else {
                let base = LogQuery { limit: PAGE, path, all, upstream, ..LogQuery::default() };
                let grep = LogQuery { grep: Some(plan.query.clone()), ..base.clone() };
                let author = LogQuery { author: Some(plan.query.clone()), ..base };
                vec![log::read_log(&root, &log::log_args(&grep))?, log::read_log(&root, &log::log_args(&author))?]
            };
            Ok::<Vec<Commit>, String>(log::merge_search(by_id, lists, SEARCH_CAP))
        });
        cx.spawn(async move |this, cx| {
            let out = job.await;
            this.update(cx, |view, cx| {
                if view.token != token || view.search.is_none() {
                    return;
                }
                view.busy = false;
                match out {
                    Ok(list) => {
                        view.put(list, true);
                        view.loaded_at = Some(Instant::now());
                        if let Some(want) = select {
                            match view.commits.iter().position(|c| c.sha.starts_with(&want)) {
                                Some(i) => view.focus_commit(i, cx),
                                None => view.note = Some(format!("No commit {want} in this repo")),
                            }
                        }
                    }
                    Err(e) => view.note = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn after_load(&mut self, cx: &mut Context<Self>) {
        let Some(want) = self.want.take() else {
            return;
        };
        match self.commits.iter().position(|c| c.sha.starts_with(&want)) {
            Some(i) => self.focus_commit(i, cx),
            None => self.run_search(&want.clone(), true, false, Some(want), cx),
        }
    }

    fn focus_commit(&mut self, index: usize, cx: &mut Context<Self>) {
        let sha = self.commits[index].sha.clone();
        if !self.filter.is_empty() && self.search.is_none() {
            self.filter.clear();
            self.input.update(cx, |i, cx| i.set_value("", cx));
        }
        self.selected = Some(sha.clone());
        if let Some(at) = self.visible().iter().position(|c| c.sha == sha) {
            self.scroll.scroll_to_item(at, ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub fn show_commit(&mut self, sha: &str, cx: &mut Context<Self>) {
        if self.repo.is_none() || !super::words::is_sha_like(sha) {
            return;
        }
        let want = sha.to_lowercase();
        if let Some(i) = self.commits.iter().position(|c| c.sha.starts_with(&want)) {
            self.focus_commit(i, cx);
            return;
        }
        if self.search.is_some() || self.file.is_some() || self.commits.is_empty() {
            self.search = None;
            self.file = None;
            self.clear_box(cx);
            self.reset();
            self.want = Some(want);
            self.load(true, cx);
        } else {
            self.run_search(&want.clone(), true, false, Some(want), cx);
        }
    }

    fn clear_box(&mut self, cx: &mut Context<Self>) {
        self.filter.clear();
        self.input.update(cx, |i, cx| i.set_value("", cx));
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search = None;
        self.clear_box(cx);
        self.reset();
        self.load(true, cx);
    }

    pub fn show_file_history(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.repo.is_none() || path.is_empty() {
            return;
        }
        self.file = Some(path.to_string());
        self.search = None;
        self.clear_box(cx);
        self.reset();
        self.load(true, cx);
    }

    pub fn clear_file(&mut self, cx: &mut Context<Self>) {
        self.file = None;
        self.reset();
        self.load(true, cx);
    }

    pub fn set_all(&mut self, all: bool, cx: &mut Context<Self>) {
        if self.all == all {
            return;
        }
        self.all = all;
        super::set_graph_all(all, cx);
        if let Some(s) = self.search.clone() {
            self.run_search(&s.query, s.only, true, None, cx);
        } else {
            self.reset();
            self.load(true, cx);
        }
        cx.notify();
    }

    pub fn pick(&mut self, sha: &str, cx: &mut Context<Self>) {
        if sha == LOAD_MORE {
            self.load_more(cx);
            return;
        }
        self.selected = if self.selected.as_deref() == Some(sha) { None } else { Some(sha.to_string()) };
        cx.notify();
    }

    pub fn open_menu(&mut self, sha: &str, cx: &mut Context<Self>) {
        let (Some(root), Some(c)) = (self.repo.clone(), self.commits.iter().find(|c| c.sha == sha)) else {
            return;
        };
        self.selected = Some(c.sha.clone());
        commit_menu::open(commit_menu::Props::of(&root, c), cx);
        cx.notify();
    }

    fn count_label(&self) -> String {
        if self.busy && self.commits.is_empty() {
            "loading…".into()
        } else if self.search.is_some() {
            format!("{} found", self.commits.len())
        } else {
            format!("{} commits", self.commits.len())
        }
    }

    fn empty_text(&self) -> String {
        if self.repo.is_none() {
            return "This folder is not a git repo.".into();
        }
        if self.busy {
            return if self.search.is_some() { "Searching…".into() } else { "Loading history…".into() };
        }
        if self.search.is_some() {
            "Nothing found".into()
        } else if !self.filter.is_empty() {
            "No commits match. Press Enter to search the whole history.".into()
        } else if self.file.is_some() {
            "No commits for this file yet".into()
        } else if self.unborn {
            "No commits yet".into()
        } else {
            "No commits".into()
        }
    }

    fn chip(t: &Theme, name: &str, label: String, color: Hsla, tip: String) -> gpui::Stateful<gpui::Div> {
        let hover = t.hover;
        let el = div()
            .id(id(name))
            .flex_none()
            .h(px(22.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(color.opacity(0.6))
            .text_size(px(11.0))
            .text_color(color)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .child(label);
        selector(el, name)
    }

    fn header(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let row = div()
            .flex_none()
            .h(px(36.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(t.border);
        let row = match &self.file {
            Some(path) => {
                let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.clone());
                row.child(
                    Self::chip(t, "history-file-filter", format!("history of {name} ✕"), t.accent, format!("Showing commits that changed {path}. Click to show all commits."))
                        .on_click(cx.listener(|v, _, _, cx| v.clear_file(cx))),
                )
            }
            None => row,
        };
        let row = match &self.search {
            Some(s) => {
                let words = if s.only {
                    format!("commit {} ✕", s.query.chars().take(10).collect::<String>())
                } else {
                    format!("search results: {} ✕", s.query)
                };
                row.child(
                    Self::chip(t, "history-search-results", words, t.accent, "Showing what the search found in the whole history. Click to go back to the list.".into())
                        .on_click(cx.listener(|v, _, _, cx| v.clear_search(cx))),
                )
            }
            None => row,
        };
        let row = if self.file.is_none() {
            let (label, color, tip) = if self.all {
                ("all branches", t.cyan, "Showing every local branch and tag, and the remote copy of this branch. Click to show only this branch.")
            } else {
                ("this branch", t.muted, "Showing this branch and its remote copy. Click to show every local branch and tag.")
            };
            row.child(
                Self::chip(t, "history-scope", label.into(), color, tip.into())
                    .on_click(cx.listener(|v, _, _, cx| v.set_all(!v.all, cx))),
            )
        } else {
            row
        };
        let tip = match (&self.search, &self.file) {
            (Some(s), _) => format!("Commits found for \"{}\"", s.query),
            (None, Some(f)) => format!("Commits that changed {f}"),
            _ => format!("Commits on the current branch (newest {})", PAGE.max(self.commits.len())),
        };
        row.child(
            div()
                .id("history-search")
                .flex_1()
                .min_w(px(0.0))
                .tooltip(tooltip("Filter commits by message, author or id. Press Enter to search the whole history."))
                .child(self.input.clone()),
        )
        .child(
            div()
                .id("history-count")
                .flex_none()
                .text_size(px(11.0))
                .text_color(t.muted)
                .tooltip(tooltip(tip))
                .child(self.count_label()),
        )
    }

    fn row(&self, c: &Commit, t: &Theme, colors: &[Hsla; 6], plain: bool, now: i64, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.selected.as_deref() == Some(c.sha.as_str());
        let merge = c.parents.len() > 1;
        let width = rail_width(self.lanes);
        let head = c.refs.iter().any(|r| r.head);
        let lane_color = colors[c.row.lane % colors.len()];
        let row = c.row.clone();
        let colors_copy = *colors;
        let rail = div()
            .w(px(width))
            .h(px(ROW_H))
            .flex_none()
            .relative()
            .overflow_hidden()
            .when(!plain, |el| {
                el.child(
                    canvas(|_, _, _| {}, move |bounds, _, window, _| paint_rail(bounds, &row, &colors_copy, window))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                )
            })
            .child({
                let dot = div()
                    .absolute()
                    .left(px(x_of(c.row.lane) - 4.0))
                    .top(px(ROW_H / 2.0 - 4.0))
                    .size(px(8.0))
                    .rounded(px(4.0));
                if head {
                    dot.bg(t.background).border_2().border_color(lane_color)
                } else {
                    dot.bg(lane_color).border_1().border_color(t.background)
                }
            });
        let mut chips: Vec<gpui::AnyElement> = c
            .refs
            .iter()
            .take(2)
            .enumerate()
            .map(|(i, r)| {
                let color = ref_color(t, r);
                div()
                    .id(id(format!("ref-{}-{i}", c.short)))
                    .flex_none()
                    .max_w(px(120.0))
                    .min_w(px(0.0))
                    .h(px(16.0))
                    .px(px(5.0))
                    .mr(px(4.0))
                    .flex()
                    .items_center()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(color)
                    .text_size(px(10.0))
                    .text_color(color)
                    .overflow_hidden()
                    .tooltip(tooltip(ref_tip(r)))
                    .child(if r.kind == RefKind::Tag { format!("◆ {}", r.name) } else { r.name.clone() })
                    .into_any_element()
            })
            .collect();
        if c.refs.len() > 2 {
            let rest = c.refs[2..].iter().map(|r| r.name.clone()).collect::<Vec<_>>().join(", ");
            chips.push(
                div()
                    .id(id(format!("ref-{}-more", c.short)))
                    .flex_none()
                    .mr(px(4.0))
                    .text_size(px(10.0))
                    .text_color(t.dim)
                    .tooltip(tooltip(rest))
                    .child(format!("+{}", c.refs.len() - 2))
                    .into_any_element(),
            );
        }
        let sha = c.sha.clone();
        let menu_sha = c.sha.clone();
        let tip = format!(
            "{}\n{} · {}{}\nRight-click for more.",
            c.subject,
            c.author,
            c.short,
            if merge { " · merge" } else { "" }
        );
        let hover = t.hover;
        let name = format!("commit-{}", c.short);
        let el = div()
            .id(SharedString::from(name.clone()))
            .h(px(ROW_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .cursor_pointer()
            .when(active, |el| el.bg(t.selected))
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |v, _, _, cx| v.pick(&sha, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |v, _, _, cx| v.open_menu(&menu_sha, cx)))
            .child(rail)
            .child(div().w(px(54.0)).flex_none().text_size(px(11.0)).text_color(t.accent).child(c.short.clone()))
            .when(merge, |el| el.child(div().flex_none().mr(px(4.0)).text_size(px(10.0)).text_color(t.yellow).child("⑂")))
            .children(chips)
            .child(div().flex_1().min_w(px(0.0)).overflow_hidden().text_ellipsis().whitespace_nowrap().text_size(px(12.0)).text_color(t.foreground).child(c.subject.clone()))
            .child(div().max_w(px(88.0)).min_w(px(0.0)).px(px(6.0)).overflow_hidden().whitespace_nowrap().text_size(px(10.0)).text_color(t.muted).child(c.author.clone()))
            .child(div().flex_none().pr(px(10.0)).text_size(px(10.0)).text_color(t.dim).child(short_ago_at(c.at, now)));
        selector(el, &name).into_any_element()
    }

    fn more_row(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let hover = t.hover;
        let tip = if self.file.is_some() { "Show older commits of this file".to_string() } else { format!("Show the next {PAGE} older commits") };
        let el = div()
            .id(LOAD_MORE)
            .h(px(ROW_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(|v, _, _, cx| v.load_more(cx)))
            .child(
                div()
                    .h(px(20.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.raised)
                    .text_size(px(11.0))
                    .text_color(if self.busy { t.dim } else { t.muted })
                    .child(if self.busy { "loading…" } else { "Load more" }),
            );
        selector(el, "history-load-more").into_any_element()
    }

    fn rows(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let t = theme(cx).clone();
        let colors = lane_colors(&t);
        let plain = self.search.is_none() && !self.filter.trim().is_empty();
        let now = now_secs();
        let shown: Vec<Commit> = self.visible().into_iter().cloned().collect();
        let more = self.more && self.search.is_none() && !shown.is_empty();
        let mut out = Vec::new();
        for i in range {
            if let Some(c) = shown.get(i) {
                out.push(self.row(c, &t, &colors, plain, now, cx));
            } else if more && i == shown.len() {
                out.push(self.more_row(&t, cx));
            }
        }
        out
    }
}

impl Render for GraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let shown = self.visible().len();
        let more = self.more && self.search.is_none() && shown > 0;
        let count = shown + usize::from(more);
        let header = self.header(&t, cx);
        let list = uniform_list("history-list", count, cx.processor(|this, range, _window, cx| this.rows(range, cx)))
            .track_scroll(self.scroll.clone())
            .size_full();
        div()
            .id("history-body")
            .track_focus(&self.focus)
            .size_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .when(self.repo.is_some(), |el| el.child(header))
            .when_some(self.note.clone(), |el, note| {
                el.child(div().px(px(10.0)).py(px(4.0)).text_size(px(11.0)).text_color(t.red).child(note))
            })
            .child(
                div()
                    .id("history-list-zone")
                    .flex_1()
                    .min_h(px(80.0))
                    .relative()
                    .child(list)
                    .when(count == 0, |el| {
                        el.child(div().absolute().top(px(6.0)).w_full().px(px(12.0)).child(hint(&t, self.empty_text())))
                    }),
            )
    }
}
