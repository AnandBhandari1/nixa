use super::accounts_tab::{self, AccountsTab};
use super::ai_tab::{self, AiDeps, AiTab};
use super::clone_win::{self, CloneView, Phase as ClonePhase};
use super::kit::{self, Ctx};
use super::pr_win::{self, Phase as PrPhase, PrDeps, PrView};
use super::profiles_tab::{self, ProfilesTab};
use super::repo_tab::{self, RepoTab};
use crate::screens::settings::parts::git_accounts::{self, GitAccounts, Tab};
use crate::screens::settings::parts::memory::text_area::TextArea;
use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use nixa_git::accounts::profiles::{Profile, load_profiles, save_profiles, stamp_repo};
use nixa_git::accounts::{Env, add_account, load_accounts};
use nixa_git::ai::{AiError, AiSettings, Answer, ErrorKind, Found, Opts, PrText, SETTINGS_KEY};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const SECURITY: &str = r#"#!/bin/sh
d=$(dirname "$0")/..
store="$d/keychain"
touch "$store"
printf '%s\n' "$1" >> "$d/security.log"
field() { printf '%s' "$1" | sed -n "s/.* $2 \"\([^\"]*\)\".*/\1/p"; }
arg() { want=$1; shift; while [ $# -gt 0 ]; do if [ "$1" = "$want" ]; then printf '%s' "$2"; return; fi; shift; done; }
case "$1" in
  -i)
    while IFS= read -r line; do
      case "$line" in
        add-generic-password*)
          s=$(field "$line" -s); a=$(field "$line" -a); w=$(field "$line" -w)
          grep -v "^$s	$a	" "$store" > "$store.new"; mv "$store.new" "$store"
          printf '%s\t%s\t%s\n' "$s" "$a" "$w" >> "$store" ;;
      esac
    done
    exit 0 ;;
  find-generic-password)
    s=$(arg -s "$@"); a=$(arg -a "$@")
    hit=$(grep "^$s	$a	" "$store" | head -n 1)
    [ -n "$hit" ] || exit 44
    case " $* " in *" -w "*) printf '%s\n' "$hit" | cut -f3 ;; *) echo "keychain item" ;; esac
    exit 0 ;;
  delete-generic-password)
    s=$(arg -s "$@"); a=$(arg -a "$@")
    grep -q "^$s	$a	" "$store" || exit 44
    grep -v "^$s	$a	" "$store" > "$store.new"; mv "$store.new" "$store"
    exit 0 ;;
esac
exit 1
"#;

const GH: &str = r#"#!/bin/sh
d=$(dirname "$0")/../gh
mkdir -p "$d"
if [ "$1 $2" = "auth token" ]; then exit 1; fi
if [ "$1 $2" = "auth status" ]; then
  [ -f "$d/status.json" ] || exit 1
  cat "$d/status.json"; exit 0
fi
printf '%s|GH_TOKEN=%s\n' "$*" "$GH_TOKEN" >> "$d/calls.log"
key="$1-$2"
[ -f "$d/err-$key" ] && cat "$d/err-$key" >&2
[ -f "$d/reply-$key" ] && cat "$d/reply-$key"
[ -f "$d/code-$key" ] && exit "$(cat "$d/code-$key")"
exit 0
"#;

const CURL: &str = r#"#!/bin/sh
d=$(dirname "$0")/../curl
mkdir -p "$d"
header=$(cat)
tok=$(printf '%s' "$header" | sed -n 's/^Authorization: Bearer //p')
for url; do :; done
case "$url" in *"/user/repos"*) f="$d/$tok.repos" ;; *) f="$d/$tok" ;; esac
if [ -f "$f" ]; then cat "$f"; exit 0; fi
echo "curl: (7) Failed to connect to host" >&2
exit 7
"#;

struct Kit {
    root: PathBuf,
    ctx: Ctx,
}

