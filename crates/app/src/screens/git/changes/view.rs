use super::diff_view::check_box;
use super::git::{FileEntry, PullMode, base_name, plural};
use super::words;
use super::{Ask, ChangesView, Popup, Tab};
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, SharedString, Window, div, px,
    uniform_list,
};
use nixa_ui::button::{ButtonKind, button, disabled};
use nixa_ui::icon_button::{IconSize, icon_button, tinted_icon_button};
use nixa_ui::popover::tooltip;
use nixa_ui::tab::{segment, segmented};
use nixa_ui::icons::spinner;
use nixa_ui::{Icon, Theme, icon, theme};

pub const FILE_ROW_H: f32 = 26.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListRow {
    Folder { name: String, count: usize, closed: bool },
    File(usize),
}

pub fn list_rows(files: &[FileEntry], closed: &std::collections::HashSet<String>) -> Vec<ListRow> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for (i, f) in files.iter().enumerate() {
        if !groups.contains_key(&f.folder) {
            order.push(f.folder.clone());
        }
        groups.entry(f.folder.clone()).or_default().push(i);
    }
    order.sort();
    let mut out = Vec::new();
    let single = order.len() == 1 && order[0].is_empty();
    for name in order {
        let items = groups.remove(&name).unwrap_or_default();
        let shut = closed.contains(&name);
        if !single {
            out.push(ListRow::Folder {
                name: name.clone(),
                count: items.len(),
                closed: shut,
            });
        }
        if !shut {
            out.extend(items.into_iter().map(ListRow::File));
        }
    }
    out
}

pub fn status_color(t: &Theme, f: &FileEntry) -> Hsla {
    if f.conflict {
        return t.red;
    }
    match f.status {
        'A' => t.green,
        'D' => t.red,
        'R' => t.blue,
        _ => t.yellow,
    }
}

