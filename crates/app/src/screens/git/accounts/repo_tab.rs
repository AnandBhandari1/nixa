use super::kit::{self, Ctx};
use super::profiles_tab::{apply_profile, picker};
use gpui::prelude::*;
use gpui::{AnyElement, App, ClipboardItem, Context, Entity, SharedString, Window, div, px};
use nixa_git::accounts::Env;
use nixa_git::accounts::helper::{git_ok, mask_url};
use nixa_git::accounts::profiles::{Identity, Profile, load_profiles, read_repo_identity, repo_warnings};
use nixa_git::accounts::repo::{
    Done, GITHUB_KEYS_URL, LOCK_MIN_AGE, Process, SignChange, Signing, SshKey, Stuck, Submodule, age_words,
    clear_stuck, enable_problem, list_submodules, running_git, set_signing, signing_info, signing_problems,
    signing_status, ssh_keys, stuck_state, sub_words, update_submodules,
};
use nixa_git::accounts::settings::{AUTO_FETCH_KEY, FETCH_MINUTES, PULL_MODE_KEY, PULL_MODES, Prefs, fetch_label, prefs_from};
use nixa_git::forge::{parse_remote, web_url_of};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SPEAK_KEY: &str = "git.speak";

const SIGN_WITH: &[(&str, &str, &str)] = &[
    ("ssh", "SSH key", "Uses a key from ~/.ssh. GitHub shows Verified when you add the same key there as a signing key."),
    ("openpgp", "GPG key", "Uses gpg. gpg must be installed on this computer."),
    ("x509", "X.509 certificate", "Uses gpgsm with a certificate, often from a company."),
];

#[derive(Default)]
pub struct Loaded {
    pub identity: Option<Result<Identity, String>>,
    pub profiles: Vec<Profile>,
    pub signing: Option<Result<Signing, String>>,
    pub submodules: Option<Result<Vec<Submodule>, String>>,
    pub stuck: Option<Stuck>,
    pub running: Vec<Process>,
    pub keys: Option<Result<Vec<SshKey>, String>>,
}

pub struct RepoTab {
    pub ctx: Ctx,
    pub root: PathBuf,
    pub data: Loaded,
    pub prefs: Prefs,
    pub speak: bool,
    pub loading: bool,
    pub busy: Option<String>,
    pub note: Option<(bool, String)>,
    pub confirm: Option<String>,
    pub picking: bool,
    pub editing_remote: Option<String>,
    pub sub_cancel: Option<Arc<AtomicBool>>,
    pub new_name: Entity<TextInput>,
    pub new_url: Entity<TextInput>,
    pub edit_url: Entity<TextInput>,
    pub sign_key: Entity<TextInput>,
}

pub fn new(ctx: Ctx, root: PathBuf, cx: &mut App) -> Entity<RepoTab> {
    cx.new(|cx| build(ctx, root, cx))
}

pub fn build(ctx: Ctx, root: PathBuf, cx: &mut Context<RepoTab>) -> RepoTab {
    let new_name = kit::input("Name, like origin or upstream", "", false, cx);
    let new_url = kit::input("URL, like https://github.com/owner/repo.git", "", false, cx);
    let edit_url = kit::input("New URL", "", false, cx);
    let sign_key = kit::input("Like ~/.ssh/id_ed25519.pub, or a GPG key id", "", false, cx);
    cx.subscribe(&new_url, |v, _, e: &InputEvent, cx| {
        if let InputEvent::Submit(_) = e {
            add_remote(v, cx)
        }
    })
    .detach();
    cx.subscribe(&edit_url, |v, _, e: &InputEvent, cx| match e {
        InputEvent::Submit(_) => save_url(v, cx),
        InputEvent::Cancel => {
            v.editing_remote = None;
            cx.notify();
        }
        InputEvent::Changed(_) => {}
    })
    .detach();
    cx.subscribe(&sign_key, |v, _, e: &InputEvent, cx| {
        if let InputEvent::Submit(_) = e {
            save_key(v, cx)
        }
    })
    .detach();
    let values = kit::all_settings(&ctx.db);
    let prefs = prefs_from(&values);
    let speak = values.get(SPEAK_KEY).and_then(|v| v.as_bool()).unwrap_or(true);
    let mut view = RepoTab {
        ctx,
        root,
        data: Loaded::default(),
        prefs,
        speak,
        loading: true,
        busy: None,
        note: None,
        confirm: None,
        picking: false,
        editing_remote: None,
        sub_cancel: None,
        new_name,
        new_url,
        edit_url,
        sign_key,
    };
    load(&mut view, cx);
    view
}

