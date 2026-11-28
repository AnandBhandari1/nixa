use super::kit::{self, Ctx};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, PathPromptOptions, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use nixa_git::accounts::profiles::{Identity, Profile, Remote, Suggestion, load_profiles, suggest_profile};
use nixa_git::accounts::settings::TRY_OTHERS_KEY;
use nixa_git::accounts::{Account, load_accounts, same_host, same_user, time};
use nixa_git::clone::{
    CloneOpts, Cloned, Input, Problem, RepoItem, clone_repo, default_folder, expand_home, filter_repos, folder_problem,
    list_account_repos, parse_clone_input, repo_name_of,
};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const WIDTH: f32 = 620.0;
pub const HEIGHT: f32 = 700.0;
pub const LIST_LIMIT: usize = 150;

#[derive(Clone, Debug, Default)]
pub struct List {
    pub repos: Vec<RepoItem>,
    pub page: u32,
    pub more: bool,
    pub loading: bool,
    pub error: Option<Problem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Form,
    Busy,
    Done,
}

pub struct CloneView {
    pub ctx: Ctx,
    pub focus: FocusHandle,
    pub into: Option<String>,
    pub accounts: Vec<Account>,
    pub profiles: Vec<Profile>,
    pub loaded: bool,
    pub list_key: Option<String>,
    pub lists: BTreeMap<String, List>,
    pub parsed: Input,
    pub auto_folder: String,
    pub folder_touched: bool,
    pub profile_id: Option<String>,
    pub suggestion: Option<Suggestion>,
    pub sign_in: Option<String>,
    pub picked: Option<(String, String, String)>,
    pub phase: Phase,
    pub cancel: Option<Arc<AtomicBool>>,
    pub step: String,
    pub error: Option<Cloned>,
    pub result: Option<(Cloned, String, Option<String>)>,
    pub link: Entity<TextInput>,
    pub filter: Entity<TextInput>,
    pub folder: Entity<TextInput>,
}

pub fn key_of(a: &Account) -> String {
    format!("{}:{}", a.host, a.username)
}

pub fn age(iso: Option<&str>, now: i64) -> String {
    let Some(at) = iso.and_then(time::parse) else { return String::new() };
    let days = (now - at).div_euclid(86_400);
    match days {
        d if d < 1 => "today".into(),
        d if d < 2 => "yesterday".into(),
        d if d < 31 => format!("{d} days ago"),
        d if d < 365 => format!("{} mo ago", d / 30),
        d => format!("{} y ago", d / 365),
    }
}

pub fn new(ctx: Ctx, url: Option<String>, into: Option<String>, cx: &mut App) -> Entity<CloneView> {
    cx.new(|cx| build(ctx, url, into, cx))
}

