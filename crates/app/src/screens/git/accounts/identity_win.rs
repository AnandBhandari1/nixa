use super::{kit, profiles_tab};
use crate::screens::git::{changes::git, history};
use gpui::prelude::*;
use gpui::{AnyWindowHandle, App, Context, FocusHandle, Focusable, KeyDownEvent, Window, px};
use nixa_git::accounts::profiles::{Profile, load_profiles};
use nixa_ui::{Theme, button::ButtonKind, text_input::TextInput, theme};
use std::path::PathBuf;

type Resume = Box<dyn FnOnce(&mut App)>;

struct IdentityView {
    ctx: kit::Ctx,
    window: AnyWindowHandle,
    repo: PathBuf,
    focus: FocusHandle,
    name: gpui::Entity<TextInput>,
    email: gpui::Entity<TextInput>,
    profiles: Option<Vec<Profile>>,
    error: Option<String>,
    busy: bool,
    resume: Option<Resume>,
}

impl IdentityView {
    fn new(repo: PathBuf, resume: Resume, window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        let ctx = kit::real();
        let name = kit::input("Name for commits", "", false, cx);
        let email = kit::input("Email for commits", "", false, cx);
        let env = ctx.env.clone();
        kit::job(
            cx,
            move || load_profiles(&env),
            |view: &mut Self, profiles, _| {
                view.profiles = Some(profiles);
            },
        );
        Self {
            ctx,
            window,
            repo,
            focus: cx.focus_handle(),
            name,
            email,
            profiles: None,
            error: None,
            busy: false,
            resume: Some(resume),
        }
    }

    fn save(&mut self, profile: Option<Profile>, cx: &mut Context<Self>) {
        if self.busy || self.resume.is_none() {
            return;
        }
        let name = kit::value(&self.name, cx);
        let email = kit::value(&self.email, cx);
        self.error = None;
        self.busy = true;
        let repo = self.repo.clone();
        let env = self.ctx.env.clone();
        kit::job(
            cx,
            move || {
                if let Some(profile) = profile {
                    return profiles_tab::apply_profile(&env, &repo, Some(&profile)).map(|_| ());
                }
                git::set_identity(&repo.to_string_lossy(), name.trim(), email.trim())
            },
            |view, result, cx| {
                view.busy = false;
                match result {
                    Ok(()) => {
                        if let Some(resume) = view.resume.take() {
                            let handle = view.window;
                            cx.defer(move |cx| {
                                handle
                                    .update(cx, |_, window, _| window.remove_window())
                                    .ok();
                                resume(cx);
                            });
                        }
                    }
                    Err(err) => view.error = Some(err),
                }
            },
        );
        cx.notify();
    }

    fn profile_list(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        match self.profiles.as_deref() {
            Some([]) => kit::hint(
                t,
                "No saved profiles. Enter a name and email for this repo.",
            )
            .into_any_element(),
            Some(profiles) => {
                let view = cx.weak_entity();
                profiles_tab::picker(
                    t,
                    "identity-window-profile",
                    profiles,
                    None,
                    self.busy,
                    move |profile, cx| {
                        view.update(cx, |view, cx| view.save(profile, cx)).ok();
                    },
                )
            }
            None => {
                kit::busy_line(t, "identity-window-loading", "Loading profiles…").into_any_element()
            }
        }
    }
}

impl Focusable for IdentityView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for IdentityView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let profile_list = self.profile_list(&t, cx);
        kit::scroll_body("identity-window")
            .track_focus(&self.focus)
            .p(px(16.))
            .gap(px(10.))
            .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !view.busy {
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .child(kit::strong(&t, "Pick a profile for this repo"))
            .child(kit::hint(&t, self.repo.display().to_string()))
            .child(profile_list)
            .child(kit::field(&self.name, "identity-window-name"))
            .child(kit::field(&self.email, "identity-window-email"))
            .when_some(self.error.clone(), |el, err| {
                el.child(kit::error_text(&t, err))
            })
            .child(
                kit::row()
                    .child(
                        kit::act(
                            &t,
                            "identity-window-use",
                            "Use name and email",
                            ButtonKind::Primary,
                            self.busy,
                        )
                        .on_click(cx.listener(|view, _, _, cx| view.save(None, cx))),
                    )
                    .child(
                        kit::act(
                            &t,
                            "identity-window-cancel",
                            "Cancel",
                            ButtonKind::Secondary,
                            self.busy,
                        )
                        .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}

pub fn open(repo: PathBuf, resume: impl FnOnce(&mut App) + 'static, cx: &mut App) {
    cx.defer(move |cx| {
        history::open_window("Git identity", 500., 460., cx, move |window, cx| {
            cx.new(|cx| IdentityView::new(repo, Box::new(resume), window.window_handle(), cx))
        });
    });
}