pub fn read_all(env: &Env, root: &Path) -> Loaded {
    let stuck = stuck_state(env, root).ok();
    let running = if stuck.as_ref().is_some_and(|s| s.index_lock.is_some()) { running_git(env, root).unwrap_or_default() } else { Vec::new() };
    Loaded {
        identity: Some(read_repo_identity(env, root)),
        profiles: load_profiles(env),
        signing: Some(signing_info(env, root)),
        submodules: Some(list_submodules(env, root)),
        stuck,
        running,
        keys: Some(ssh_keys(env)),
    }
}

pub fn load(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    v.loading = true;
    let env = v.ctx.env.clone();
    let root = v.root.clone();
    kit::job(cx, move || read_all(&env, &root), |v, data, cx| {
        let key = data.signing.as_ref().and_then(|s| s.as_ref().ok()).and_then(|s| s.key.clone()).unwrap_or_default();
        let shown = kit::short_path(&key, &v.ctx.env.home);
        v.sign_key.update(cx, |i, cx| i.set_value(shown, cx));
        v.data = data;
        v.loading = false;
    });
    cx.notify();
}

fn run(v: &mut RepoTab, tag: &str, work: impl FnOnce(&Env, &Path) -> Result<String, String> + Send + 'static, cx: &mut Context<RepoTab>) {
    if v.busy.is_some() {
        return;
    }
    v.busy = Some(tag.to_string());
    v.confirm = None;
    v.note = None;
    let env = v.ctx.env.clone();
    let root = v.root.clone();
    kit::job(cx, move || work(&env, &root), |v, done, cx| {
        v.busy = None;
        v.note = Some(match done {
            Ok(text) => (true, text),
            Err(text) => (false, text),
        });
        load(v, cx);
    });
    cx.notify();
}

fn done_result(d: Done) -> Result<String, String> {
    if d.ok { Ok(d.text()) } else { Err(d.text()) }
}

fn expand(text: &str, home: &Path) -> String {
    match text.strip_prefix("~/") {
        Some(rest) => home.join(rest).display().to_string(),
        None => text.to_string(),
    }
}