pub fn build(ctx: Ctx, url: Option<String>, into: Option<String>, cx: &mut Context<CloneView>) -> CloneView {
    let link = kit::input("https://github.com/owner/repo, or owner/repo", url.as_deref().unwrap_or(""), false, cx);
    let filter = kit::input("Filter your repos", "", false, cx);
    let folder = kit::input("~/Projects/repo", "", false, cx);
    cx.subscribe(&link, |v, _, e: &InputEvent, cx| match e {
        InputEvent::Changed(_) => {
            refresh(v, cx);
            cx.notify();
        }
        InputEvent::Submit(_) => start(v, cx),
        InputEvent::Cancel => {}
    })
    .detach();
    cx.subscribe(&filter, |_, _, e: &InputEvent, cx| {
        if let InputEvent::Changed(_) = e {
            cx.notify();
        }
    })
    .detach();
    cx.subscribe(&folder, |v, input, e: &InputEvent, cx| match e {
        InputEvent::Changed(_) => {
            let value = input.read(cx).value().to_string();
            if value.trim().is_empty() {
                v.folder_touched = false;
            } else if value != v.auto_folder {
                v.folder_touched = true;
            }
            suggest(v, cx);
            cx.notify();
        }
        InputEvent::Submit(_) => start(v, cx),
        InputEvent::Cancel => {}
    })
    .detach();
    let mut view = CloneView {
        ctx,
        focus: cx.focus_handle(),
        into,
        accounts: Vec::new(),
        profiles: Vec::new(),
        loaded: false,
        list_key: None,
        lists: BTreeMap::new(),
        parsed: parse_clone_input(""),
        auto_folder: String::new(),
        folder_touched: false,
        profile_id: None,
        suggestion: None,
        sign_in: None,
        picked: None,
        phase: Phase::Form,
        cancel: None,
        step: String::new(),
        error: None,
        result: None,
        link,
        filter,
        folder,
    };
    refresh(&mut view, cx);
    let env = view.ctx.env.clone();
    kit::job(cx, move || (load_accounts(&env), load_profiles(&env)), |v, (accounts, profiles), cx| {
        v.accounts = accounts.into_iter().filter(|a| !a.host.is_empty() && !a.username.is_empty()).collect();
        v.profiles = profiles;
        v.loaded = true;
        suggest(v, cx);
        let first = v
            .accounts
            .iter()
            .find(|a| a.default && a.host == "github.com")
            .or_else(|| v.accounts.iter().find(|a| a.host == "github.com"))
            .or_else(|| v.accounts.first())
            .cloned();
        if let Some(a) = first {
            pick_list(v, &a, cx);
        }
    });
    view
}

pub fn folder_path(v: &CloneView, cx: &App) -> String {
    expand_home(&kit::value(&v.folder, cx), &v.ctx.env.home).trim_end_matches('/').to_string()
}

pub fn folder_issue(v: &CloneView, cx: &App) -> Option<&'static str> {
    let text = kit::value(&v.folder, cx);
    if text.trim().is_empty() {
        return Some("Type the folder to clone into.");
    }
    folder_problem(&folder_path(v, cx), &v.ctx.env.home)
}

pub fn host_accounts(v: &CloneView) -> Vec<Account> {
    match (&v.parsed.host, v.parsed.ok && v.parsed.protocol == "https") {
        (Some(h), true) => v.accounts.iter().filter(|a| same_host(&a.host, h)).cloned().collect(),
        _ => Vec::new(),
    }
}

fn picked_now(v: &CloneView) -> Option<&(String, String, String)> {
    v.picked.as_ref().filter(|p| v.parsed.ok && p.2 == v.parsed.url)
}

pub fn chosen_profile(v: &CloneView) -> Option<Profile> {
    match &v.profile_id {
        Some(id) => v.profiles.iter().find(|p| &p.id == id).cloned(),
        None => v.suggestion.as_ref().map(|s| s.profile.clone()),
    }
}

pub fn sign_in_account(v: &CloneView) -> Option<Account> {
    let list = host_accounts(v);
    if list.is_empty() {
        return None;
    }
    let find = |name: Option<&str>| name.and_then(|n| list.iter().find(|a| same_user(&a.username, n)).cloned());
    let profile = chosen_profile(v);
    let from_profile = v.parsed.host.as_ref().and_then(|h| profile.as_ref().and_then(|p| p.accounts.get(h).cloned()));
    find(v.sign_in.as_deref())
        .or_else(|| find(picked_now(v).map(|p| p.1.as_str())))
        .or_else(|| find(from_profile.as_deref()))
        .or_else(|| find(v.parsed.user.as_deref()))
        .or_else(|| list.iter().find(|a| a.default).cloned())
        .or_else(|| list.first().cloned())
}

pub fn ready(v: &CloneView, cx: &App) -> bool {
    v.phase == Phase::Form && v.parsed.ok && folder_issue(v, cx).is_none() && chosen_profile(v).is_some()
}