pub(super) fn tiny(t: &Theme, id: impl Into<SharedString>, glyph: Icon, tip: impl Into<SharedString>, tone: Option<Hsla>) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = id.into();
    let sel = id.to_string();
    let el = match tone {
        Some(c) => tinted_icon_button(t, id, glyph, IconSize::Small, false, c),
        None => icon_button(t, id, glyph, IconSize::Small, false),
    };
    el.debug_selector(move || sel)
        .tooltip(tooltip(tip.into()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn word_button(t: &Theme, id: impl Into<SharedString>, label: impl Into<SharedString>, tip: impl Into<SharedString>, off: bool) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = id.into();
    let sel = id.to_string();
    disabled(
        button(t, id, label, ButtonKind::Secondary)
            .h(px(24.))
            .px(px(8.))
            .debug_selector(move || sel)
            .tooltip(tooltip(tip.into()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
        off,
    )
}

fn top_bar(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> AnyElement {
    let name = v
        .project
        .as_deref()
        .or(v.root.as_deref())
        .map(|p| base_name(p).to_string())
        .unwrap_or_else(|| "No repo".into());
    let many = v.repos.len() > 1;
    let other_changes = v.repos.iter()
        .filter(|repo| v.project.as_deref() != Some(repo.path.as_str()) && repo.count > 0)
        .count();
    let repo_width = if v.root.is_some() { 130. } else { 260. };
    let hover = t.hover;
    let repo = div()
        .id("changes-repo")
        .debug_selector(|| "changes-repo".into())
        .h(px(24.))
        .px(px(6.))
        .gap(px(5.))
        .min_w_0()
        .max_w(px(repo_width))
        .flex()
        .items_center()
        .rounded(px(6.))
        .when(many, |el| el.cursor_pointer().hover(move |s| s.bg(hover)))
        .tooltip(tooltip(match (&v.root, many) {
            (Some(r), true) => format!("{r}\nClick to pick another repo in this folder"),
            (Some(r), false) => r.clone(),
            (None, _) => "No git repo here".into(),
        }))
        .when(many, |el| {
            el.on_click(cx.listener(|v, e: &ClickEvent, _, cx| super::open_repo_menu(v, e.position(), cx)))
        })
        .child(icon(Icon::Folder).size(px(13.)).flex_none().text_color(t.blue))
        .child(div().flex_1().min_w_0().truncate().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(name))
        .when(other_changes > 0, |el| {
            el.child(div().flex_none().text_size(px(10.5)).text_color(t.accent).child(format!("+{other_changes}")))
        })
        .when(many, |el| el.child(icon(Icon::ChevronDown).size(px(11.)).flex_none().text_color(t.dim)));
    let branch = match (&v.header.branch, v.header.detached) {
        (_, true) => "detached".to_string(),
        (Some(b), _) => b.clone(),
        (None, _) => String::new(),
    };
    let branch_chip = (!branch.is_empty()).then(|| {
        div()
            .id("changes-branch")
            .debug_selector(|| "changes-branch".into())
            .h(px(22.))
            .px(px(6.))
            .gap(px(4.))
            .min_w_0()
            .flex()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(t.border)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip("Branches: switch, make or delete a branch"))
            .on_click(cx.listener(|v, _, _, cx| super::emit_window(v, "branches", cx)))
            .child(icon(Icon::GitBranch).size(px(12.)).text_color(t.magenta))
            .child(div().min_w_0().max_w(px(95.)).truncate().text_size(px(11.5)).text_color(t.foreground).child(branch))
    });
    let ident = v.ident.as_ref();
    let ready = ident.is_some_and(|i| i.ready());
    let who = ident
        .and_then(|i| i.local_name.clone())
        .filter(|_| ready)
        .unwrap_or_else(|| "Pick a profile".into());
    let ident_chip = v.root.is_some().then(|| {
        div()
            .id("changes-identity")
            .debug_selector(|| "changes-identity".into())
            .h(px(22.))
            .px(px(6.))
            .gap(px(4.))
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(match ident {
                Some(i) if ready => format!(
                    "Commits as {} <{}>\nClick to change it for this repo",
                    i.local_name.clone().unwrap_or_default(),
                    i.local_email.clone().unwrap_or_default()
                ),
                _ => "This repo has no name and email for commits. Click to pick one.".into(),
            }))
            .on_click(cx.listener(|v, _, _, cx| super::open_identity(v, None, cx)))
            .child(icon(if ready { Icon::CircleCheck } else { Icon::CircleAlert }).size(px(12.)).text_color(if ready { t.green } else { t.yellow }))
            .child(div().max_w(px(110.)).truncate().text_size(px(11.)).text_color(if ready { t.muted } else { t.yellow }).child(who))
    });
    div()
        .h(px(36.))
        .flex_none()
        .px(px(8.))
        .gap(px(6.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .bg(t.chrome)
        .child(repo)
        .children(branch_chip)
        .child(div().flex_1())
        .children(ident_chip)
        .child(
            tiny(t, "changes-settings", Icon::Settings, "Git settings and accounts", None)
                .on_click(cx.listener(|v, _, _, cx| super::emit_window(v, "settings", cx))),
        )
        .into_any_element()
}

fn action_bar(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> AnyElement {
    let job = v.job.as_ref().filter(|j| !j.quiet);
    let fetching = v.job.as_ref().is_some_and(|j| j.quiet);
    let none = v.root.is_none();
    let no_remote = !v.has_remote;
    let h = &v.header;
    let remote_tip = |text: String| {
        if none {
            "No git repo here".to_string()
        } else if no_remote {
            "This repo has no remote".to_string()
        } else {
            text
        }
    };
    let pull_label = if h.behind > 0 { format!("Pull ↓{}", h.behind) } else { "Pull".into() };
    let push_label = match (h.upstream.is_some(), h.ahead) {
        (false, _) if h.branch.is_some() && !no_remote => "Publish".to_string(),
        (_, n) if n > 0 => format!("Push ↑{n}"),
        _ => "Push".into(),
    };
    let mode = v.pull_mode;
    let mode_tip = match mode {
        PullMode::Rebase => "rebase",
        PullMode::Merge => "merge",
        PullMode::FfOnly => "fast-forward only",
    };
    let off = none || no_remote || job.is_some() || v.busy;
    let sync_off = off || !super::can_sync(v);
    let sync_tip = |text: String| {
        if !none && !no_remote && !super::can_sync(v) {
            "Check out a branch with a commit first".to_string()
        } else {
            remote_tip(text)
        }
    };
    let pr = words::pr_visible(v.ident.as_ref(), h.branch.as_deref(), h.detached, h.unborn);
    let mut bar = div()
        .h(px(36.))
        .flex_none()
        .px(px(8.))
        .gap(px(5.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .child(
            segmented(t)
                .child(
                    segment(t, "changes-tab-changes", "Changes", v.tab == Tab::Changes)
                        .debug_selector(|| "changes-tab-changes".into())
                        .on_click(cx.listener(|v, _, _, cx| super::set_tab(v, Tab::Changes, cx))),
                )
                .child(
                    segment(t, "changes-tab-graph", "Graph", v.tab == Tab::Graph)
                        .debug_selector(|| "changes-tab-graph".into())
                        .on_click(cx.listener(|v, _, _, cx| super::set_tab(v, Tab::Graph, cx))),
                ),
        )
        .child(div().flex_1());
    if let Some(j) = job {
        bar = bar
            .child(spinner("changes-job-spin", 12., t.dim))
            .child(div().text_size(px(11.5)).text_color(t.dim).child(if j.stopping {
                "Stopping…".to_string()
            } else {
                format!("{}…", j.kind.word())
            }))
            .child(
                word_button(t, "changes-stop", "Stop", "Stop the git work that is running now", j.stopping)
                    .on_click(cx.listener(|v, _, _, cx| super::stop_remote(v, cx))),
            );
    }
    bar.child(
        disabled(tiny(t, "changes-fetch", if fetching { Icon::Spinner } else { Icon::Refresh }, remote_tip("Fetch: get the latest from the remote, change nothing here".into()), None), off)
            .when(!off, |el| el.on_click(cx.listener(|v, _, _, cx| super::fetch(v, cx)))),
    )
    .child(
        word_button(t, "changes-pull", pull_label, sync_tip(format!("Pull with {mode_tip}")), sync_off)
            .when(!sync_off, |el| el.on_click(cx.listener(|v, _, _, cx| super::pull(v, cx)))),
    )
    .child(
        word_button(t, "changes-push", push_label, sync_tip("Push your commits to the remote".into()), sync_off)
            .when(!sync_off, |el| el.on_click(cx.listener(|v, _, _, cx| super::push(v, cx)))),
    )
    .child(
        disabled(tiny(t, "changes-push-menu", Icon::ChevronDown, remote_tip("More push, pull and fetch choices".into()), None), off)
            .when(!off, |el| el.on_click(cx.listener(|v, e: &ClickEvent, _, cx| super::open_push_menu(v, e.position(), cx)))),
    )
    .child(
        disabled(tiny(t, "changes-stash", Icon::Archive, "Stash: put changes aside for later", None), none)
            .when(!none, |el| el.on_click(cx.listener(|v, _, _, cx| super::emit_window(v, "stash", cx)))),
    )
    .when(pr, |el| {
        el.child(
            tiny(t, "changes-pr", Icon::Send, "Pull request for this branch", None)
                .on_click(cx.listener(|v, _, _, cx| super::emit_window(v, "pr", cx))),
        )
    })
    .into_any_element()
}

fn banner(t: &Theme, id: &'static str, tone: Hsla, title: impl Into<SharedString>, body: Option<String>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex_none()
        .mx(px(8.))
        .mt(px(8.))
        .px(px(10.))
        .py(px(8.))
        .gap(px(6.))
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(tone.opacity(0.5))
        .bg(tone.opacity(0.08))
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(tone).child(title.into()))
        .when_some(body, |el, b| el.child(div().text_size(px(11.5)).text_color(t.muted).child(b)))
}

fn row_of(items: Vec<AnyElement>) -> gpui::Div {
    div().flex().flex_wrap().gap(px(6.)).children(items)
}

fn banners(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if let Some(left) = &v.left {
        out.push(
            banner(
                t,
                "changes-left-stash",
                t.cyan,
                "You have changes left from before",
                Some(format!("When you switched to {}, your changes were put aside. Bring them back here?", left.to)),
            )
            .child(row_of(vec![
                word_button(t, "changes-left-back", "Bring back", "Put the changes back in this branch", v.busy)
                    .on_click(cx.listener(|v, _, _, cx| super::bring_back(v, cx)))
                    .into_any_element(),
                word_button(t, "changes-left-skip", "Not now", "Keep them in the stash list", false)
                    .on_click(cx.listener(|v, _, _, cx| super::skip_left(v, cx)))
                    .into_any_element(),
            ]))
            .into_any_element(),
        );
    }
    if let Some(st) = v.state.as_ref().filter(|s| s.op.is_some()) {
        let op = st.op.unwrap_or(super::git::Op::Merge);
        let (what, detail, n) = words::state_text(st);
        let tone = if n > 0 { t.red } else { t.yellow };
        let mut buttons = vec![
            word_button(
                t,
                "changes-op-continue",
                "Continue",
                if n > 0 { "Fix the conflicts first" } else { "Go on with the next step" },
                v.busy || n > 0,
            )
            .when(n == 0, |el| el.on_click(cx.listener(|v, _, _, cx| super::with_identity(v, super::Action::Op(super::git::Verb::Continue), cx))))
            .into_any_element(),
        ];
        if op.can_skip() {
            buttons.push(
                word_button(t, "changes-op-skip", "Skip", "Leave out this commit and go on", v.busy)
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.ask = Some(Ask::Skip);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        buttons.push(
            word_button(t, "changes-op-abort", "Abort", format!("Stop the {} and go back to how it was", op.noun()), v.busy)
                .on_click(cx.listener(|v, _, _, cx| {
                    v.ask = Some(Ask::Abort);
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(banner(t, "changes-op", tone, what, Some(detail)).child(row_of(buttons)).into_any_element());
    }
    if let Some(ask) = &v.ask {
        let (title, body, yes) = match ask {
            Ask::Discard { label, .. } => (format!("Discard {label}?"), "This cannot be undone.".to_string(), "Discard"),
            Ask::DiscardAll => (
                format!("Discard all changes in {}?", plural(v.files.len(), "file", "files")),
                "New files are deleted too. This cannot be undone.".to_string(),
                "Discard all",
            ),
            Ask::Side { path, ours } => (
                if *ours { format!("Keep your side of {path}?") } else { format!("Take the incoming side of {path}?") },
                "The other side of every conflict in this file is dropped.".to_string(),
                if *ours { "Keep current" } else { "Take incoming" },
            ),
            Ask::Abort => {
                let noun = v.state.as_ref().and_then(|s| s.op).map(|o| o.noun()).unwrap_or("merge");
                (format!("Abort the {noun}?"), "Everything goes back to how it was before it started.".to_string(), "Abort")
            }
            Ask::Skip => ("Skip this commit?".to_string(), "Its changes are left out.".to_string(), "Skip"),
        };
        out.push(
            banner(t, "changes-ask", t.red, title, Some(body))
                .child(row_of(vec![
                    disabled(
                        button(t, "changes-ask-yes", yes, ButtonKind::Danger)
                            .h(px(24.))
                            .px(px(9.))
                            .debug_selector(|| "changes-ask-yes".into()),
                        v.busy || v.identity_busy || super::local_job(v),
                    )
                    .on_click(cx.listener(|v, _, _, cx| super::answer(v, true, cx)))
                    .into_any_element(),
                    button(t, "changes-ask-no", "Cancel", ButtonKind::Secondary)
                        .h(px(24.))
                        .px(px(9.))
                        .debug_selector(|| "changes-ask-no".into())
                        .on_click(cx.listener(|v, _, _, cx| super::answer(v, false, cx)))
                        .into_any_element(),
                ]))
                .into_any_element(),
        );
    }
    if v.limited {
        out.push(
            banner(
                t,
                "changes-limited",
                t.yellow,
                "Showing the first 1000 files",
                Some("This repo has more changed files. Use the terminal for the rest.".into()),
            )
            .into_any_element(),
        );
    }
    out
}

fn files_header(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> AnyElement {
    let n = v.files.len();
    let staged = super::staged_count(v);
    let can_stage = !super::stageable_all(v).is_empty();
    let (added, removed) = v.line_totals;
    div()
        .h(px(30.))
        .flex_none()
        .px(px(10.))
        .gap(px(4.))
        .flex()
        .items_center()
        .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child("Changes"))
        .child(div().text_size(px(11.)).text_color(t.dim).child(if staged > 0 {
            format!("{n} · {staged} staged")
        } else {
            n.to_string()
        }))
        .child(div().text_size(px(10.5)).text_color(t.green).child(format!("+{added}")))
        .child(div().text_size(px(10.5)).text_color(t.red).child(format!("−{removed}")))
        .when(v.loading || v.busy, |el| el.child(spinner("changes-loading", 11., t.dim)))
        .child(div().flex_1())
        .when(can_stage, |el| {
            el.child(
                tiny(t, "changes-stage-all", Icon::Plus, "Stage all changes", None)
                    .on_click(cx.listener(|v, _, _, cx| super::stage_all(v, cx))),
            )
        })
        .when(staged > 0, |el| {
            el.child(
                tiny(t, "changes-unstage-all", Icon::Minus, "Unstage all changes", None)
                    .on_click(cx.listener(|v, _, _, cx| super::unstage_all(v, cx))),
            )
        })
        .when(n > 0, |el| {
            el.child(
                tiny(t, "changes-discard-all", Icon::Undo, "Discard all changes (asks first)", Some(t.red))
                    .on_click(cx.listener(|v, _, _, cx| super::ask_discard_all(v, cx))),
            )
        })
        .into_any_element()
}

fn folder_row(v: &ChangesView, t: &Theme, name: &str, count: usize, closed: bool, index: usize, cx: &mut Context<ChangesView>) -> AnyElement {
    let key = name.to_string();
    let hover = t.hover;
    div()
        .id(("changes-folder", index))
        .debug_selector(move || format!("changes-folder-{index}"))
        .w_full()
        .h(px(FILE_ROW_H))
        .px(px(8.))
        .gap(px(5.))
        .flex()
        .items_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .on_click(cx.listener(move |v, _, _, cx| super::toggle_folder(v, &key, cx)))
        .child(icon(if closed { Icon::ChevronRight } else { Icon::ChevronDown }).size(px(11.)).text_color(t.dim))
        .child(icon(Icon::Folder).size(px(12.)).text_color(t.blue))
        .child(div().flex_1().min_w_0().truncate().text_size(px(11.5)).text_color(t.muted).child(if name.is_empty() { "/".to_string() } else { name.to_string() }))
        .child(div().text_size(px(10.5)).text_color(t.dim).child(count.to_string()))
        .children(super::row_actions::folder_buttons(v, t, name, index, cx))
        .into_any_element()
}

fn file_row(v: &ChangesView, t: &Theme, index: usize, cx: &mut Context<ChangesView>) -> AnyElement {
    let Some(f) = v.files.get(index) else {
        return div().into_any_element();
    };
    let path = f.path.clone();
    let open = v.open.as_deref() == Some(f.path.as_str());
    let tone = status_color(t, f);
    let hover = t.hover;
    let grouped = !(v.files.iter().all(|x| x.folder.is_empty()));
    let letter = if f.conflict { "!".to_string() } else { f.status.to_string() };
    let mut row = div()
        .id(("changes-file", index))
        .debug_selector(move || format!("changes-file-{index}"))
        .group("changes-file")
        .w_full()
        .h(px(FILE_ROW_H))
        .pl(px(if grouped { 26. } else { 10. }))
        .pr(px(6.))
        .gap(px(6.))
        .flex()
        .items_center()
        .cursor_pointer()
        .when(open, |el| el.bg(t.selected))
        .hover(move |s| s.bg(hover))
        .tooltip(tooltip(format!(
            "{}\nClick to see the diff. Right click for more.",
            f.path
        )))
        .on_click(cx.listener({
            let path = path.clone();
            move |v, _, _, cx| super::open_diff(v, &path, cx)
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener({
                let path = path.clone();
                move |v, e: &MouseDownEvent, _, cx| super::open_file_menu(v, &path, e.position, cx)
            }),
        )
        .child(div().w(px(12.)).flex_none().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(tone).child(letter))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(if f.status == 'D' { t.dim } else { t.foreground })
                .child(base_name(&f.path).to_string()),
        );
    if f.staged {
        row = row.child(div().flex_none().size(px(6.)).rounded_full().bg(t.accent));
    }
    if f.binary {
        row = row.child(div().flex_none().text_size(px(10.5)).text_color(t.dim).child("binary"));
    } else {
        if f.add > 0 {
            row = row.child(div().flex_none().text_size(px(10.5)).text_color(t.green).child(format!("+{}", f.add)));
        }
        if f.del > 0 {
            row = row.child(div().flex_none().text_size(px(10.5)).text_color(t.red).child(format!("−{}", f.del)));
        }
    }
    let diff_path = path.clone();
    row = row.child(
        div()
            .id(("changes-diff-tab", index))
            .debug_selector(move || format!("changes-diff-tab-{index}"))
            .flex_none()
            .px(px(4.))
            .cursor_pointer()
            .rounded(px(4.))
            .hover(move |el| el.bg(hover))
            .text_color(t.accent)
            .tooltip(tooltip("Open diff in a tab"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |v, _, _, cx| {
                if let Some(repo) = v.root.clone() {
                    super::diff_tabs::open(repo, diff_path.clone(), cx);
                }
            }))
            .child("±"),
    );
    let label = base_name(&f.path).to_string();
    if f.conflict {
        let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
        let resolve = path.clone();
        row = row
            .child(
                tiny(t, format!("changes-resolve-{index}"), Icon::SquarePen, "Resolve conflict…", Some(t.red))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        if let Some(repo) = v.root.clone() {
                            cx.emit(super::ChangesEvent::Open(super::GitWindow::Conflict { repo, file: resolve.clone() }));
                        }
                    })),
            )
            .child(
                tiny(t, format!("changes-conflict-discard-{index}"), Icon::Undo, "Discard changes (asks first)", Some(t.red))
                    .on_click(cx.listener(move |v, _, _, cx| super::ask_discard(v, vec![path.clone()], &label, cx))),
            )
            .child(
                tiny(t, format!("changes-ours-{index}"), Icon::ArrowUp, "Keep current: use your side", None)
                    .on_click(cx.listener(move |v, _, _, cx| super::ask_side(v, &p1, true, cx))),
            )
            .child(
                tiny(t, format!("changes-theirs-{index}"), Icon::ArrowDown, "Take incoming: use their side", None)
                    .on_click(cx.listener(move |v, _, _, cx| super::ask_side(v, &p2, false, cx))),
            )
            .child(
                tiny(t, format!("changes-resolved-{index}"), Icon::Check, "Mark resolved: you fixed the file yourself", Some(t.green))
                    .on_click(cx.listener(move |v, _, _, cx| super::mark_resolved(v, p3.clone(), cx))),
            );
        return row.into_any_element();
    }
    let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
    let (l1, l2, l3) = (label.clone(), label.clone(), label);
    if f.y != ' ' {
        row = row.child(
            tiny(t, format!("changes-stage-{index}"), Icon::Plus, "Stage this file", None)
                .on_click(cx.listener(move |v, _, _, cx| super::stage(v, vec![p1.clone()], &l1, cx))),
        );
    }
    if f.staged {
        row = row.child(
            tiny(t, format!("changes-unstage-{index}"), Icon::Minus, "Unstage this file", None)
                .on_click(cx.listener(move |v, _, _, cx| super::unstage(v, vec![p2.clone()], &l2, cx))),
        );
    }
    row.child(
        tiny(t, format!("changes-discard-{index}"), Icon::Undo, "Discard the changes in this file (asks first)", Some(t.red))
            .on_click(cx.listener(move |v, _, _, cx| super::ask_discard(v, vec![p3.clone()], &l3, cx))),
    )
    .into_any_element()
}

fn file_list(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> AnyElement {
    if v.files.is_empty() {
        let text = if v.loading && !v.loaded { "Loading…" } else { "No changes. Everything is committed." };
        return div()
            .id("changes-empty")
            .debug_selector(|| "changes-empty".into())
            .flex_1()
            .px(px(12.))
            .py(px(10.))
            .text_size(px(12.))
            .text_color(t.dim)
            .child(text)
            .into_any_element();
    }
    let rows = list_rows(&v.files, &v.closed);
    let count = rows.len();
    uniform_list(
        "changes-files",
        count,
        cx.processor(move |v: &mut ChangesView, range: std::ops::Range<usize>, _, cx| {
            let t = theme(cx).clone();
            range
                .filter_map(|i| rows.get(i).cloned().map(|r| (i, r)))
                .map(|(i, r)| match r {
                    ListRow::Folder { name, count, closed } => folder_row(v, &t, &name, count, closed, i, cx),
                    ListRow::File(f) => file_row(v, &t, f, cx),
                })
                .collect::<Vec<_>>()
        }),
    )
    .flex_1()
    .min_h(px(60.))
    .into_any_element()
}

fn commit_box(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> AnyElement {
    let message = super::message_text(v, cx);
    let staged = super::staged_count(v);
    let working = v.busy || v.identity_busy || super::local_job(v);
    let empty = message.trim().is_empty() && !v.amend;
    let can_commit = !working && !empty && (staged > 0 || v.amend) && v.root.is_some();
    let can_all = !working && !empty && (!v.files.is_empty() || v.amend) && v.root.is_some();
    let label = if v.amend {
        "Amend".to_string()
    } else if staged > 0 {
        format!("Commit {staged}")
    } else {
        "Commit".into()
    };
    let commit_tip = if empty {
        "Write a commit message first".to_string()
    } else if staged == 0 && !v.amend {
        "Stage some files first, or use Commit all".to_string()
    } else if v.amend {
        "Change the last commit".to_string()
    } else {
        format!("Commit {}", plural(staged, "staged file", "staged files"))
    };
    let ai_tip = if v.ai_busy {
        format!("Stop {}", v.ai_label)
    } else {
        format!("Write the message with {}", v.ai_label)
    };
    let ai_off = !v.ai_busy && (v.busy || staged == 0);
    let show_desc = v.show_desc;
    let amend = v.amend;
    let input_box = |id: &'static str, child: AnyElement| {
        div().id(id).debug_selector(move || id.to_string()).min_w_0().child(child)
    };
    let mut col = div()
        .flex_none()
        .p(px(8.))
        .gap(px(6.))
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(t.border)
        .bg(t.chrome)
        .child(
            div()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(input_box("changes-message", div().flex_1().min_w_0().child(v.message.clone()).into_any_element()).flex_1())
                .child(
                    tiny(t, "changes-ai", if v.ai_busy { Icon::Square } else { Icon::Star }, ai_tip, Some(t.magenta))
                        .when(ai_off, |el| el.opacity(0.45).cursor_not_allowed())
                        .when(!ai_off, |el| el.on_click(cx.listener(|v, _, _, cx| super::suggest(v, cx))))
                    .when(!v.ai_busy, |el| el.on_mouse_down(MouseButton::Right, cx.listener(|v, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        super::prefs::open_agents(v, e.position, window, cx);
                    }))),
                ),
        );
    if show_desc {
        col = col.child(input_box("changes-description", div().flex_1().min_w_0().child(v.description.clone()).into_any_element()));
    }
    if let Some(fail) = &v.hook_fail {
        col = col.child(
            div()
                .id("changes-hook-fail")
                .debug_selector(|| "changes-hook-fail".into())
                .px(px(8.))
                .py(px(6.))
                .gap(px(4.))
                .flex()
                .flex_col()
                .rounded(px(7.))
                .border_1()
                .border_color(t.red.opacity(0.5))
                .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.red).child(fail.title.clone()))
                .when(!fail.detail.is_empty(), |el| el.child(div().text_size(px(11.)).text_color(t.muted).child(fail.detail.clone())))
                .child(
                    div().flex().child(
                        word_button(t, "changes-no-hooks", "Commit without hooks", "Skip the pre-commit and commit-msg checks this one time", working)
                            .on_click(cx.listener(|v, _, _, cx| super::commit_without_hooks(v, cx))),
                    ),
                ),
        );
    }
    col.child(
        div()
            .flex()
            .gap(px(8.))
            .items_center()
            .child(
                check_box(t, "changes-amend", amend, "Amend", !v.has_head)
                    .tooltip(tooltip("Change the last commit instead of making a new one"))
                    .when(v.has_head, |el| el.on_click(cx.listener(move |v, _, _, cx| super::toggle_amend(v, !amend, cx)))),
            )
            .child(
                check_box(t, "changes-show-desc", show_desc, "Description", false).on_click(cx.listener(move |v, _, _, cx| {
                    v.show_desc = !show_desc;
                    cx.notify();
                })),
            )
            .child(div().flex_1())
            .child(
                disabled(tiny(t, "changes-review", Icon::Search, "Ask an agent to review staged changes", None), staged == 0 || v.busy)
                    .on_click(cx.listener(|v, _, _, cx| super::emit_window(v, "review", cx))),
            )
            .child(
                word_button(t, "changes-commit-all", "All", "Stage everything and commit", !can_all)
                    .when(can_all, |el| el.on_click(cx.listener(|v, _, _, cx| super::commit(v, true, cx)))),
            )
            .child(
                disabled(
                    button(t, "changes-commit", label, ButtonKind::Primary)
                        .h(px(26.))
                        .px(px(12.))
                        .debug_selector(|| "changes-commit".into())
                        .tooltip(tooltip(commit_tip)),
                    !can_commit,
                )
                .when(can_commit, |el| el.on_click(cx.listener(|v, _, _, cx| super::commit(v, false, cx)))),
            ),
    )
    .into_any_element()
}

fn no_repo(v: &ChangesView, t: &Theme, cx: &mut Context<ChangesView>) -> AnyElement {
    let text = match (&v.context, &v.error) {
        (None, _) => "Open a terminal in a project folder to see its git changes.".to_string(),
        (_, Some(e)) if e.contains("Not a git") => "This folder is not a git repo.".to_string(),
        (_, Some(e)) => format!("Cannot read git here: {e}"),
        _ => "Loading…".to_string(),
    };
    div()
        .id("changes-no-repo")
        .debug_selector(|| "changes-no-repo".into())
        .flex_1()
        .p(px(14.))
        .text_size(px(12.))
        .text_color(t.dim)
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(text)
        .when(v.error.as_deref() == Some("Not a git repository"), |el| el.child(
            div().flex().gap(px(6.)).child(
                word_button(t, "changes-init", "Initialize repo", "Run git init in this folder", v.busy)
                    .on_click(cx.listener(|v, _, _, cx| super::row_actions::init_repo(v, cx))),
            ).child(
                word_button(t, "changes-clone", "Clone a repo…", "Clone a repository into this folder", false)
                    .on_click(cx.listener(|v, _, _, cx| crate::screens::git::accounts::clone_win::open(None, v.folder.clone(), cx))),
            ),
        ))
        .into_any_element()
}

fn popup_layer(v: &ChangesView, cx: &mut Context<ChangesView>) -> Option<AnyElement> {
    Some(match v.popup.as_ref()? {
        Popup::Repos { at, hi } => super::menus::repo_menu_view(v, *at, *hi, cx),
        Popup::Push { at, menu } => super::menus::push_menu_view(menu, *at, cx),
        Popup::File { at, menu } => super::menus::file_menu_view(menu, *at, cx),
        Popup::Rejected(r) => super::menus::rejected_view(r, cx),
        Popup::Identity(b) => super::menus::identity_view(b, cx),
        Popup::Agents { at, view, .. } => nixa_ui::popover::float_at(
            *at,
            gpui::Corner::BottomRight,
            div().id("changes-agent-popover")
                .occlude()
                .on_mouse_down_out(cx.listener(|v, _, _, cx| super::close_popup(v, cx)))
                .child(view.clone()),
        ).into_any_element(),
    })
}

impl Render for ChangesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let mut root = div()
            .id("changes-view")
            .debug_selector(|| "changes-view".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" && v.popup.is_some() {
                    super::close_popup(v, cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.background)
            .text_color(t.foreground)
            .child(top_bar(self, &t, cx))
            .child(action_bar(self, &t, cx));
        if self.root.is_none() {
            root = root.child(no_repo(self, &t, cx));
        } else if self.tab == Tab::Graph {
            let graph = self.graph.get_or_insert_with(|| {
                crate::screens::git::history::new_graph(self.root.as_ref().map(std::path::PathBuf::from), cx)
            });
            root = root.child(div().debug_selector(|| "changes-history-graph".into()).flex_1().min_h_0().child(graph.clone()));
        } else {
            let tops = banners(self, &t, cx);
            root = root.children(tops).child(files_header(self, &t, cx));
            match self.diff.clone() {
                Some(d) => {
                    root = root
                        .child(div().flex().flex_col().h(px(170.)).flex_none().child(file_list(self, &t, cx)))
                        .child(div().flex_1().min_h_0().border_t_1().border_color(t.border).child(d));
                }
                None => root = root.child(div().flex_1().min_h_0().flex().flex_col().child(file_list(self, &t, cx))),
            }
            root = root.child(commit_box(self, &t, cx));
        }
        root.children(popup_layer(self, cx))
    }
}