pub fn name_ok(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

pub fn add_remote(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    let name = kit::value(&v.new_name, cx).trim().to_string();
    let url = kit::value(&v.new_url, cx).trim().to_string();
    let taken = v.data.identity.as_ref().and_then(|i| i.as_ref().ok()).is_some_and(|i| i.remotes.iter().any(|r| r.name == name));
    let problem = if name.is_empty() || url.is_empty() {
        Some("Type a name and a URL first.".to_string())
    } else if !name_ok(&name) {
        Some("Use only letters, numbers, dot, dash or underscore in the name.".to_string())
    } else if taken {
        Some(format!("A remote named {name} already exists."))
    } else {
        None
    };
    if let Some(p) = problem {
        v.note = Some((false, p));
        cx.notify();
        return;
    }
    v.new_name.update(cx, |i, cx| i.set_value("", cx));
    v.new_url.update(cx, |i, cx| i.set_value("", cx));
    run(v, "remote-add", move |env, root| git_ok(env, root, &["remote", "add", "--", &name, &url]).map(|_| format!("Added {name}.")), cx);
}

pub fn start_edit_url(v: &mut RepoTab, name: &str, url: &str, window: &mut Window, cx: &mut Context<RepoTab>) {
    v.editing_remote = Some(name.to_string());
    v.confirm = None;
    let url = url.to_string();
    v.edit_url.update(cx, |i, cx| i.set_value(url, cx));
    window.focus(&gpui::Focusable::focus_handle(v.edit_url.read(cx), cx));
    cx.notify();
}

pub fn save_url(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    let Some(name) = v.editing_remote.clone() else { return };
    let url = kit::value(&v.edit_url, cx).trim().to_string();
    if url.is_empty() {
        v.note = Some((false, "Type the new URL first.".into()));
        cx.notify();
        return;
    }
    v.editing_remote = None;
    run(v, "remote-edit", move |env, root| git_ok(env, root, &["remote", "set-url", "--", &name, &url]).map(|_| format!("Changed the URL of {name}.")), cx);
}

pub fn remove_remote(v: &mut RepoTab, name: &str, cx: &mut Context<RepoTab>) {
    let name = name.to_string();
    run(v, "remote-remove", move |env, root| git_ok(env, root, &["remote", "remove", "--", &name]).map(|_| format!("Removed {name}.")), cx);
}

pub fn set_sign(v: &mut RepoTab, change: SignChange, ok: String, cx: &mut Context<RepoTab>) {
    run(
        v,
        "sign",
        move |env, root| {
            let (d, _) = set_signing(env, root, &change);
            if d.ok { Ok(ok) } else { Err(d.text()) }
        },
        cx,
    );
}

pub fn toggle_sign(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    let Some(Ok(info)) = v.data.signing.clone() else { return };
    let home = v.ctx.env.home.clone();
    if !info.enabled
        && let Some(p) = enable_problem(&info, &|p| kit::short_path(p, &home))
    {
        v.note = Some((false, p));
        cx.notify();
        return;
    }
    let on = !info.enabled;
    let text = if on { "Signing is on for this repo. New commits here will be signed." } else { "Signing is off for this repo." };
    set_sign(v, SignChange { enabled: Some(Some(on)), ..SignChange::default() }, text.into(), cx);
}

pub fn save_key(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    let text = kit::value(&v.sign_key, cx).trim().to_string();
    let key = (!text.is_empty()).then(|| expand(&text, &v.ctx.env.home));
    let ssh = v.data.signing.as_ref().and_then(|s| s.as_ref().ok()).is_some_and(|s| s.format == "ssh");
    let ok = match (&key, ssh) {
        (None, _) => "Removed the key from this repo.",
        (Some(_), true) => "Saved the key. This repo now signs with an SSH key.",
        (Some(_), false) => "Saved the key for this repo.",
    };
    set_sign(v, SignChange { key: Some(key), ..SignChange::default() }, ok.into(), cx);
}

pub fn use_key(v: &mut RepoTab, key: &SshKey, cx: &mut Context<RepoTab>) {
    let change = SignChange { format: Some(Some("ssh".into())), key: Some(Some(key.path.clone())), ..SignChange::default() };
    set_sign(v, change, format!("This repo now signs with {}.", key.name), cx);
}

pub fn set_pref(v: &mut RepoTab, key: &str, value: Value, cx: &mut Context<RepoTab>) {
    match kit::set_setting(&v.ctx.db, key, &value) {
        Ok(()) => {
            v.prefs = prefs_from(&kit::all_settings(&v.ctx.db));
            if key == SPEAK_KEY {
                v.speak = value.as_bool().unwrap_or(true);
            }
        }
        Err(e) => v.note = Some((false, format!("Could not save the settings: {e}"))),
    }
    cx.notify();
}

pub fn update_subs(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    if let Some(stop) = v.sub_cancel.take() {
        stop.store(true, Ordering::SeqCst);
        cx.notify();
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    v.sub_cancel = Some(stop.clone());
    run(
        v,
        "subs",
        move |env, root| {
            let d = update_submodules(env, root, Some(stop));
            if d.ok { Ok("The submodules are up to date.".into()) } else { Err(d.text()) }
        },
        cx,
    );
}

pub fn fix(v: &mut RepoTab, what: &'static str, cx: &mut Context<RepoTab>) {
    run(v, what, move |env, root| done_result(clear_stuck(env, root, what)), cx);
}

pub fn remove_setup(v: &mut RepoTab, cx: &mut Context<RepoTab>) {
    let app = super::accounts_tab::APP;
    run(
        v,
        "setup",
        move |env, root| {
            nixa_git::accounts::profiles::clear_repo(env, root)
                .map(|_| format!("Removed. {app} will not set up this repo again until you pick a profile for it. Your name and email in this repo stay as they are."))
                .map_err(|e| format!("Could not remove the setup: {e}"))
        },
        cx,
    );
}

pub fn pick_profile(v: &mut RepoTab, profile: Option<Profile>, cx: &mut Context<RepoTab>) {
    v.picking = false;
    run(v, "profile", move |env, root| apply_profile(env, root, profile.as_ref()), cx);
}

fn section(t: &Theme, title: &str) -> gpui::Div {
    kit::label(t, title.to_string()).pt(px(18.))
}

fn ask(v: &RepoTab, t: &Theme, id: &str, text: String, yes: &str, then: impl Fn(&mut RepoTab, &mut Context<RepoTab>) + 'static, cx: &mut Context<RepoTab>) -> AnyElement {
    let me = id.to_string();
    if v.confirm.as_deref() != Some(id) {
        return div().into_any_element();
    }
    div()
        .pt(px(6.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(kit::warn_text(t, text))
        .child(
            kit::row()
                .child(
                    kit::act(t, format!("{me}-no"), "Cancel", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, _, cx| {
                        v.confirm = None;
                        cx.notify();
                    })),
                )
                .child(kit::act(t, format!("{me}-yes"), yes.to_string(), ButtonKind::Danger, v.busy.is_some()).on_click(cx.listener(move |v, _, _, cx| then(v, cx)))),
        )
        .into_any_element()
}

fn open_confirm(id: &'static str) -> impl Fn(&mut RepoTab, &gpui::ClickEvent, &mut Window, &mut Context<RepoTab>) {
    move |v, _, _, cx| {
        v.confirm = if v.confirm.as_deref() == Some(id) { None } else { Some(id.to_string()) };
        cx.notify();
    }
}

fn stuck_items(v: &RepoTab, t: &Theme, cx: &mut Context<RepoTab>) -> Option<AnyElement> {
    let s = v.data.stuck.as_ref()?;
    if !s.stuck() {
        return None;
    }
    let mut col = kit::card(t, "repo-stuck", true).child(kit::hint(t, "Git left something behind that the Changes panel cannot fix."));
    if let Some(start) = &s.bisect {
        let from = start.as_ref().map(|x| format!(" (it started on {x})")).unwrap_or_default();
        let back = start.clone().unwrap_or_else(|| "where you started".into());
        col = col
            .child(kit::strong(t, format!("A bisect is still going on{from}.")))
            .child(kit::hint(t, "While it runs, git keeps you on old commits. Stop it when you are done."))
            .child(kit::row().child(kit::act(t, "repo-stuck-bisect", "Stop the bisect", ButtonKind::Secondary, v.busy.is_some()).on_click(cx.listener(open_confirm("bisect")))))
            .child(ask(v, t, "bisect", format!("Stop the bisect and go back to {back}? If your changed files are in the way, git stops and tells you."), "Yes, do it", |v, cx| fix(v, "bisect", cx), cx));
    }
    if let Some((op, _, running)) = &s.sequencer {
        if *running {
            col = col.child(kit::strong(t, format!("A {op} stopped on a conflict."))).child(kit::hint(t, "Fix the conflict, then use Continue or Abort in the Changes panel."));
        } else {
            let done = if op == "revert" { "reverted" } else { "picked" };
            col = col
                .child(kit::strong(t, format!("A {op} that did not finish left its list of commits behind.")))
                .child(kit::hint(t, format!("Git shows a {op} in progress until the list is cleared.")))
                .child(kit::row().child(kit::act(t, "repo-stuck-seq", "Clear the list", ButtonKind::Secondary, v.busy.is_some()).on_click(cx.listener(open_confirm("sequencer")))))
                .child(ask(v, t, "sequencer", format!("Clear the list? Your files and commits stay as they are. Commits in the list that were not {done} yet are skipped."), "Yes, do it", |v, cx| fix(v, "sequencer", cx), cx));
        }
    }
    if let Some((name, age)) = &s.index_lock {
        let fresh = age.is_none_or(|a| a < LOCK_MIN_AGE) || !v.data.running.is_empty();
        let words = age_words(age.unwrap_or(0));
        if fresh {
            col = col
                .child(kit::strong(t, format!("A git command is working in this repo now ({name} is {words} old).")))
                .child(kit::hint(t, "Wait for it to end, then press Check again."));
        } else {
            col = col
                .child(kit::strong(t, format!("A lock file was left behind by a git command that stopped ({name}, {words} old).")))
                .child(kit::hint(t, "Git cannot change this repo while it is there."))
                .child(kit::row().child(kit::act(t, "repo-stuck-lock", "Remove the lock file", ButtonKind::Secondary, v.busy.is_some()).on_click(cx.listener(open_confirm("indexLock")))))
                .child(ask(
                    v,
                    t,
                    "indexLock",
                    format!("Remove the lock file? Do this only when no git command is running in this repo. {} checks that first and stops if one is running.", super::accounts_tab::APP),
                    "Yes, do it",
                    |v, cx| fix(v, "indexLock", cx),
                    cx,
                ));
        }
    }
    if let Some(dir) = &s.rebase_dir {
        if dir.contains("apply") {
            col = col.child(kit::strong(t, "A git am is in progress.")).child(kit::hint(t, "Finish or stop it in a terminal with git am --continue or git am --abort."));
        } else {
            col = col.child(kit::strong(t, "A rebase is in progress.")).child(kit::hint(t, "Use Continue or Abort in the Changes panel."));
        }
    }
    if s.merge_head.is_some() {
        col = col.child(kit::strong(t, "A merge is in progress.")).child(kit::hint(t, "Use Continue or Abort in the Changes panel."));
    }
    let checking = v.loading;
    Some(
        div()
            .flex()
            .flex_col()
            .child(section(t, "Fix a stuck repo"))
            .child(col.child(kit::row().pt(px(4.)).child(kit::act(t, "repo-stuck-check", if checking { "Checking…" } else { "Check again" }, ButtonKind::Secondary, checking).on_click(cx.listener(|v, _, _, cx| load(v, cx))))))
            .into_any_element(),
    )
}

fn identity_block(v: &RepoTab, t: &Theme, cx: &mut Context<RepoTab>) -> AnyElement {
    let app = super::accounts_tab::APP;
    let mut col = div().flex().flex_col().gap(px(3.));
    let identity = match &v.data.identity {
        None => return kit::busy_line(t, "repo-id-loading", "Loading…").into_any_element(),
        Some(Err(e)) => return kit::error_text(t, format!("Could not read the identity: {e}")).into_any_element(),
        Some(Ok(i)) if !i.is_repo => return kit::hint(t, "This folder is not a git repo.").into_any_element(),
        Some(Ok(i)) => i,
    };
    let profile = identity.profile_id.as_ref().and_then(|id| v.data.profiles.iter().find(|p| &p.id == id));
    col = col.child(
        kit::row()
            .child(kit::small(t, "Profile").w(px(84.)).flex_none())
            .child(match profile {
                Some(p) => kit::row().w_auto().child(kit::dot(kit::profile_color(t, &p.color), 8.)).child(kit::strong(t, p.label.clone())).into_any_element(),
                None if identity.unmanaged => kit::warn_text(t, format!("Not managed by {app}. Pick a profile to manage it again.")).pt(px(0.)).into_any_element(),
                None => kit::warn_text(t, "No profile picked for this repo").pt(px(0.)).into_any_element(),
            })
            .child(div().flex_1())
            .child(kit::act(t, "repo-id-change", if v.picking { "Close" } else { "Change" }, ButtonKind::Secondary, v.busy.is_some()).on_click(cx.listener(|v, _, _, cx| {
                v.picking = !v.picking;
                cx.notify();
            }))),
    );
    if v.picking {
        let me = cx.entity().downgrade();
        if v.data.profiles.is_empty() {
            col = col.child(kit::hint(t, "Pick or make a profile for this repo in the Profiles tab."));
        } else {
            col = col.child(picker(t, "repo-pick", &v.data.profiles, identity.profile_id.as_deref(), v.busy.is_some(), move |p, cx| {
                me.update(cx, |v, cx| pick_profile(v, p, cx)).ok();
            }));
        }
    }
    let who = match (&identity.name, &identity.email) {
        (None, None) => "No name or email".to_string(),
        (n, e) => format!("{} <{}>", n.clone().unwrap_or_default(), e.clone().unwrap_or_default()),
    };
    col = col.child(kit::row().child(kit::small(t, "Commits as").w(px(84.)).flex_none()).child(kit::small(t, who).flex_1()));
    let scope = identity.email_scope.as_str();
    col = col.child(match scope {
        "local" => kit::hint(t, "Name and email are set in this repo."),
        "none" => kit::warn_text(t, "No email is set. Commits will fail until you pick a profile."),
        other => kit::warn_text(t, format!("Name and email come from your {other} git config. Pick a profile so this repo has its own.")),
    });
    let mut sign = div().flex_1().min_w_0().flex().flex_col();
    for (host, b) in &identity.bindings {
        if let Some(u) = &b.username {
            sign = sign.child(kit::small(t, format!("{host}: signs in as {u}{}", if b.helper_installed { "" } else { " (sign-in helper missing)" })));
        }
    }
    if identity.bindings.values().all(|b| b.username.is_none()) {
        sign = sign.child(kit::small(t, "This repo uses your default account for each host."));
    }
    col = col.child(kit::row().items_start().child(kit::small(t, "Sign in").w(px(84.)).flex_none()).child(sign));
    for w in repo_warnings(identity, profile) {
        col = col.child(kit::warn_text(t, w));
    }
    col.into_any_element()
}

fn signing_block(v: &RepoTab, t: &Theme, cx: &mut Context<RepoTab>) -> AnyElement {
    let home = v.ctx.env.home.clone();
    let short = move |p: &str| kit::short_path(p, &home);
    let info = match &v.data.signing {
        None => return kit::busy_line(t, "repo-sign-loading", "Loading…").into_any_element(),
        Some(Err(e)) => return kit::error_text(t, format!("Could not read the signing setting: {e}")).into_any_element(),
        Some(Ok(i)) => i.clone(),
    };
    let busy = v.busy.is_some();
    let mut col = div().flex().flex_col().gap(px(3.)).child(kit::small(t, signing_status(&info, &short)));
    for p in signing_problems(&info, &short) {
        col = col.child(kit::warn_text(t, p));
    }
    col = col
        .child(kit::check(t, "repo-sign-on", info.enabled, "Sign my commits in this repo").on_click(cx.listener(|v, _, _, cx| toggle_sign(v, cx))))
        .child(kit::hint(t, "Only this repo changes. Your global git config stays as it is."));
    if info.local_enabled.is_some() {
        col = col.child(
            kit::row()
                .child(kit::hint(t, "This repo has its own on or off setting for signing.").w_auto())
                .child(kit::act(t, "repo-sign-global", "Use my global setting", ButtonKind::Secondary, busy).on_click(cx.listener(|v, _, _, cx| {
                    set_sign(v, SignChange { enabled: Some(None), ..SignChange::default() }, "This repo now follows your global git config for signing.".into(), cx)
                }))),
        );
    }
    col = col.child(kit::small(t, "Sign with").pt(px(6.)));
    for (id, label, detail) in SIGN_WITH {
        let on = info.format == *id;
        col = col.child(kit::radio(t, format!("repo-sign-{id}"), on, *label, detail).on_click(cx.listener(move |v, _, _, cx| {
            if !on {
                set_sign(v, SignChange { format: Some(Some(id.to_string())), ..SignChange::default() }, format!("This repo now signs with {label}."), cx)
            }
        })));
    }
    let hint = match info.format.as_str() {
        "ssh" => "Type the path of your public key file, like ~/.ssh/id_ed25519.pub, or press Use for signing on a key below.",
        "x509" => "Type the certificate id. Leave it empty to use the one that matches your email.",
        _ => "Type your GPG key id. Leave it empty to use the key that matches your email.",
    };
    col.child(
        kit::row()
            .pt(px(4.))
            .child(div().flex_1().min_w_0().child(kit::field(&v.sign_key, "repo-sign-key")))
            .child(kit::act(t, "repo-sign-save", "Save key", ButtonKind::Secondary, busy).on_click(cx.listener(|v, _, _, cx| save_key(v, cx)))),
    )
    .child(kit::hint(t, hint))
    .child(kit::hint(t, "The app has no terminal, so it cannot ask for a passphrase. If your key has one, add it to the SSH agent first: run ssh-add in a terminal."))
    .into_any_element()
}

fn remotes_block(v: &RepoTab, t: &Theme, cx: &mut Context<RepoTab>) -> AnyElement {
    let Some(Ok(identity)) = &v.data.identity else { return div().into_any_element() };
    let busy = v.busy.is_some();
    let mut col = div().flex().flex_col();
    if identity.remotes.is_empty() {
        col = col.child(kit::hint(t, "This repo has no remotes yet."));
    }
    for r in &identity.remotes {
        let sid = kit::safe_id(&r.name);
        let web = parse_remote(&r.url).map(|x| web_url_of(&x));
        let (n1, n2, u1) = (r.name.clone(), r.name.clone(), r.url.clone());
        let editing = v.editing_remote.as_deref() == Some(r.name.as_str());
        let mut card = kit::card(t, SharedString::from(format!("repo-remote-{sid}")), false)
            .child(kit::row().child(kit::strong(t, r.name.clone()).w_auto().flex_none()).child(kit::small(t, mask_url(&r.url)).flex_1()))
            .when(!r.push_url.is_empty() && r.push_url != r.url, |el| el.child(kit::hint(t, format!("Push to: {}", mask_url(&r.push_url)))))
            .when(r.token_in_url, |el| el.child(kit::warn_text(t, format!("The {} URL has a password or token in it. Remove it from the URL and use an account instead.", r.name))));
        if editing {
            card = card.child(
                kit::row()
                    .child(div().flex_1().min_w_0().child(kit::field(&v.edit_url, &format!("repo-remote-{sid}-url"))))
                    .child(kit::act(t, format!("repo-remote-{sid}-cancel"), "Cancel", ButtonKind::Secondary, false).on_click(cx.listener(|v, _, _, cx| {
                        v.editing_remote = None;
                        cx.notify();
                    })))
                    .child(kit::act(t, format!("repo-remote-{sid}-save"), "Save", ButtonKind::Primary, busy).on_click(cx.listener(|v, _, _, cx| save_url(v, cx)))),
            );
        } else {
            let confirm_id = format!("remote:{}", r.name);
            let c2 = confirm_id.clone();
            card = card.child(
                kit::row()
                    .pt(px(4.))
                    .when_some(web, |el, url| el.child(kit::act(t, format!("repo-remote-{sid}-web"), "Open on web", ButtonKind::Secondary, false).on_click(move |_, _, cx| cx.open_url(&url))))
                    .child(kit::act(t, format!("repo-remote-{sid}-edit"), "Edit URL", ButtonKind::Secondary, busy).on_click(cx.listener(move |v, _, window, cx| start_edit_url(v, &n1, &u1, window, cx))))
                    .child(kit::act(t, format!("repo-remote-{sid}-remove"), "Remove", ButtonKind::Danger, busy).on_click(cx.listener(move |v, _, _, cx| {
                        v.confirm = if v.confirm.as_deref() == Some(c2.as_str()) { None } else { Some(c2.clone()) };
                        cx.notify();
                    }))),
            );
            card = card.child(ask(v, t, &confirm_id, format!("Remove the remote {}? Branches that track it lose that link.", r.name), "Remove", move |v, cx| remove_remote(v, &n2, cx), cx));
        }
        col = col.child(card);
    }
    col.child(
        kit::row()
            .pt(px(4.))
            .child(div().w(px(150.)).flex_none().child(kit::field(&v.new_name, "repo-remote-new-name")))
            .child(div().flex_1().min_w_0().child(kit::field(&v.new_url, "repo-remote-new-url")))
            .child(kit::act(t, "repo-remote-add", if v.busy.as_deref() == Some("remote-add") { "Adding…" } else { "Add remote" }, ButtonKind::Secondary, busy).on_click(cx.listener(|v, _, _, cx| add_remote(v, cx)))),
    )
    .into_any_element()
}

fn submodules_block(v: &RepoTab, t: &Theme, cx: &mut Context<RepoTab>) -> Option<AnyElement> {
    let list = match &v.data.submodules {
        Some(Ok(l)) if !l.is_empty() => l.clone(),
        Some(Err(e)) => return Some(kit::error_text(t, format!("Could not read the submodules: {e}")).into_any_element()),
        _ => return None,
    };
    let mut col = div().flex().flex_col().gap(px(3.));
    for s in &list {
        let (word, tone) = sub_words(s.state);
        let color = match tone {
            "ok" => t.green,
            "warn" => t.yellow,
            "bad" => t.red,
            _ => t.muted,
        };
        col = col.child(
            kit::row()
                .child(kit::dot(color, 7.))
                .child(div().min_w_0().truncate().text_size(px(12.5)).text_color(t.foreground).child(s.path.clone()))
                .child(kit::small(t, word))
                .when_some(s.short.clone(), |el, sha| el.child(kit::small(t, sha))),
        );
    }
    let running = v.sub_cancel.is_some() && v.busy.as_deref() == Some("subs");
    let label = match (running, v.sub_cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst))) {
        (true, true) => "Stopping…",
        (true, false) => "Stop",
        _ => "Update all",
    };
    Some(
        col.child(kit::hint(t, "Update all downloads the missing submodules and checks out the commit this repo records in each one."))
            .child(
                kit::row()
                    .child(kit::act(t, "repo-subs-update", label, ButtonKind::Secondary, v.busy.is_some() && !running).on_click(cx.listener(|v, _, _, cx| update_subs(v, cx))))
                    .when(running, |el| el.child(kit::busy_line(t, "repo-subs-spin", "Updating…"))),
            )
            .into_any_element(),
    )
}

fn keys_block(v: &RepoTab, t: &Theme, cx: &mut Context<RepoTab>) -> AnyElement {
    let keys = match &v.data.keys {
        None => return kit::busy_line(t, "repo-keys-loading", "Loading…").into_any_element(),
        Some(Err(e)) => return kit::error_text(t, format!("Could not read the SSH keys: {e}")).into_any_element(),
        Some(Ok(k)) => k.clone(),
    };
    let current = v.data.signing.as_ref().and_then(|s| s.as_ref().ok()).filter(|s| s.format == "ssh").and_then(|s| s.key.clone());
    let mut col = div().flex().flex_col();
    if keys.is_empty() {
        col = col.child(kit::hint(t, "No public keys in ~/.ssh. To make one, run ssh-keygen -t ed25519 in a terminal."));
    }
    for k in &keys {
        let sid = kit::safe_id(&k.name);
        let used = current.as_deref() == Some(k.path.as_str());
        let (pk, name, k2) = (k.public_key.clone(), k.name.clone(), k.clone());
        let agent = match k.in_agent {
            Some(true) => " · in the SSH agent",
            Some(false) => " · not in the SSH agent",
            None => "",
        };
        col = col.child(
            kit::card(t, SharedString::from(format!("repo-key-{sid}")), used)
                .child(kit::row().child(kit::strong(t, k.name.clone()).w_auto().flex_none()).child(kit::small(t, format!("{} {}{agent}", k.kind, k.bits)).flex_1()))
                .child(kit::hint(t, k.fingerprint.clone()))
                .when(!k.comment.is_empty(), |el| el.child(kit::hint(t, k.comment.clone())))
                .child(
                    kit::row()
                        .pt(px(4.))
                        .child(kit::act(t, format!("repo-key-{sid}-copy"), "Copy public key", ButtonKind::Secondary, false).on_click(cx.listener(move |v, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(pk.clone()));
                            v.note = Some((true, format!("Copied the public key {name}.")));
                            cx.notify();
                        })))
                        .child(if used {
                            kit::small(t, "This repo already signs with this key").into_any_element()
                        } else {
                            kit::act(t, format!("repo-key-{sid}-sign"), "Use for signing", ButtonKind::Secondary, v.busy.is_some())
                                .on_click(cx.listener(move |v, _, _, cx| use_key(v, &k2, cx)))
                                .into_any_element()
                        }),
                ),
        );
    }
    col.child(kit::hint(t, "To use a key on GitHub, copy it and add it there. Add it as an Authentication key to push over SSH, and as a Signing key so your signed commits show Verified."))
        .child(kit::row().pt(px(4.)).child(kit::act(t, "repo-keys-github", "Open GitHub SSH keys", ButtonKind::Secondary, false).on_click(|_, _, cx| cx.open_url(GITHUB_KEYS_URL))))
        .into_any_element()
}