fn guess_identity(parsed: &Input) -> Identity {
    let remotes = match (&parsed.host, parsed.ok) {
        (Some(h), true) => vec![Remote {
            name: "origin".into(),
            url: parsed.url.clone(),
            host: Some(h.clone()),
            owner: parsed.owner.clone(),
            repo: Some(parsed.repo.clone()),
            user: parsed.user.clone(),
            ..Remote::default()
        }],
        _ => Vec::new(),
    };
    Identity {
        profile_id: None,
        name: None,
        email: None,
        name_scope: "none".into(),
        email_scope: "none".into(),
        local_name: None,
        local_email: None,
        fallback_name: None,
        fallback_email: None,
        keep_identity: false,
        unmanaged: false,
        remotes,
        bindings: BTreeMap::new(),
        is_repo: true,
    }
}

pub fn suggest(v: &mut CloneView, cx: &App) {
    let identity = guess_identity(&v.parsed);
    let root = PathBuf::from(folder_path(v, cx));
    let mut found = suggest_profile(&v.profiles, &identity, &root, &v.ctx.env.home);
    if let Some((host, user, _)) = picked_now(v).cloned()
        && found.as_ref().is_none_or(|f| f.rule == "only" || f.rule == "folder")
        && let Some(mine) = v.profiles.iter().find(|p| p.accounts.get(&host).is_some_and(|u| same_user(u, &user)))
    {
        found = Some(Suggestion { profile: mine.clone(), rule: "picked", reason: format!("You picked it from the repos of @{user}.") });
    }
    v.suggestion = found;
}

pub fn refresh(v: &mut CloneView, cx: &mut Context<CloneView>) {
    v.parsed = parse_clone_input(&kit::value(&v.link, cx));
    if !v.folder_touched {
        v.auto_folder = if v.parsed.ok { default_folder(&v.ctx.env.home, &v.parsed.url, v.into.as_deref()) } else { String::new() };
        if kit::value(&v.folder, cx) != v.auto_folder {
            let next = v.auto_folder.clone();
            v.folder.update(cx, |i, cx| i.set_value(next, cx));
            v.folder_touched = false;
        }
    }
    if let Some(s) = v.sign_in.clone()
        && !host_accounts(v).iter().any(|a| same_user(&a.username, &s))
    {
        v.sign_in = None;
    }
    suggest(v, cx);
}

pub fn load_repos(v: &mut CloneView, account: &Account, page: u32, cx: &mut Context<CloneView>) {
    let key = key_of(account);
    let entry = v.lists.entry(key.clone()).or_default();
    entry.loading = true;
    entry.error = None;
    let env = v.ctx.env.clone();
    let (host, user) = (account.host.clone(), account.username.clone());
    kit::job(cx, move || list_account_repos(&env, &host, &user, page), move |v, got, _| {
        let now = v.lists.entry(key).or_default();
        now.loading = false;
        match got {
            Ok(r) => {
                if page == 1 {
                    now.repos.clear();
                }
                for item in r.repos {
                    if !now.repos.iter().any(|x| x.full_name == item.full_name) {
                        now.repos.push(item);
                    }
                }
                now.page = page;
                now.more = r.more;
            }
            Err(e) => now.error = Some(e),
        }
    });
    cx.notify();
}

pub fn pick_list(v: &mut CloneView, account: &Account, cx: &mut Context<CloneView>) {
    let key = key_of(account);
    v.list_key = Some(key.clone());
    if !v.lists.contains_key(&key) {
        load_repos(v, account, 1, cx);
    }
    cx.notify();
}

pub fn pick_repo(v: &mut CloneView, repo: &RepoItem, cx: &mut Context<CloneView>) {
    if v.phase == Phase::Busy {
        return;
    }
    let url = parse_clone_input(&repo.clone_url).url;
    let account = v.accounts.iter().find(|a| Some(key_of(a)) == v.list_key).cloned();
    v.picked = account.map(|a| (a.host, a.username, url));
    v.sign_in = None;
    let link = repo.clone_url.clone();
    v.link.update(cx, |i, cx| i.set_value(link, cx));
    refresh(v, cx);
    v.error = None;
    cx.notify();
}