impl Drop for Kit {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn kit(name: &str) -> Kit {
    let root = std::env::temp_dir().join(format!("mga-{}-{name}", std::process::id()));
    fs::remove_dir_all(&root).ok();
    let bin = root.join("bin");
    let home = root.join("home");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(home.join(".gitconfig"), "").unwrap();
    script(&bin.join("security"), SECURITY);
    script(&bin.join("gh"), GH);
    script(&bin.join("curl"), CURL);
    let env = Env {
        dir: root.join("data").join("git"),
        home: home.clone(),
        service: "Nixa Test".into(),
        security: bin.join("security"),
        curl: bin.join("curl"),
        git: PathBuf::from("git"),
        gh: Some(bin.join("gh")),
        git_env: vec![
            ("HOME".into(), home.display().to_string()),
            ("GIT_CONFIG_GLOBAL".into(), home.join(".gitconfig").display().to_string()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ],
    };
    let ctx = Ctx { env, db: root.join("data").join("nixa.db") };
    Kit { root, ctx }
}

impl Kit {
    fn file(&self, dir: &str, name: &str, body: &str) {
        let d = self.root.join(dir);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(name), body).unwrap();
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap_or_default()
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(dir).envs(self.ctx.env.git_env.iter().cloned()).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q", "-b", "main"]);
        self.git(&path, &["-c", "user.name=T", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", "one"]);
        path
    }

    fn user(&self, token: &str, login: &str) {
        let reply = format!(
            "HTTP/2 200\r\ncontent-type: application/json\r\nx-oauth-scopes: repo, read:org, workflow\r\n\r\n{{\"login\":\"{login}\",\"name\":\"Name {login}\",\"avatar_url\":\"https://a/{login}\"}}"
        );
        self.file("curl", token, &reply);
    }

    fn profile(&self, id: &str, user: &str) -> Profile {
        let mut accounts = BTreeMap::new();
        accounts.insert("github.com".to_string(), user.to_string());
        let p = Profile {
            id: id.into(),
            label: id.into(),
            name: format!("Name {id}"),
            email: format!("{id}@example.com"),
            accounts,
            folders: Vec::new(),
            owners: Vec::new(),
            color: "info".into(),
        };
        let mut all = load_profiles(&self.ctx.env);
        all.push(p.clone());
        save_profiles(&self.ctx.env, &all).unwrap();
        p
    }
}

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        nixa_ui::init(cx);
        crate::screens::settings::parts::memory::text_area::bind_keys(cx);
    });
}

fn settle<V: 'static>(vcx: &mut VisualTestContext, view: &Entity<V>, done: impl Fn(&V) -> bool) {
    settle_as(vcx, view, "view", done);
}

fn settle_as<V: 'static>(vcx: &mut VisualTestContext, view: &Entity<V>, what: &str, done: impl Fn(&V) -> bool) {
    let began = Instant::now();
    loop {
        vcx.run_until_parked();
        if view.read_with(vcx, |v, _| done(v)) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(20), "{what} never settled");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click(vcx: &mut VisualTestContext, id: &str) {
    let at = vcx.debug_bounds(Box::leak(id.to_string().into_boxed_str())).unwrap_or_else(|| panic!("{id} is not on screen")).center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn type_in(vcx: &mut VisualTestContext, input: &Entity<nixa_ui::text_input::TextInput>, text: &str) {
    vcx.update(|window, cx| {
        input.update(cx, |i, cx| i.set_value("", cx));
        window.focus(&input.focus_handle(cx));
    });
    vcx.run_until_parked();
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

fn area_in(vcx: &mut VisualTestContext, area: &Entity<TextArea>, text: &str) {
    vcx.update(|window, cx| {
        area.update(cx, |a, cx| a.set_value("", cx));
        window.focus(&area.focus_handle(cx));
    });
    vcx.run_until_parked();
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

#[gpui::test]
fn adding_a_token_keeps_it_only_in_the_keychain(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("add");
    k.user("tok-one", "alice");
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| accounts_tab::build(ctx, cx));
    settle(vcx, &view, |v: &AccountsTab| !v.loading);
    click(vcx, "acct-add");
    assert!(view.read_with(vcx, |v, _| v.add.open));
    let token = view.read_with(vcx, |v, _| v.add_token.clone());
    type_in(vcx, &token, "tok-one");
    click(vcx, "acct-add-save");
    settle(vcx, &view, |v: &AccountsTab| !v.add.busy && !v.accounts.is_empty());
    let (names, note) = view.read_with(vcx, |v, _| (v.accounts.iter().map(|a| a.username.clone()).collect::<Vec<_>>(), v.note.clone()));
    assert_eq!(names, ["alice"]);
    assert_eq!(note, Some((true, "Added @alice.".to_string())));
    assert!(k.read("keychain").contains("tok-one"));
    let saved = fs::read_to_string(nixa_git::accounts::accounts_file(&k.ctx.env)).unwrap();
    assert!(saved.contains("alice") && !saved.contains("tok-one"));
    assert!(vcx.debug_bounds("acct-github_com_alice-reconnect").is_some());
}

#[gpui::test]
fn removing_an_account_can_delete_its_token(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("remove");
    k.user("tok-two", "bob");
    add_account(&k.ctx.env, "github.com", "tok-two", None).unwrap();
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| accounts_tab::build(ctx, cx));
    settle(vcx, &view, |v: &AccountsTab| !v.loading && !v.checking_all);
    click(vcx, "acct-github_com_bob-remove");
    assert!(vcx.debug_bounds("acct-github_com_bob-remove-yes").is_some());
    assert!(view.read_with(vcx, |v, _| v.remove_token));
    click(vcx, "acct-github_com_bob-remove-yes");
    settle(vcx, &view, |v: &AccountsTab| v.busy.is_empty() && v.accounts.is_empty());
    assert!(load_accounts(&k.ctx.env).is_empty());
    assert!(!k.read("keychain").contains("tok-two"));
}

#[gpui::test]
fn there_is_no_gh_import(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("gh");
    k.file("gh", "status.json", r#"{"hosts":{"github.com":[{"login":"carol","active":true,"state":"success"}]}}"#);
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| accounts_tab::build(ctx, cx));
    settle(vcx, &view, |v: &AccountsTab| !v.loading);
    assert!(vcx.debug_bounds("acct-add").is_some());
    assert!(vcx.debug_bounds("acct-import").is_none());
    assert!(load_accounts(&k.ctx.env).is_empty());
}

#[gpui::test]
fn the_try_other_accounts_switch_is_saved(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("try");
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| accounts_tab::build(ctx, cx));
    settle(vcx, &view, |v: &AccountsTab| !v.loading);
    assert!(view.read_with(vcx, |v, _| v.try_others));
    click(vcx, "acct-try-others");
    assert!(!view.read_with(vcx, |v, _| v.try_others));
    let stored = kit::get_setting(&k.ctx.db, nixa_git::accounts::settings::TRY_OTHERS_KEY);
    assert_eq!(stored, Some(serde_json::Value::Bool(false)));
}

