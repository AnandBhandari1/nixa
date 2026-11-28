use super::{ChangesView, Popup, git, menus};
use crate::screens::git::{accounts, ai};
use crate::screens::settings::parts::git_accounts;
use gpui::{App, Context, Focusable, Pixels, Point, Window};
use nixa_git::accounts::{profiles::load_profiles, settings::prefs_from};

#[derive(Default)]
pub(super) struct PanelPrefs {
    pub mode: git::PullMode,
    pub auto_fetch_minutes: u32,
    pub profiles: Vec<menus::Profile>,
}

pub(super) fn load() -> PanelPrefs {
    let ctx = accounts::kit::real();
    let prefs = prefs_from(&accounts::kit::all_settings(&ctx.db));
    let mode = match prefs.pull_mode.as_str() {
        "merge" => git::PullMode::Merge,
        "ff-only" => git::PullMode::FfOnly,
        _ => git::PullMode::Rebase,
    };
    PanelPrefs {
        mode,
        auto_fetch_minutes: prefs.auto_fetch_minutes,
        profiles: load_profiles(&ctx.env),
    }
}

pub(super) fn profiles() -> Vec<menus::Profile> {
    load_profiles(&accounts::kit::real().env)
}

pub(super) fn open_settings(repo: Option<String>, tab: &str, reason: Option<String>, cx: &mut App) {
    git_accounts::open_for(repo, tab, reason, cx);
}

pub(super) fn open_agents(
    v: &mut ChangesView,
    at: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<ChangesView>,
) {
    let view = ai::new_menu(ai::model::real(), cx);
    let events = cx.subscribe_in(&view, window, |v, _, event, window, cx| {
        if *event == ai::menu::AgentMenuEvent::Settings {
            open_settings(v.root.clone(), "ai", None, cx);
        }
        v.popup = None;
        window.focus(&v.focus);
        cx.notify();
    });
    window.focus(&view.focus_handle(cx));
    v.popup = Some(Popup::Agents {
        at,
        view,
        _events: events,
    });
    cx.notify();
}
