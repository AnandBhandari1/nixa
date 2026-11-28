use super::kit::{self, Ctx};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Focusable, FontWeight, SharedString, Window, div, px};
use nixa_git::accounts::settings::TRY_OTHERS_KEY;
use nixa_git::accounts::{
    self as acc, Account, AddError, TOKEN_SCOPES, Tone, account_id, describe_status,
    expiry_text, checked_text, missing_scopes, remove_text, token_page_url,
};
use nixa_git::accounts::profiles::{Profile, load_profiles};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use serde_json::Value;
use std::collections::BTreeMap;

pub const HOST: &str = "github.com";
pub const APP: &str = "Nixa";

#[derive(Default)]
pub struct Add {
    pub open: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub needs_user: bool,
}

pub struct AccountsTab {
    pub ctx: Ctx,
    pub accounts: Vec<Account>,
    pub loading: bool,
    pub load_error: Option<String>,
    pub checking_all: bool,
    pub busy: BTreeMap<String, String>,
    pub errors: BTreeMap<String, String>,
    pub note: Option<(bool, String)>,
    pub add: Add,
    pub re_id: Option<String>,
    pub remove_id: Option<String>,
    pub remove_token: bool,
    pub remove_profiles: Vec<Profile>,
    pub try_others: bool,
    pub try_error: Option<String>,
    pub add_token: Entity<TextInput>,
    pub add_user: Entity<TextInput>,
    pub re_token: Entity<TextInput>,
}

pub fn new(ctx: Ctx, cx: &mut App) -> Entity<AccountsTab> {
    cx.new(|cx| build(ctx, cx))
}