#[gpui::test]
fn a_new_profile_is_saved_and_can_be_deleted(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("prof");
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| profiles_tab::build(ctx, None, cx));
    settle(vcx, &view, |v: &ProfilesTab| !v.loading);
    click(vcx, "prof-new");
    let (label, name, email, folders) = view.read_with(vcx, |v, _| (v.label.clone(), v.name.clone(), v.email.clone(), v.folders.clone()));
    click(vcx, "prof-save");
    assert!(view.read_with(vcx, |v, _| v.form_error.is_some()));
    type_in(vcx, &label, "Work");
    type_in(vcx, &name, "Pat Doe");
    type_in(vcx, &email, "pat@example.com");
    area_in(vcx, &folders, "~/Projects/work");
    click(vcx, "prof-save");
    settle(vcx, &view, |v: &ProfilesTab| !v.busy && !v.loading && v.editing.is_none());
    let saved = load_profiles(&k.ctx.env);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].label, "Work");
    assert_eq!(saved[0].folders, ["~/Projects/work"]);
    let id = kit::safe_id(&saved[0].id);
    click(vcx, &format!("prof-{id}-delete"));
    click(vcx, &format!("prof-{id}-delete-yes"));
    settle(vcx, &view, |v: &ProfilesTab| !v.busy && !v.loading && v.profiles.is_empty());
    assert!(load_profiles(&k.ctx.env).is_empty());
}

#[gpui::test]
fn a_repo_takes_a_profile_from_the_picker(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("pick");
    let repo = k.repo("app");
    k.profile("home", "dana");
    let ctx = k.ctx.clone();
    let root = repo.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| profiles_tab::build(ctx, Some(root), cx));
    settle(vcx, &view, |v: &ProfilesTab| !v.loading);
    click(vcx, "prof-repo-change");
    click(vcx, "prof-pick-home");
    settle(vcx, &view, |v: &ProfilesTab| !v.busy && !v.loading && v.identity.as_ref().is_some_and(|i| i.profile_id.is_some()));
    assert_eq!(k.git(&repo, &["config", "--local", "user.email"]), "home@example.com");
    assert_eq!(k.git(&repo, &["config", "--local", "nixa.profile"]), "home");
}