impl Render for RepoTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let app = super::accounts_tab::APP;
        let stuck = stuck_items(self, &t, cx);
        let identity = identity_block(self, &t, cx);
        let signing = signing_block(self, &t, cx);
        let remotes = remotes_block(self, &t, cx);
        let subs = submodules_block(self, &t, cx);
        let keys = keys_block(self, &t, cx);
        let mut pulls = div().flex().flex_col();
        for (id, label, detail) in PULL_MODES {
            pulls = pulls.child(kit::radio(&t, format!("repo-pull-{id}"), self.prefs.pull_mode == *id, *label, detail).on_click(cx.listener(move |v, _, _, cx| set_pref(v, PULL_MODE_KEY, Value::String(id.to_string()), cx))));
        }
        let mut fetch = kit::wrap_row();
        for m in FETCH_MINUTES {
            let m = *m;
            fetch = fetch.child(kit::pill(&t, format!("repo-fetch-{m}"), fetch_label(m), self.prefs.auto_fetch_minutes == m, false).on_click(cx.listener(move |v, _, _, cx| set_pref(v, AUTO_FETCH_KEY, Value::from(m), cx))));
        }
        let speak = self.speak;
        let name = kit::repo_name(&self.root.display().to_string());
        kit::scroll_body("repo-tab")
            .child(
                kit::row()
                    .child(kit::strong(&t, name))
                    .child(kit::small(&t, kit::short_path(&self.root.display().to_string(), &self.ctx.env.home)))
                    .child(div().flex_1())
                    .when(self.loading, |el| el.child(kit::busy_line(&t, "repo-loading", "")))
                    .child(kit::act(&t, "repo-refresh", "Refresh", ButtonKind::Secondary, self.loading).on_click(cx.listener(|v, _, _, cx| load(v, cx)))),
            )
            .when_some(self.note.clone(), |el, (ok, text)| el.child(if ok { kit::ok_text(&t, text) } else { kit::error_text(&t, text) }))
            .when_some(stuck, |el, s| el.child(s))
            .child(section(&t, "Identity"))
            .child(identity)
            .child(section(&t, "Signing"))
            .child(signing)
            .child(section(&t, "Remotes"))
            .child(remotes)
            .when_some(subs, |el, s| el.child(section(&t, "Submodules")).child(s))
            .child(section(&t, "Pull mode (all repos)"))
            .child(pulls)
            .child(section(&t, "Auto fetch (all repos)"))
            .child(fetch)
            .child(section(&t, "Git voice"))
            .child(kit::check(&t, "repo-speak", speak, "Speak git results and failures aloud").on_click(cx.listener(move |v, _, _, cx| set_pref(v, SPEAK_KEY, Value::Bool(!speak), cx))))
            .child(kit::hint(&t, "Popups still appear when this is off. The main Voice switch turns every spoken alert off."))
            .child(section(&t, "SSH keys on this computer"))
            .child(keys)
            .child(section(&t, "Remove setup"))
            .child(kit::hint(&t, format!("Removes the profile link and the sign-in helper from this repo's git config. Your name and email in this repo stay. {app} will not set up this repo again until you pick a profile for it.")))
            .child(kit::row().pt(px(4.)).child(kit::act(&t, "repo-setup-remove", format!("Remove {app} setup from this repo"), ButtonKind::Danger, self.busy.is_some()).on_click(cx.listener(open_confirm("setup")))))
            .child(ask(self, &t, "setup", "Remove the setup from this repo?".into(), "Remove", remove_setup, cx))
    }
}