pub fn build(ctx: Ctx, cx: &mut Context<AccountsTab>) -> AccountsTab {
    let add_token = kit::input("Paste a GitHub token", "", true, cx);
    let add_user = kit::input("GitHub user name of this token", "", false, cx);
    let re_token = kit::input("Paste the new token", "", true, cx);
    cx.subscribe(&add_token, |v, _, e: &InputEvent, cx| on_key(v, e, false, cx)).detach();
    cx.subscribe(&add_user, |v, _, e: &InputEvent, cx| on_key(v, e, false, cx)).detach();
    cx.subscribe(&re_token, |v, _, e: &InputEvent, cx| on_key(v, e, true, cx)).detach();
    let try_others = kit::get_setting(&ctx.db, TRY_OTHERS_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut view = AccountsTab {
        ctx,
        accounts: Vec::new(),
        loading: true,
        load_error: None,
        checking_all: false,
        busy: BTreeMap::new(),
        errors: BTreeMap::new(),
        note: None,
        add: Add::default(),
        re_id: None,
        remove_id: None,
        remove_token: true,
        remove_profiles: Vec::new(),
        try_others,
        try_error: None,
        add_token,
        add_user,
        re_token,
    };
    load(&mut view, true, cx);
    view
}

fn on_key(v: &mut AccountsTab, e: &InputEvent, re: bool, cx: &mut Context<AccountsTab>) {
    match e {
        InputEvent::Submit(_) if re => submit_reconnect(v, cx),
        InputEvent::Submit(_) => submit_add(v, cx),
        InputEvent::Cancel if re => {
            v.re_id = None;
            cx.notify();
        }
        InputEvent::Cancel => close_add(v, cx),
        InputEvent::Changed(_) => {}
    }
}

pub fn load(v: &mut AccountsTab, then_check: bool, cx: &mut Context<AccountsTab>) {
    let env = v.ctx.env.clone();
    kit::job(cx, move || acc::load_accounts(&env), move |v, list, cx| {
        v.accounts = list;
        v.loading = false;
        if then_check && !v.accounts.is_empty() {
            check_all(v, cx);
        }
    });
}

fn shape(list: &[Account]) -> Vec<Account> {
    list.iter().map(|a| Account { checked_at: None, ..a.clone() }).collect()
}

pub fn check_all(v: &mut AccountsTab, cx: &mut Context<AccountsTab>) {
    let list: Vec<Account> = v.accounts.iter().filter(|a| !v.busy.contains_key(&account_id(a))).cloned().collect();
    if list.is_empty() || v.checking_all {
        return;
    }
    v.checking_all = true;
    for a in &list {
        v.busy.insert(account_id(a), "Checking…".into());
    }
    let env = v.ctx.env.clone();
    let before = shape(&v.accounts);
    kit::job(cx, move || acc::check_accounts(&env, &list).map(|_| acc::load_accounts(&env)), move |v, got, cx| {
        v.checking_all = false;
        v.busy.clear();
        match got {
            Ok(list) => {
                if shape(&list) != before {
                    cx.notify();
                }
                v.accounts = list;
            }
            Err(e) => v.note = Some((false, format!("Could not check the accounts. {e}"))),
        }
    });
}

fn act(
    v: &mut AccountsTab,
    account: &Account,
    words: &str,
    work: impl FnOnce(&acc::Env) -> Result<Option<String>, String> + Send + 'static,
    cx: &mut Context<AccountsTab>,
) {
    let id = account_id(account);
    if v.busy.contains_key(&id) {
        return;
    }
    v.busy.insert(id.clone(), words.into());
    v.errors.remove(&id);
    v.note = None;
    let env = v.ctx.env.clone();
    kit::job(
        cx,
        move || {
            let done = work(&env);
            (done, acc::load_accounts(&env))
        },
        move |v, (done, list), _| {
            v.busy.remove(&id);
            v.accounts = list;
            match done {
                Ok(Some(text)) => v.note = Some((true, text)),
                Ok(None) => {}
                Err(e) => {
                    v.errors.insert(id, e);
                }
            }
        },
    );
    cx.notify();
}

pub fn toggle_add(v: &mut AccountsTab, window: &mut Window, cx: &mut Context<AccountsTab>) {
    v.add.open = !v.add.open;
    v.add.error = None;
    if v.add.open {
        window.focus(&v.add_token.focus_handle(cx));
    }
    cx.notify();
}

pub fn close_add(v: &mut AccountsTab, cx: &mut Context<AccountsTab>) {
    v.add = Add::default();
    v.add_token.update(cx, |i, cx| i.set_value("", cx));
    v.add_user.update(cx, |i, cx| i.set_value("", cx));
    cx.notify();
}

pub fn submit_add(v: &mut AccountsTab, cx: &mut Context<AccountsTab>) {
    if v.add.busy {
        return;
    }
    let token = kit::value(&v.add_token, cx).trim().to_string();
    if token.is_empty() {
        v.add.error = Some("Paste a token first.".into());
        cx.notify();
        return;
    }
    let user = kit::value(&v.add_user, cx).trim().to_string();
    let user = (v.add.needs_user && !user.is_empty()).then_some(user);
    v.add.busy = true;
    v.add.error = None;
    v.note = None;
    let env = v.ctx.env.clone();
    kit::job(
        cx,
        move || {
            let added = acc::add_account(&env, HOST, &token, user.as_deref());
            (added, acc::load_accounts(&env))
        },
        |v, (added, list): (Result<Account, AddError>, Vec<Account>), cx| {
            v.add.busy = false;
            v.accounts = list;
            match added {
                Ok(a) => {
                    v.add_token.update(cx, |i, cx| i.set_value("", cx));
                    v.add_user.update(cx, |i, cx| i.set_value("", cx));
                    v.add = Add::default();
                    let more = if a.status == "ok" { "" } else { " It needs a look, see below." };
                    v.note = Some((true, format!("Added @{}.{more}", a.username)));
                }
                Err(e) => {
                    v.add.error = Some(e.message.clone());
                    if e.needs_username {
                        v.add.needs_user = true;
                    }
                }
            }
        },
    );
    cx.notify();
}

pub fn open_reconnect(v: &mut AccountsTab, account: &Account, window: &mut Window, cx: &mut Context<AccountsTab>) {
    let id = account_id(account);
    v.re_id = if v.re_id.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
    v.remove_id = None;
    v.errors.remove(&id);
    v.re_token.update(cx, |i, cx| i.set_value("", cx));
    if v.re_id.is_some() {
        window.focus(&v.re_token.focus_handle(cx));
    }
    cx.notify();
}

pub fn submit_reconnect(v: &mut AccountsTab, cx: &mut Context<AccountsTab>) {
    let Some(account) = v.accounts.iter().find(|a| Some(account_id(a)) == v.re_id).cloned() else {
        return;
    };
    let token = kit::value(&v.re_token, cx).trim().to_string();
    if token.is_empty() {
        v.errors.insert(account_id(&account), "Paste the new token first.".into());
        cx.notify();
        return;
    }
    v.re_token.update(cx, |i, cx| i.set_value("", cx));
    v.re_id = None;
    let who = account.clone();
    act(
        v,
        &account,
        "Checking the new token…",
        move |env| {
            let done = acc::reconnect(env, &who, &token).map_err(|e| e.message)?;
            Ok((done.status == "ok").then(|| format!("@{} is connected again.", who.username)))
        },
        cx,
    );
}

pub fn open_remove(v: &mut AccountsTab, account: &Account, cx: &mut Context<AccountsTab>) {
    let id = account_id(account);
    v.remove_id = if v.remove_id.as_deref() == Some(id.as_str()) { None } else { Some(id) };
    v.remove_token = true;
    v.re_id = None;
    v.remove_profiles = if v.remove_id.is_some() { load_profiles(&v.ctx.env) } else { Vec::new() };
    cx.notify();
}

pub fn confirm_remove(v: &mut AccountsTab, account: &Account, cx: &mut Context<AccountsTab>) {
    let who = account.clone();
    let delete = v.remove_token;
    v.remove_id = None;
    act(
        v,
        account,
        "Removing…",
        move |env| {
            acc::remove_account(env, &who, delete)?;
            Ok(Some(format!("Removed @{}.", who.username)))
        },
        cx,
    );
}

pub fn make_default(v: &mut AccountsTab, account: &Account, cx: &mut Context<AccountsTab>) {
    let who = account.clone();
    act(
        v,
        account,
        "Saving…",
        move |env| {
            acc::set_default(env, &who.host, &who.username)?;
            Ok(Some(format!("@{} is now the default for {}.", who.username, who.host)))
        },
        cx,
    );
}

pub fn check_one(v: &mut AccountsTab, account: &Account, cx: &mut Context<AccountsTab>) {
    let who = account.clone();
    act(v, account, "Checking…", move |env| acc::check_account(env, &who).map(|_| None), cx);
}

pub fn set_try_others(v: &mut AccountsTab, on: bool, cx: &mut Context<AccountsTab>) {
    v.try_others = on;
    v.try_error = kit::set_setting(&v.ctx.db, TRY_OTHERS_KEY, &Value::Bool(on))
        .err()
        .map(|e| format!("Could not save this setting. {e}"));
    cx.notify();
}

fn open_token_page(cx: &mut App) {
    cx.open_url(&token_page_url(HOST, APP));
}

fn tone_color(t: &Theme, tone: Tone) -> gpui::Hsla {
    match tone {
        Tone::Ok => t.green,
        Tone::Warn => t.yellow,
        Tone::Bad => t.red,
        Tone::Hint => t.dim,
    }
}

fn add_panel(v: &AccountsTab, t: &Theme, cx: &mut Context<AccountsTab>) -> AnyElement {
    let busy = v.add.busy;
    kit::card(t, "acct-add-box", false)
        .child(kit::strong(t, "Add a GitHub account with a token"))
        .child(kit::small(t, format!("Host: {HOST}")))
        .child(kit::field(&v.add_token, "acct-add-token"))
        .when(v.add.needs_user, |el| {
            el.child(kit::field(&v.add_user, "acct-add-user")).child(kit::hint(
                t,
                format!("GitHub cannot be reached now. {APP} saves the token for this user name and checks it later."),
            ))
        })
        .child(kit::hint(
            t,
            format!(
                "The token needs these scopes: {}. {APP} checks it with GitHub, then keeps it in the macOS Keychain.",
                TOKEN_SCOPES.join(", ")
            ),
        ))
        .when_some(v.add.error.clone(), |el, e| el.child(kit::error_text(t, e)))
        .child(
            kit::row()
                .pt(px(6.))
                .child(
                    kit::act(t, "acct-add-create", "Create a token on GitHub", ButtonKind::Secondary, false)
                        .on_click(|_, _, cx| open_token_page(cx)),
                )
                .child(div().flex_1())
                .when(busy, |el| el.child(kit::busy_line(t, "acct-add-spin", "Checking the token…")))
                .when(!busy, |el| {
                    el.child(
                        kit::act(t, "acct-add-cancel", "Cancel", ButtonKind::Secondary, false)
                            .on_click(cx.listener(|v, _, _, cx| close_add(v, cx))),
                    )
                    .child(
                        kit::act(t, "acct-add-save", "Add account", ButtonKind::Primary, false)
                            .on_click(cx.listener(|v, _, _, cx| submit_add(v, cx))),
                    )
                }),
        )
        .into_any_element()
}

fn account_card(v: &AccountsTab, t: &Theme, a: &Account, now: i64, cx: &mut Context<AccountsTab>) -> AnyElement {
    let id = account_id(a);
    let sid = kit::safe_id(&id);
    let (tone, status) = describe_status(a, now);
    let color = tone_color(t, tone);
    let missing = missing_scopes(a);
    let letter: String = if a.name.trim().is_empty() { &a.username } else { &a.name }
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into());
    let scopes = if !a.scopes.is_empty() {
        format!("Scopes: {}", a.scopes.join(", "))
    } else if a.status == "ok" {
        "Scopes: not listed (fine-grained token)".into()
    } else {
        String::new()
    };
    let facts: Vec<String> = [
        scopes,
        expiry_text(a, now),
        checked_text(a, now),
        if a.source == "gh" { "Added from gh".into() } else { "Pasted token".into() },
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    let busy = v.busy.get(&id).cloned();
    let needs_token = ["expired", "revoked", "error"].contains(&a.status.as_str());
    let shown = if a.name.trim().is_empty() { a.username.clone() } else { a.name.clone() };
    let mut card = kit::card(t, SharedString::from(format!("acct-{sid}")), a.default)
        .child(
            kit::row()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(32.))
                        .flex_none()
                        .rounded_full()
                        .bg(t.selected)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.accent)
                        .child(letter),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            kit::row()
                                .child(kit::strong(t, shown))
                                .when(a.default, |el| {
                                    el.child(div().flex_none().text_size(px(11.5)).text_color(t.accent).child(format!("★ Default for {}", a.host)))
                                }),
                        )
                        .child(kit::small(t, format!("@{} · {}", a.username, a.host))),
                ),
        )
        .child(
            kit::row()
                .pt(px(4.))
                .gap(px(6.))
                .child(kit::dot(color, 8.))
                .child(div().text_size(px(12.)).text_color(color).child(status)),
        )
        .when_some(a.message.clone().filter(|m| !m.is_empty()), |el, m| {
            el.child(if a.status == "ok" { kit::hint(t, m) } else { kit::error_text(t, m) })
        })
        .when(!missing.is_empty(), |el| {
            el.child(kit::warn_text(
                t,
                format!(
                    "Missing scopes: {}. Some git or GitHub actions may fail. Make a new token with {}.",
                    missing.join(", "),
                    TOKEN_SCOPES.join(", ")
                ),
            ))
        })
        .child(kit::hint(t, facts.join(" · ")));
    if let Some(words) = busy {
        card = card.child(div().pt(px(8.)).child(kit::busy_line(t, SharedString::from(format!("acct-{sid}-spin")), words)));
    } else {
        let (a1, a2, a3, a4) = (a.clone(), a.clone(), a.clone(), a.clone());
        card = card.child(
            kit::wrap_row()
                .pt(px(8.))
                .gap(px(6.))
                .when(!a.default, |el| {
                    el.child(
                        kit::act(t, format!("acct-{sid}-default"), "★ Make default", ButtonKind::Secondary, false)
                            .on_click(cx.listener(move |v, _, _, cx| make_default(v, &a1, cx))),
                    )
                })
                .child(
                    kit::act(t, format!("acct-{sid}-check"), "Check now", ButtonKind::Secondary, false)
                        .on_click(cx.listener(move |v, _, _, cx| check_one(v, &a2, cx))),
                )
                .child(
                    kit::act(
                        t,
                        format!("acct-{sid}-reconnect"),
                        "Reconnect",
                        if needs_token { ButtonKind::Primary } else { ButtonKind::Secondary },
                        false,
                    )
                    .on_click(cx.listener(move |v, _, window, cx| open_reconnect(v, &a3, window, cx))),
                )
                .child(
                    kit::act(t, format!("acct-{sid}-remove"), "Remove", ButtonKind::Danger, false)
                        .on_click(cx.listener(move |v, _, _, cx| open_remove(v, &a4, cx))),
                ),
        );
        if v.re_id.as_deref() == Some(id.as_str()) {
            let who = a.clone();
            card = card.child(
                div()
                    .pt(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(kit::hint(t, format!("Paste a new token for @{0}. It must belong to @{0}.", a.username)))
                    .child(kit::field(&v.re_token, &format!("acct-{sid}-retoken")))
                    .child(
                        kit::row()
                            .child(
                                kit::act(t, format!("acct-{sid}-retoken-create"), "Create a token on GitHub", ButtonKind::Secondary, false)
                                    .on_click(|_, _, cx| open_token_page(cx)),
                            )
                            .child(div().flex_1())
                            .child(
                                kit::act(t, format!("acct-{sid}-retoken-cancel"), "Cancel", ButtonKind::Secondary, false)
                                    .on_click(cx.listener(move |v, _, window, cx| open_reconnect(v, &who, window, cx))),
                            )
                            .child(
                                kit::act(t, format!("acct-{sid}-retoken-save"), "Save token", ButtonKind::Primary, false)
                                    .on_click(cx.listener(|v, _, _, cx| submit_reconnect(v, cx))),
                            ),
                    ),
            );
        }
        if v.remove_id.as_deref() == Some(id.as_str()) {
            let lines = remove_text(a, &v.accounts, &v.remove_profiles, v.remove_token, APP);
            let (b1, b2) = (a.clone(), a.clone());
            let on = v.remove_token;
            card = card.child(
                div()
                    .pt(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(lines.into_iter().enumerate().map(|(i, line)| {
                        if i == 0 { kit::hint(t, line).text_color(t.foreground) } else { kit::warn_text(t, line) }
                    }))
                    .child(
                        kit::check(t, format!("acct-{sid}-remove-token"), on, "Also delete its token from the macOS Keychain")
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.remove_token = !on;
                                cx.notify();
                            })),
                    )
                    .when(a.source == "gh", |el| el.child(kit::hint(t, "Your gh login stays as it is.")))
                    .child(
                        kit::row()
                            .justify_end()
                            .child(
                                kit::act(t, format!("acct-{sid}-remove-cancel"), "Cancel", ButtonKind::Secondary, false)
                                    .on_click(cx.listener(move |v, _, _, cx| open_remove(v, &b1, cx))),
                            )
                            .child(
                                kit::act(t, format!("acct-{sid}-remove-yes"), "Remove", ButtonKind::Danger, false)
                                    .on_click(cx.listener(move |v, _, _, cx| confirm_remove(v, &b2, cx))),
                            ),
                    ),
            );
        }
    }
    card.when_some(v.errors.get(&id).cloned(), |el, e| el.child(kit::error_text(t, e)))
        .into_any_element()
}