fn fake_ai() -> AiDeps {
    AiDeps {
        detect: |_| {
            vec![
                Found { id: "claude".into(), label: "Claude Code".into(), installed: true, path: "/fake/claude".into() },
                Found { id: "codex".into(), label: "Codex CLI".into(), installed: true, path: "/fake/codex".into() },
                Found { id: "omp".into(), label: "Oh My Pi".into(), installed: false, path: String::new() },
            ]
        },
        ping: |s: &AiSettings, _: &Opts| Ok(Answer { text: "pong".into(), agent: s.agent.clone(), label: "Codex CLI".into(), model: String::new() }),
        ollama: |_| Vec::new(),
        models: |_, _| Ok(Default::default()),
    }
}

#[gpui::test]
fn the_ai_tab_saves_its_settings_in_the_store(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("ai");
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| ai_tab::build(ctx, fake_ai(), None, cx));
    settle(vcx, &view, |v: &AiTab| !v.looking);
    click(vcx, "ai-agent-codex");
    click(vcx, "ai-style-plain");
    let saved = AiSettings::from_value(kit::get_setting(&k.ctx.db, SETTINGS_KEY).as_ref());
    assert_eq!(saved.agent, "codex");
    assert_eq!(saved.style.preset, "plain");
    let prefix = view.read_with(vcx, |v, _| v.prefix.clone());
    area_in(vcx, &prefix, "not a rule");
    assert_eq!(view.read_with(vcx, |v, _| v.bad_lines.clone()), [1]);
    area_in(vcx, &prefix, "feature/* => [{suffix}]");
    assert!(view.read_with(vcx, |v, _| v.bad_lines.is_empty()));
    let saved = AiSettings::from_value(kit::get_setting(&k.ctx.db, SETTINGS_KEY).as_ref());
    assert_eq!(saved.style.prefix_rules.len(), 1);
    let timeout = view.read_with(vcx, |v, _| v.timeout.clone());
    type_in(vcx, &timeout, "5");
    assert!(view.read_with(vcx, |v, _| v.timeout_error));
    type_in(vcx, &timeout, "90");
    assert!(!view.read_with(vcx, |v, _| v.timeout_error));
    click(vcx, "ai-test");
    settle(vcx, &view, |v: &AiTab| v.testing.is_none() && v.test_result.is_some());
    assert_eq!(view.read_with(vcx, |v, _| v.test_result.clone()), Some((true, "Codex CLI answered: pong".to_string())));
    let saved = AiSettings::from_value(kit::get_setting(&k.ctx.db, SETTINGS_KEY).as_ref());
    assert_eq!(saved.timeout_sec, 90);
}

#[gpui::test]
fn the_repo_tab_adds_a_remote_and_saves_shared_prefs(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("repo");
    let repo = k.repo("site");
    let ctx = k.ctx.clone();
    let root = repo.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| repo_tab::build(ctx, root, cx));
    settle(vcx, &view, |v: &RepoTab| !v.loading);
    let (name, url) = view.read_with(vcx, |v, _| (v.new_name.clone(), v.new_url.clone()));
    type_in(vcx, &name, "origin");
    type_in(vcx, &url, "https://github.com/owner/site.git");
    vcx.update(|_, cx| view.update(cx, repo_tab::add_remote));
    settle(vcx, &view, |v: &RepoTab| v.busy.is_none() && !v.loading);
    assert_eq!(k.git(&repo, &["remote", "get-url", "origin"]), "https://github.com/owner/site.git");
    assert_eq!(view.read_with(vcx, |v, _| v.note.clone()), Some((true, "Added origin.".to_string())));
    type_in(vcx, &name, "bad name");
    type_in(vcx, &url, "https://github.com/owner/x.git");
    vcx.update(|_, cx| view.update(cx, repo_tab::add_remote));
    assert_eq!(
        view.read_with(vcx, |v, _| v.note.clone()),
        Some((false, "Use only letters, numbers, dot, dash or underscore in the name.".to_string()))
    );
    vcx.update(|_, cx| view.update(cx, |v, cx| repo_tab::set_pref(v, nixa_git::accounts::settings::PULL_MODE_KEY, "merge".into(), cx)));
    assert_eq!(view.read_with(vcx, |v, _| v.prefs.pull_mode.clone()), "merge");
    assert!(vcx.debug_bounds("repo-remote-origin-edit").is_some());
}