pub fn choose_folder(v: &mut CloneView, cx: &mut Context<CloneView>) {
    if v.phase == Phase::Busy {
        return;
    }
    let answer = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Put the repo here".into()) });
    let name = repo_name_of(&v.parsed.url);
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = answer.await
            && let Some(path) = paths.first()
        {
            let base = path.display().to_string().trim_end_matches('/').to_string();
            let next = if !name.is_empty() && !base.ends_with(&format!("/{name}")) { format!("{base}/{name}") } else { base };
            this.update(cx, |v, cx| {
                v.folder_touched = true;
                v.folder.update(cx, |i, cx| i.set_value(next, cx));
                suggest(v, cx);
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
}

pub fn start(v: &mut CloneView, cx: &mut Context<CloneView>) {
    if !ready(v, cx) {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let opts = CloneOpts {
        url: v.parsed.url.clone(),
        folder: folder_path(v, cx),
        account: sign_in_account(v).map(|a| (a.host, a.username)),
        profile: chosen_profile(v),
        accounts: v.accounts.clone(),
        try_others: kit::get_setting(&v.ctx.db, TRY_OTHERS_KEY).and_then(|x| x.as_bool()).unwrap_or(true),
        cancel: Some(cancel.clone()),
        timeout: None,
    };
    let display = v.parsed.display.clone();
    let label = opts.profile.as_ref().map(|p| p.label.clone());
    v.phase = Phase::Busy;
    v.cancel = Some(cancel);
    v.error = None;
    v.step = format!("Cloning {display}…");
    let env = v.ctx.env.clone();
    kit::job(cx, move || clone_repo(&env, &opts), move |v, done, _| {
        v.cancel = None;
        if done.ok {
            v.result = Some((done, display, label));
            v.phase = Phase::Done;
        } else {
            v.error = Some(done);
            v.phase = Phase::Form;
        }
    });
    cx.notify();
}

pub fn stop(v: &mut CloneView, cx: &mut Context<CloneView>) {
    if let Some(c) = &v.cancel {
        c.store(true, Ordering::SeqCst);
        v.step = "Stopping…".into();
        cx.notify();
    }
}

pub fn open_space(folder: String, cx: &mut App) {
    let ws = cx.windows().into_iter().find_map(|w| w.downcast::<crate::workspace::Workspace>());
    if let Some(ws) = ws {
        ws.update(cx, |ws, window, cx| {
            ws.add_space(folder, cx);
            window.activate_window();
        })
        .ok();
    }
}

fn open_accounts(cx: &mut App) {
    cx.defer(|cx| crate::screens::settings::open(super::SETTINGS_PART, cx));
}

fn close(v: &mut CloneView, window: &mut Window) {
    if let Some(c) = &v.cancel {
        c.store(true, Ordering::SeqCst);
    }
    window.remove_window();
}

fn link_note(v: &CloneView, t: &Theme, cx: &App) -> gpui::Div {
    if kit::value(&v.link, cx).trim().is_empty() {
        return kit::hint(t, "Paste a link from GitHub, or type owner/repo. You can also pick one of your repos below.");
    }
    if !v.parsed.ok {
        return kit::warn_text(t, v.parsed.problem.clone().unwrap_or_default());
    }
    let from = v.parsed.host.as_ref().map(|h| format!(" from {h}")).unwrap_or_default();
    kit::hint(t, format!("Will clone {}{from}.", v.parsed.display))
}

fn repo_picker(v: &CloneView, t: &Theme, cx: &mut Context<CloneView>) -> AnyElement {
    if !v.loaded {
        return kit::busy_line(t, "clone-reading", "Reading your accounts…").into_any_element();
    }
    if v.accounts.is_empty() {
        return div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(kit::hint(t, "Add a GitHub account to pick from your repos."))
            .child(kit::row().child(kit::act(t, "clone-accounts", "Open accounts", ButtonKind::Secondary, false).on_click(|_, _, cx| open_accounts(cx))))
            .into_any_element();
    }
    let busy = v.phase == Phase::Busy;
    let account = v.accounts.iter().find(|a| Some(key_of(a)) == v.list_key).cloned();
    let list = account.as_ref().and_then(|a| v.lists.get(&key_of(a))).cloned();
    let q = kit::value(&v.filter, cx);
    let all = list.as_ref().map(|l| l.repos.clone()).unwrap_or_default();
    let shown: Vec<RepoItem> = filter_repos(&all, &q).into_iter().cloned().collect();
    let now = time::now();
    let mut chips = kit::wrap_row();
    for (i, a) in v.accounts.iter().enumerate() {
        let on = Some(key_of(a)) == v.list_key;
        let text = if a.host == "github.com" { format!("@{}", a.username) } else { format!("@{} · {}", a.username, a.host) };
        let a2 = a.clone();
        chips = chips.child(kit::pill(t, format!("clone-list-acct-{i}"), text, on, busy).on_click(cx.listener(move |v, _, _, cx| pick_list(v, &a2, cx))));
    }
    if let Some(a) = account.clone() {
        let loading = list.as_ref().is_some_and(|l| l.loading);
        chips = chips.child(div().flex_1()).child(kit::pill(t, "clone-list-reload", "↻ Reload", false, busy || loading).on_click(cx.listener(move |v, _, _, cx| load_repos(v, &a, 1, cx))));
    }
    let mut body = div()
        .id("clone-repo-list")
        .debug_selector(|| "clone-repo-list".into())
        .w_full()
        .h(px(168.))
        .flex_none()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .p(px(3.))
        .rounded(px(8.))
        .border_1()
        .border_color(t.border)
        .bg(t.chrome);
    if let Some(l) = &list {
        if l.loading && all.is_empty() {
            body = body.child(div().p(px(6.)).child(kit::busy_line(t, "clone-list-spin", "Loading repos…")));
        }
        if let Some(e) = &l.error {
            body = body.child(div().px(px(6.)).child(kit::error_text(t, e.title.clone())).when(!e.hint.is_empty(), |el| el.child(kit::hint(t, e.hint.clone()))));
        }
        if !l.loading && l.error.is_none() && shown.is_empty() {
            body = body.child(div().p(px(6.)).child(kit::hint(t, if q.trim().is_empty() { "This account has no repos." } else { "No repo matches the filter." })));
        }
    }
    for (i, repo) in shown.iter().take(LIST_LIMIT).enumerate() {
        let on = v.parsed.ok && parse_clone_input(&repo.clone_url).url == v.parsed.url;
        let tags: Vec<&str> = [(repo.private, "private"), (repo.fork, "fork"), (repo.archived, "archived")].into_iter().filter(|x| x.0).map(|x| x.1).collect();
        let r2 = repo.clone();
        let hover = t.hover;
        body = body.child(
            div()
                .id(("clone-repo", i))
                .min_h(px(30.))
                .flex_none()
                .px(px(8.))
                .py(px(3.))
                .flex()
                .flex_row()
                .gap(px(8.))
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(on, |el| el.bg(t.selected))
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |v, _, _, cx| pick_repo(v, &r2, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().text_size(px(12.5)).text_color(if on { t.accent } else { t.foreground }).child(repo.full_name.clone()))
                        .when(!repo.description.is_empty(), |el| el.child(div().truncate().text_size(px(11.)).text_color(t.dim).child(repo.description.clone()))),
                )
                .when(!tags.is_empty(), |el| el.child(div().flex_none().text_size(px(11.)).text_color(t.muted).child(tags.join(" · "))))
                .child(div().w(px(70.)).flex_none().text_size(px(11.)).text_color(t.dim).child(age(repo.pushed_at.as_deref(), now))),
        );
    }
    if shown.len() > LIST_LIMIT {
        body = body.child(div().p(px(6.)).child(kit::hint(t, format!("{} more. Type in the filter to find them.", shown.len() - LIST_LIMIT))));
    }
    if let (Some(l), Some(a)) = (&list, account)
        && l.more
        && l.error.is_none()
    {
        let next = l.page.max(1) + 1;
        body = body.child(
            kit::row().p(px(4.)).child(kit::act(t, "clone-more", if l.loading { "Loading…" } else { "Load more" }, ButtonKind::Secondary, l.loading).on_click(cx.listener(move |v, _, _, cx| load_repos(v, &a, next, cx)))),
        );
    }
    div().flex().flex_col().gap(px(6.)).child(chips).child(kit::field(&v.filter, "clone-filter")).child(body).into_any_element()
}

fn profile_picker(v: &CloneView, t: &Theme, cx: &mut Context<CloneView>) -> AnyElement {
    let chosen = chosen_profile(v);
    let suggested = v.suggestion.as_ref().map(|s| s.profile.id.clone());
    let busy = v.phase == Phase::Busy;
    let mut row = kit::wrap_row();
    for p in &v.profiles {
        let on = chosen.as_ref().is_some_and(|c| c.id == p.id);
        let text = if suggested.as_deref() == Some(p.id.as_str()) { format!("{} (suggested)", p.label) } else { p.label.clone() };
        let id = p.id.clone();
        row = row.child(
            kit::row()
                .w_auto()
                .gap(px(5.))
                .child(kit::dot(kit::profile_color(t, &p.color), 8.))
                .child(kit::pill(t, format!("clone-profile-{}", kit::safe_id(&p.id)), text, on, busy).on_click(cx.listener(move |v, _, _, cx| {
                    v.profile_id = Some(id.clone());
                    v.sign_in = None;
                    cx.notify();
                }))),
        );
    }
    let reason = match (&chosen, &v.profile_id, &v.suggestion) {
        (Some(_), None, Some(s)) if !s.reason.is_empty() => Some(s.reason.clone()),
        _ => None,
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(row)
        .when(v.profiles.is_empty() && v.loaded, |el| {
            el.child(kit::warn_text(t, "No profiles yet. Every repo needs a profile. Make one first."))
                .child(kit::row().child(kit::act(t, "clone-profiles", "Open profiles", ButtonKind::Secondary, false).on_click(|_, _, cx| open_accounts(cx))))
        })
        .when(!v.profiles.is_empty() && chosen.is_none(), |el| el.child(kit::warn_text(t, "Pick a profile. Every repo needs one. Your global git name and email are never used.")))
        .when_some(reason, |el, r| el.child(kit::hint(t, r)))
        .when_some(chosen, |el, p| el.child(kit::hint(t, format!("Commits use {} <{}>.", p.name, p.email))))
        .into_any_element()
}

fn sign_in_picker(v: &CloneView, t: &Theme, cx: &mut Context<CloneView>) -> AnyElement {
    let p = &v.parsed;
    match p.protocol.as_str() {
        "ssh" | "git" => return kit::hint(t, "SSH links sign in with your SSH keys.").into_any_element(),
        "local" => return kit::hint(t, "A folder on this computer needs no sign-in.").into_any_element(),
        "https" => {}
        _ => return kit::hint(t, "This link has no sign-in.").into_any_element(),
    }
    let list = host_accounts(v);
    if list.is_empty() {
        let host = p.host.clone().unwrap_or_default();
        return kit::hint(t, format!("No account for {host}. A public repo still works. For a private one, add an account in Settings, Git accounts.")).into_any_element();
    }
    let current = sign_in_account(v).map(|a| key_of(&a));
    let busy = v.phase == Phase::Busy;
    let mut row = kit::wrap_row();
    for (i, a) in list.iter().enumerate() {
        let on = current.as_deref() == Some(key_of(a).as_str());
        let name = a.username.clone();
        row = row.child(kit::pill(t, format!("clone-signin-{i}"), format!("@{}", a.username), on, busy).on_click(cx.listener(move |v, _, _, cx| {
            v.sign_in = Some(name.clone());
            cx.notify();
        })));
    }
    div().flex().flex_col().gap(px(4.)).child(row).child(kit::hint(t, "If this account cannot see the repo, your other accounts are tried.")).into_any_element()
}

fn error_block(v: &CloneView, t: &Theme, e: &Cloned) -> AnyElement {
    let home = v.ctx.env.home.clone();
    let folder = e.folder.clone();
    div()
        .pt(px(6.))
        .flex()
        .flex_col()
        .child(kit::error_text(t, if e.title.is_empty() { "The clone failed".to_string() } else { e.title.clone() }))
        .when(!e.hint.is_empty(), |el| el.child(kit::hint(t, e.hint.clone())))
        .when(!e.detail.is_empty(), |el| el.child(kit::hint(t, e.detail.clone()).pt(px(4.))))
        .when(e.kept && !e.folder.is_empty(), |el| {
            el.child(kit::small(t, format!("It is in {}", kit::short_path(&folder, &home))).pt(px(4.)))
                .child(kit::row().pt(px(8.)).child(kit::act(t, "clone-error-open", "Open as a space", ButtonKind::Secondary, false).on_click(move |_, window, cx| {
                    open_space(folder.clone(), cx);
                    window.remove_window();
                })))
        })
        .when(e.kind == "auth" || e.kind == "notFound", |el| {
            el.child(kit::row().pt(px(8.)).child(kit::act(t, "clone-error-accounts", "Open accounts", ButtonKind::Secondary, false).on_click(|_, _, cx| open_accounts(cx))))
        })
        .into_any_element()
}

fn footer(items: Vec<AnyElement>, t: &Theme) -> gpui::Div {
    kit::row().flex_none().justify_end().pt(px(10.)).mt(px(8.)).border_t_1().border_color(t.border).children(items)
}

fn form(v: &CloneView, t: &Theme, cx: &mut Context<CloneView>) -> (AnyElement, AnyElement) {
    let issue = if kit::value(&v.folder, cx).trim().is_empty() { None } else { folder_issue(v, cx) };
    let busy = v.phase == Phase::Busy;
    let repos = repo_picker(v, t, cx);
    let profiles = profile_picker(v, t, cx);
    let sign = v.parsed.ok.then(|| sign_in_picker(v, t, cx));
    let content = kit::scroll_body("clone-body")
        .child(kit::label(t, "Repo link or owner/name").pt(px(4.)))
        .child(kit::field(&v.link, "clone-link"))
        .child(div().pt(px(4.)).child(link_note(v, t, cx)))
        .child(kit::label(t, "Or pick one of your repos"))
        .child(repos)
        .child(kit::label(t, "Folder"))
        .child(
            kit::row()
                .child(div().flex_1().min_w_0().child(kit::field(&v.folder, "clone-folder")))
                .child(kit::act(t, "clone-choose", "Choose…", ButtonKind::Secondary, busy).on_click(cx.listener(|v, _, _, cx| choose_folder(v, cx)))),
        )
        .child(div().pt(px(4.)).child(match issue {
            Some(i) => kit::warn_text(t, i),
            None => kit::hint(t, "The folder must be new or empty. It is made for you."),
        }))
        .child(kit::label(t, "Profile for this repo"))
        .child(profiles)
        .when_some(sign, |el, s| el.child(kit::label(t, "Sign in as")).child(s))
        .when_some(v.error.clone(), |el, e| el.child(error_block(v, t, &e)))
        .into_any_element();
    let bottom = if busy {
        let stopping = v.cancel.as_ref().is_none_or(|c| c.load(Ordering::SeqCst));
        footer(
            vec![
                kit::row()
                    .flex_1()
                    .min_w_0()
                    .child(kit::busy_line(t, "clone-busy", v.step.clone()))
                    .child(kit::small(t, "Big repos can take a few minutes."))
                    .into_any_element(),
                kit::act(t, "clone-stop", "Stop", ButtonKind::Secondary, stopping).on_click(cx.listener(|v, _, _, cx| stop(v, cx))).into_any_element(),
            ],
            t,
        )
    } else {
        footer(
            vec![
                kit::act(t, "clone-cancel", "Cancel", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, window, _| close(v, window))).into_any_element(),
                kit::act(t, "clone-start", "Clone", ButtonKind::Primary, !ready(v, cx)).on_click(cx.listener(|v, _, _, cx| start(v, cx))).into_any_element(),
            ],
            t,
        )
    };
    (content, bottom.into_any_element())
}

fn done_body(v: &CloneView, t: &Theme, cx: &mut Context<CloneView>) -> (AnyElement, AnyElement) {
    let Some((r, display, label)) = v.result.clone() else { return (div().into_any_element(), div().into_any_element()) };
    let mut col = div().flex_1().min_h_0().pt(px(6.)).flex().flex_col().gap(px(2.)).child(kit::ok_text(t, format!("Cloned {display}.")));
    col = col.child(div().pt(px(6.)).text_size(px(12.5)).text_color(t.foreground).child(format!("It is in {}", kit::short_path(&r.folder, &v.ctx.env.home))));
    if let Some(l) = label {
        col = col.child(kit::hint(t, format!("Profile: {l}.")));
    }
    for (host, user) in &r.bound {
        let on = if host == "github.com" { String::new() } else { format!(" on {host}") };
        col = col.child(kit::hint(t, format!("Pushes and pulls sign in as @{user}{on}.")));
    }
    if let Some(u) = r.used.as_ref().filter(|u| u.fallback) {
        col = col.child(kit::warn_text(t, format!("It signed in as @{}.", u.username)));
    }
    for w in &r.warnings {
        col = col.child(kit::warn_text(t, w.clone()));
    }
    let folder = r.folder.clone();
    let bottom = footer(
        vec![
            kit::act(t, "clone-done-close", "Close", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, window, _| close(v, window))).into_any_element(),
            kit::act(t, "clone-open", "Open as a space", ButtonKind::Primary, false)
                .on_click(move |_, window, cx| {
                    open_space(folder.clone(), cx);
                    window.remove_window();
                })
                .into_any_element(),
        ],
        t,
    );
    (col.into_any_element(), bottom.into_any_element())
}

impl Focusable for CloneView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CloneView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let (content, bottom) = if self.phase == Phase::Done { done_body(self, &t, cx) } else { form(self, &t, cx) };
        div()
            .id("clone-view")
            .debug_selector(|| "clone-view".into())
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|v, event, window, cx| {
                match kit::window_key(event) {
                    Some(kit::WindowKey::Close) => close(v, window),
                    Some(kit::WindowKey::Submit) if v.phase == Phase::Form => start(v, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .size_full()
            .flex()
            .flex_col()
            .p(px(16.))
            .bg(t.background)
            .text_color(t.foreground)
            .child(div().pb(px(6.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(t.bright).child(SharedString::from("Clone a repo")))
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .child(bottom)
    }
}

pub fn open(url: Option<String>, into: Option<String>, cx: &mut App) {
    cx.defer(move |cx| open_now(kit::real(), url, into, cx));
}

pub fn open_now(ctx: Ctx, url: Option<String>, into: Option<String>, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("Clone a repo".into()), appears_transparent: false, traffic_light_position: None }),
        window_min_size: Some(size(px(480.), px(480.))),
        ..WindowOptions::default()
    };
    cx.open_window(options, |window, cx| {
        let view = new(ctx, url, into, cx);
        let link = view.read(cx).link.clone();
        window.focus(&link.focus_handle(cx));
        view
    })
    .ok();
}