impl Render for AccountsTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let now = acc::time::now();
        let list: AnyElement = if self.loading {
            kit::busy_line(&t, "acct-loading", "Loading accounts…").into_any_element()
        } else if let Some(e) = &self.load_error {
            kit::error_text(&t, e.clone()).into_any_element()
        } else if self.accounts.is_empty() {
            kit::hint(&t, "No accounts yet. Add one with a token.").into_any_element()
        } else {
            let accounts = self.accounts.clone();
            div()
                .flex()
                .flex_col()
                .children(accounts.iter().map(|a| account_card(self, &t, a, now, cx)))
                .into_any_element()
        };
        let on = self.try_others;
        kit::scroll_body("acct-tab")
            .child(kit::hint(
                &t,
                "Tokens are kept in the macOS Keychain. A repo uses the account of its profile, or the default account for the host.",
            ))
            .child(
                kit::wrap_row()
                    .pt(px(10.))
                    .child(
                        kit::act(&t, "acct-add", "Add with a token", if self.add.open { ButtonKind::Primary } else { ButtonKind::Secondary }, false)
                            .on_click(cx.listener(|v, _, window, cx| toggle_add(v, window, cx))),
                    )
                    .child(div().flex_1())
                    .when(!self.accounts.is_empty(), |el| {
                        el.child(
                            kit::act(&t, "acct-check-all", "Check all", ButtonKind::Secondary, self.checking_all)
                                .on_click(cx.listener(|v, _, _, cx| check_all(v, cx))),
                        )
                    }),
            )
            .when(self.add.open, |el| el.child(div().pt(px(10.)).child(add_panel(self, &t, cx))))
            .when_some(self.note.clone(), |el, (ok, text)| {
                el.child(if ok { kit::ok_text(&t, text) } else { kit::error_text(&t, text) })
            })
            .child(kit::label(&t, "Accounts"))
            .child(list)
            .child(kit::label(&t, "When GitHub refuses a sign-in"))
            .child(
                kit::check(&t, "acct-try-others", on, "Try my other GitHub accounts once")
                    .on_click(cx.listener(move |v, _, _, cx| set_try_others(v, !on, cx))),
            )
            .child(kit::hint(
                &t,
                "If push, pull, fetch or clone is refused, each other account for that host is tried once. The repo setting is not changed.",
            ))
            .when_some(self.try_error.clone(), |el, e| el.child(kit::error_text(&t, e)))
    }
}
