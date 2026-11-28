use crate::screens::git::accounts::accounts_tab::{self, AccountsTab};
use crate::screens::git::accounts::ai_tab::{self, AiDeps, AiTab};
use crate::screens::git::accounts::kit::{self, Ctx};
use crate::screens::git::accounts::profiles_tab::{self, ProfilesTab};
use crate::screens::git::accounts::repo_tab::{self, RepoTab};
use crate::screens::git::accounts::{SETTINGS_PART, clone_win};
use gpui::prelude::*;
use gpui::{App, Context, Entity, Window, div, px};
use nixa_ui::button::ButtonKind;
use nixa_ui::tab::{segment, segmented};
use nixa_ui::theme;
use std::path::{Path, PathBuf};

pub const PART: &str = SETTINGS_PART;
pub const TAB_ENV: &str = "NIXA_APP_GIT_TAB";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Accounts,
    Profiles,
    Ai,
    Repo,
}

pub const TABS: &[(Tab, &str, &str)] = &[
    (Tab::Accounts, "git-tab-accounts", "Accounts"),
    (Tab::Profiles, "git-tab-profiles", "Profiles"),
    (Tab::Ai, "git-tab-ai", "AI"),
    (Tab::Repo, "git-tab-repo", "This repo"),
];

pub struct GitAccounts {
    pub ctx: Ctx,
    pub ai_deps: AiDeps,
    pub repo: Option<PathBuf>,
    pub tab: Tab,
    pub reason: Option<String>,
    pub accounts: Option<Entity<AccountsTab>>,
    pub profiles: Option<Entity<ProfilesTab>>,
    pub ai: Option<Entity<AiTab>>,
    pub this_repo: Option<Entity<RepoTab>>,
}

pub fn tab_named(name: &str) -> Option<Tab> {
    match name.trim().to_lowercase().as_str() {
        "accounts" => Some(Tab::Accounts),
        "profiles" => Some(Tab::Profiles),
        "ai" => Some(Tab::Ai),
        "repo" | "this repo" => Some(Tab::Repo),
        _ => None,
    }
}

pub fn repo_root(ctx: &Ctx, folder: &str) -> Option<PathBuf> {
    let full = nixa_git::clone::expand_home(folder, &ctx.env.home);
    let dir = Path::new(&full);
    if !dir.is_dir() {
        return None;
    }
    nixa_git::accounts::helper::git_ok(&ctx.env, dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

pub fn new(folder: Option<String>, cx: &mut App) -> Entity<GitAccounts> {
    let ctx = kit::real();
    let start = std::env::var(TAB_ENV).ok().filter(|_| !cfg!(test)).and_then(|t| tab_named(&t));
    cx.new(|cx| build(ctx, ai_tab::real_deps(), folder, start, cx))
}

pub fn build(ctx: Ctx, ai_deps: AiDeps, folder: Option<String>, start: Option<Tab>, cx: &mut Context<GitAccounts>) -> GitAccounts {
    let repo = folder.as_deref().and_then(|f| repo_root(&ctx, f));
    let tab = match start {
        Some(Tab::Repo) if repo.is_none() => Tab::Accounts,
        Some(t) => t,
        None => Tab::Accounts,
    };
    let mut view = GitAccounts { ctx, ai_deps, repo, tab, reason: None, accounts: None, profiles: None, ai: None, this_repo: None };
    show(&mut view, tab, cx);
    view
}

pub fn open_for(folder: Option<String>, tab: &str, reason: Option<String>, cx: &mut App) {
    let tab = tab_named(tab).unwrap_or(Tab::Accounts);
    cx.defer(move |cx| {
        crate::screens::settings::open(PART, cx);
        let Some(window) = cx.windows().into_iter()
            .find_map(|window| window.downcast::<crate::screens::settings::SettingsView>())
        else { return };
        window.update(cx, |settings, _, cx| {
            let Some(part) = settings.kept.git_accounts.clone() else { return };
            part.update(cx, |v, cx| {
                let repo = folder.as_deref().and_then(|folder| repo_root(&v.ctx, folder));
                if repo != v.repo {
                    v.repo = repo;
                    v.profiles = None;
                    v.this_repo = None;
                    v.ai = None;
                }
                v.reason = reason;
                show(v, tab, cx);
            });
        }).ok();
    });
}

pub fn show(v: &mut GitAccounts, tab: Tab, cx: &mut Context<GitAccounts>) {
    v.tab = tab;
    match tab {
        Tab::Accounts => {
            if v.accounts.is_none() {
                v.accounts = Some(accounts_tab::new(v.ctx.clone(), cx));
            }
        }
        Tab::Profiles => match &v.profiles {
            Some(p) => p.update(cx, profiles_tab::load),
            None => v.profiles = Some(profiles_tab::new(v.ctx.clone(), v.repo.clone(), cx)),
        },
        Tab::Ai => {
            if v.ai.is_none() {
                v.ai = Some(ai_tab::new(v.ctx.clone(), v.ai_deps, v.repo.clone(), cx));
            }
        }
        Tab::Repo => match (&v.this_repo, &v.repo) {
            (Some(r), _) => r.update(cx, repo_tab::load),
            (None, Some(root)) => v.this_repo = Some(repo_tab::new(v.ctx.clone(), root.clone(), cx)),
            (None, None) => v.tab = Tab::Accounts,
        },
    }
    cx.notify();
}

impl Render for GitAccounts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let mut tabs = segmented(&t);
        for (tab, id, label) in TABS {
            if *tab == Tab::Repo && self.repo.is_none() {
                continue;
            }
            let tab = *tab;
            let tag = id.to_string();
            tabs = tabs.child(
                segment(&t, *id, *label, self.tab == tab)
                    .debug_selector(move || tag.clone())
                    .on_click(cx.listener(move |v, _, _, cx| show(v, tab, cx))),
            );
        }
        let into = self.repo.as_ref().and_then(|r| r.parent()).map(|p| p.display().to_string());
        let body = match self.tab {
            Tab::Accounts => self.accounts.clone().map(|e| e.into_any_element()),
            Tab::Profiles => self.profiles.clone().map(|e| e.into_any_element()),
            Tab::Ai => self.ai.clone().map(|e| e.into_any_element()),
            Tab::Repo => self.this_repo.clone().map(|e| e.into_any_element()),
        }
        .unwrap_or_else(|| div().into_any_element());
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                kit::row()
                    .flex_none()
                    .child(tabs)
                    .child(div().flex_1())
                    .child(kit::act(&t, "git-clone-open", "Clone a repo…", ButtonKind::Secondary, false).on_click(move |_, _, cx| clone_win::open(None, into.clone(), cx))),
            )
            .when_some(self.reason.clone(), |el, reason| el.child(kit::warn_text(&t, reason)))
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
    }
}