#[gpui::test]
fn remove_setup_marks_the_repo_unmanaged(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("setup");
    let repo = k.repo("tool");
    let p = k.profile("work", "erin");
    stamp_repo(&k.ctx.env, &repo, &p, false).unwrap();
    let ctx = k.ctx.clone();
    let root = repo.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| repo_tab::build(ctx, root, cx));
    settle(vcx, &view, |v: &RepoTab| !v.loading);
    vcx.update(|_, cx| view.update(cx, repo_tab::remove_setup));
    settle(vcx, &view, |v: &RepoTab| v.busy.is_none() && !v.loading);
    assert_eq!(k.git(&repo, &["config", "--local", "nixa.unmanaged"]), "true");
    assert_eq!(k.git(&repo, &["config", "--local", "user.email"]), "work@example.com");
}

#[gpui::test]
fn clone_copies_a_local_repo_with_the_picked_profile(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("clone");
    let source = k.repo("source");
    k.profile("me", "fay");
    let ctx = k.ctx.clone();
    let into = k.root.join("out").display().to_string();
    let (view, vcx) = cx.add_window_view(move |_, cx| clone_win::build(ctx, None, Some(into), cx));
    settle(vcx, &view, |v: &CloneView| v.loaded);
    let link = view.read_with(vcx, |v, _| v.link.clone());
    type_in(vcx, &link, &source.display().to_string());
    let folder = view.read_with(vcx, |v, cx| kit::value(&v.folder, cx));
    assert_eq!(folder, k.root.join("out").join("source").display().to_string());
    assert!(view.read_with(vcx, clone_win::ready));
    click(vcx, "clone-start");
    settle(vcx, &view, |v: &CloneView| v.phase != ClonePhase::Busy);
    assert_eq!(view.read_with(vcx, |v, _| v.phase), ClonePhase::Done, "{:?}", view.read_with(vcx, |v, _| v.error.clone()));
    let made = k.root.join("out").join("source");
    assert_eq!(k.git(&made, &["config", "--local", "user.email"]), "me@example.com");
    assert!(vcx.debug_bounds("clone-open").is_some());
}

#[gpui::test]
fn clone_needs_a_profile_first(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("noprof");
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| clone_win::build(ctx, Some("owner/repo".into()), None, cx));
    settle(vcx, &view, |v: &CloneView| v.loaded);
    assert!(view.read_with(vcx, |v, _| v.parsed.ok));
    assert!(!view.read_with(vcx, clone_win::ready));
    assert!(vcx.debug_bounds("clone-profiles").is_some());
}

fn fake_pr() -> PrDeps {
    PrDeps {
        ai: |_, _, _, _, _| Ok(PrText { title: "Add the login page".into(), body: "- Adds a page".into(), agent: "claude".into() }),
    }
}

fn pr_repo(k: &Kit) -> PathBuf {
    let repo = k.repo("web");
    k.git(&repo, &["remote", "add", "origin", "https://github.com/acme/web.git"]);
    k.git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    k.git(&repo, &["checkout", "-q", "-b", "feature/login-page"]);
    k.git(&repo, &["update-ref", "refs/remotes/origin/feature/login-page", "HEAD"]);
    k.git(&repo, &["config", "branch.feature/login-page.remote", "origin"]);
    k.git(&repo, &["config", "branch.feature/login-page.merge", "refs/heads/feature/login-page"]);
    k.user("tok-pr", "gail");
    add_account(&k.ctx.env, "github.com", "tok-pr", None).unwrap();
    let p = k.profile("work", "gail");
    stamp_repo(&k.ctx.env, &repo, &p, false).unwrap();
    k.file("gh", "reply-repo-view", "main\n");
    k.file("gh", "err-pr-view", "no pull requests found for branch \"feature/login-page\"\n");
    k.file("gh", "code-pr-view", "1");
    repo
}

#[gpui::test]
fn a_pull_request_is_made_with_gh_and_the_keychain_token(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("pr");
    let repo = pr_repo(&k);
    k.file("gh", "reply-pr-create", "https://github.com/acme/web/pull/7\n");
    let ctx = k.ctx.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| pr_win::build(ctx, fake_pr(), Some(repo), cx));
    settle_as(vcx, &view, "pr load", |v: &PrView| v.phase != PrPhase::Loading && !v.checking);
    assert_eq!(view.read_with(vcx, |v, _| v.phase), PrPhase::Form, "{:?}", view.read_with(vcx, |v, _| v.error.clone()));
    let (title, base) = view.read_with(vcx, |v, cx| (kit::value(&v.title, cx), v.base.clone()));
    assert_eq!(base, "main");
    assert!(!title.is_empty());
    click(vcx, "pr-ai");
    settle_as(vcx, &view, "pr ai", |v: &PrView| v.ai_cancel.is_none());
    assert_eq!(view.read_with(vcx, |v, cx| kit::value(&v.title, cx)), "Add the login page");
    click(vcx, "pr-create");
    settle_as(vcx, &view, "pr create", |v: &PrView| !v.busy);
    assert_eq!(view.read_with(vcx, |v, _| v.phase), PrPhase::Done, "{:?}", view.read_with(vcx, |v, _| v.error.clone()));
    assert_eq!(view.read_with(vcx, |v, _| v.created.as_ref().and_then(|o| o.number)), Some(7));
    let calls = k.read("gh/calls.log");
    assert!(calls.contains("pr create") && calls.contains("GH_TOKEN=tok-pr"), "{calls}");
    assert!(vcx.debug_bounds("pr-open").is_some());
}

#[gpui::test]
fn the_pr_window_says_when_gh_is_missing(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("nogh");
    let repo = k.repo("x");
    let mut ctx = k.ctx.clone();
    ctx.env.gh = None;
    let (view, vcx) = cx.add_window_view(move |_, cx| pr_win::build(ctx, fake_pr(), Some(repo), cx));
    settle(vcx, &view, |v: &PrView| v.phase != PrPhase::Loading);
    assert_eq!(view.read_with(vcx, |v, _| v.phase), PrPhase::Failed);
    assert_eq!(view.read_with(vcx, |v, _| v.error.as_ref().map(|e| e.kind.clone())), Some("noGh".to_string()));
    assert!(vcx.debug_bounds("pr-retry").is_some());
}

#[gpui::test]
fn the_part_shows_this_repo_only_for_a_repo(cx: &mut TestAppContext) {
    setup(cx);
    let k = kit("part");
    let repo = k.repo("proj");
    let plain = k.root.join("plain");
    fs::create_dir_all(&plain).unwrap();
    let ctx = k.ctx.clone();
    let folder = plain.display().to_string();
    let (view, vcx) = cx.add_window_view(move |_, cx| git_accounts::build(ctx, fake_ai(), Some(folder), None, cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("git-tab-accounts").is_some());
    assert!(view.read_with(vcx, |v, _| v.repo.is_none()));
    assert!(vcx.debug_bounds("git-tab-repo").is_none());
    let ctx = k.ctx.clone();
    let folder = repo.display().to_string();
    let (view, vcx) = cx.add_window_view(move |_, cx| git_accounts::build(ctx, fake_ai(), Some(folder), None, cx));
    vcx.run_until_parked();
    click(vcx, "git-tab-repo");
    assert_eq!(view.read_with(vcx, |v, _| v.tab), Tab::Repo);
    assert!(view.read_with(vcx, |v, _| v.this_repo.is_some()));
    click(vcx, "git-tab-profiles");
    assert_eq!(view.read_with(vcx, |v, _| v.tab), Tab::Profiles);
    click(vcx, "git-tab-ai");
    assert!(view.read_with(vcx, |v: &GitAccounts, _| v.ai.is_some()));
}

#[test]
fn start_tab_names_are_read() {
    assert_eq!(git_accounts::tab_named("AI"), Some(Tab::Ai));
    assert_eq!(git_accounts::tab_named("this repo"), Some(Tab::Repo));
    assert_eq!(git_accounts::tab_named("nope"), None);
    assert!(repo_tab::name_ok("up-stream.2"));
    assert!(!repo_tab::name_ok("-x"));
    assert_eq!(clone_win::age(None, 0), "");
    let _ = (ErrorKind::Failed, AiError::new(ErrorKind::Failed, "x", None));
}

#[test]
fn pr_read_distinguishes_a_non_repo_from_a_detached_head() {
    let k = kit("pr-non-repo");
    let outside = pr_win::read_start(&k.ctx.env, Some(&k.root));
    assert_eq!(outside.err().unwrap().kind, "noRepo");
    let repo = k.repo("detached");
    k.git(&repo, &["checkout", "-q", "--detach"]);
    let detached = pr_win::read_start(&k.ctx.env, Some(&repo));
    assert_eq!(detached.err().unwrap().title, "You are not on a branch");
}
