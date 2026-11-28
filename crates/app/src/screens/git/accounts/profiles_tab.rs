use super::kit::{self, Ctx};
use crate::screens::settings::parts::memory::text_area::TextArea;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Focusable, PathPromptOptions, SharedString, Window, div, px};
use nixa_git::accounts::profiles::{
    self as prof, COLORS, Choice, Identity, Profile, account_options, clear_repo, describe_accounts, load_profiles,
    normalize_profile, profile_id, read_repo_identity, save_profiles, split_lines, split_list, stamp_repo,
    validate_profile,
};
use nixa_git::accounts::{Account, load_accounts};
use nixa_ui::button::ButtonKind;
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::{Theme, theme};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct Editing {
    pub id: Option<String>,
    pub accounts: BTreeMap<String, String>,
    pub color: String,
}

pub struct ProfilesTab {
    pub ctx: Ctx,
    pub repo: Option<PathBuf>,
    pub profiles: Vec<Profile>,
    pub accounts: Vec<Account>,
    pub identity: Option<Identity>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub note: Option<String>,
    pub editing: Option<Editing>,
    pub form_error: Option<String>,
    pub confirm_delete: Option<String>,
    pub picking: bool,
    import: Vec<Profile>,
    pub label: Entity<TextInput>,
    pub name: Entity<TextInput>,
    pub email: Entity<TextInput>,
    pub owners: Entity<TextInput>,
    pub folders: Entity<TextArea>,
}

pub fn new(ctx: Ctx, repo: Option<PathBuf>, cx: &mut App) -> Entity<ProfilesTab> {
    cx.new(|cx| build(ctx, repo, cx))
}

pub fn build(ctx: Ctx, repo: Option<PathBuf>, cx: &mut Context<ProfilesTab>) -> ProfilesTab {
    let label = kit::input("Work, Personal…", "", false, cx);
    let name = kit::input("Your Name", "", false, cx);
    let email = kit::input("you@example.com", "", false, cx);
    let owners = kit::input("my-org, another-org", "", false, cx);
    let folders = cx.new(|cx| TextArea::new(cx, "~/Projects/work"));
    for input in [&label, &name, &email, &owners] {
        cx.subscribe(input, |v, _, e: &InputEvent, cx| match e {
            InputEvent::Submit(_) => save(v, cx),
            InputEvent::Cancel => {
                v.editing = None;
                cx.notify();
            }
            InputEvent::Changed(_) => {
                if v.form_error.take().is_some() {
                    cx.notify();
                }
            }
        })
        .detach();
    }
    let mut view = ProfilesTab {
        ctx,
        repo,
        profiles: Vec::new(),
        accounts: Vec::new(),
        identity: None,
        loading: true,
        busy: false,
        error: None,
        note: None,
        editing: None,
        form_error: None,
        confirm_delete: None,
        picking: false,
        import: Vec::new(),
        label,
        name,
        email,
        owners,
        folders,
    };
    load(&mut view, cx);
    view
}

type Loaded = (Vec<Profile>, Vec<Account>, Option<Identity>);

pub fn load(v: &mut ProfilesTab, cx: &mut Context<ProfilesTab>) {
    v.loading = true;
    let env = v.ctx.env.clone();
    let repo = v.repo.clone();
    kit::job(
        cx,
        move || -> Loaded {
            let identity = repo.as_ref().and_then(|r| read_repo_identity(&env, r).ok());
            (load_profiles(&env), load_accounts(&env), identity)
        },
        |v, (profiles, accounts, identity), _| {
            v.profiles = profiles;
            v.accounts = accounts;
            v.identity = identity;
            v.loading = false;
        },
    );
}

fn finish(v: &mut ProfilesTab, note: String, cx: &mut Context<ProfilesTab>) {
    v.busy = false;
    v.note = Some(note);
    load(v, cx);
}

pub fn start_edit(v: &mut ProfilesTab, profile: Option<&Profile>, window: &mut Window, cx: &mut Context<ProfilesTab>) {
    let p = profile.cloned().unwrap_or_default();
    let color = if profile.is_some() { p.color.clone() } else { prof::next_color(&v.profiles) };
    v.editing = Some(Editing { id: profile.map(|p| p.id.clone()), accounts: p.accounts.clone(), color });
    v.form_error = None;
    v.confirm_delete = None;
    v.note = None;
    v.label.update(cx, |i, cx| i.set_value(p.label.clone(), cx));
    v.name.update(cx, |i, cx| i.set_value(p.name.clone(), cx));
    v.email.update(cx, |i, cx| i.set_value(p.email.clone(), cx));
    v.owners.update(cx, |i, cx| i.set_value(p.owners.join(", "), cx));
    v.folders.update(cx, |i, cx| i.set_value(p.folders.join("\n"), cx));
    window.focus(&v.label.focus_handle(cx));
    cx.notify();
}

pub fn save(v: &mut ProfilesTab, cx: &mut Context<ProfilesTab>) {
    let Some(edit) = v.editing.as_ref() else { return };
    if v.busy {
        return;
    }
    let label = kit::value(&v.label, cx);
    let name = kit::value(&v.name, cx);
    let email = kit::value(&v.email, cx);
    if let Some(problem) = validate_profile(&label, &name, &email) {
        v.form_error = Some(problem.into());
        cx.notify();
        return;
    }
    let folders = split_lines(v.folders.read(cx).value());
    let owners = split_list(&kit::value(&v.owners, cx));
    let id = edit.id.clone();
    let accounts = edit.accounts.clone();
    let color = edit.color.clone();
    let repo = v.repo.clone();
    let in_use = v.identity.as_ref().and_then(|i| i.profile_id.clone());
    v.busy = true;
    v.form_error = None;
    let env = v.ctx.env.clone();
    kit::job(
        cx,
        move || -> Result<String, String> {
            let current = load_profiles(&env);
            let exists = id.as_ref().is_some_and(|id| current.iter().any(|p| &p.id == id));
            let taken: Vec<String> = current.iter().map(|p| p.id.clone()).collect();
            let profile = normalize_profile(&Profile {
                id: if exists { id.clone().unwrap_or_default() } else { profile_id(&label, &taken) },
                label,
                name,
                email,
                accounts,
                folders,
                owners,
                color,
            });
            let next: Vec<Profile> = if exists {
                current.iter().map(|p| if p.id == profile.id { profile.clone() } else { p.clone() }).collect()
            } else {
                current.iter().cloned().chain([profile.clone()]).collect()
            };
            save_profiles(&env, &next).map_err(|e| format!("Could not save the profile. {e}"))?;
            match repo {
                Some(root) if in_use.as_deref() == Some(profile.id.as_str()) => Ok(match stamp_repo(&env, &root, &profile, false) {
                    Ok(_) => "Saved. This repo now uses the new details.".into(),
                    Err(e) => format!("Saved, but this repo could not be updated. {e}"),
                }),
                _ => Ok("Saved.".into()),
            }
        },
        |v, done, cx| match done {
            Ok(note) => {
                v.editing = None;
                finish(v, note, cx);
            }
            Err(e) => {
                v.busy = false;
                v.form_error = Some(e);
            }
        },
    );
    cx.notify();
}

pub fn remove(v: &mut ProfilesTab, profile: &Profile, cx: &mut Context<ProfilesTab>) {
    if v.busy {
        return;
    }
    v.busy = true;
    v.confirm_delete = None;
    let env = v.ctx.env.clone();
    let gone = profile.clone();
    kit::job(
        cx,
        move || {
            let current = load_profiles(&env);
            let next: Vec<Profile> = current.into_iter().filter(|p| p.id != gone.id).collect();
            save_profiles(&env, &next).map(|_| gone)
        },
        |v, done, cx| match done {
            Ok(p) => {
                if v.editing.as_ref().and_then(|e| e.id.as_deref()) == Some(p.id.as_str()) {
                    v.editing = None;
                }
                finish(v, format!("Deleted {}.", p.label), cx);
            }
            Err(e) => {
                v.busy = false;
                v.error = Some(format!("Could not delete the profile. {e}"));
            }
        },
    );
    cx.notify();
}

pub fn use_profile(v: &mut ProfilesTab, profile: Option<Profile>, cx: &mut Context<ProfilesTab>) {
    let Some(root) = v.repo.clone() else { return };
    if v.busy {
        return;
    }
    v.busy = true;
    v.picking = false;
    let env = v.ctx.env.clone();
    kit::job(cx, move || apply_profile(&env, &root, profile.as_ref()), |v, done, cx| match done {
        Ok(note) => finish(v, note, cx),
        Err(e) => {
            v.busy = false;
            v.error = Some(e);
        }
    });
    cx.notify();
}

pub fn apply_profile(env: &nixa_git::accounts::Env, root: &std::path::Path, profile: Option<&Profile>) -> Result<String, String> {
    match profile {
        Some(p) => {
            let done = stamp_repo(env, root, p, false)?;
            let signs: Vec<String> = done.bound.iter().map(|(h, u)| format!("@{u} on {h}")).collect();
            Ok(if signs.is_empty() {
                format!("This repo now uses {}.", p.label)
            } else {
                format!("This repo now uses {}. It signs in as {}.", p.label, signs.join(", "))
            })
        }
        None => {
            clear_repo(env, root)?;
            Ok("This repo has no profile now.".into())
        }
    }
}

pub fn picker(
    t: &Theme,
    id: &str,
    profiles: &[Profile],
    current: Option<&str>,
    busy: bool,
    on_pick: impl Fn(Option<Profile>, &mut App) + Clone + 'static,
) -> AnyElement {
    let mut row = kit::wrap_row().pt(px(6.));
    for p in profiles {
        let pick = on_pick.clone();
        let chosen = p.clone();
        row = row.child(
            kit::row()
                .w_auto()
                .gap(px(5.))
                .child(kit::dot(kit::profile_color(t, &p.color), 8.))
                .child(
                    kit::pill(t, format!("{id}-{}", kit::safe_id(&p.id)), p.label.clone(), current == Some(p.id.as_str()), busy)
                        .on_click(move |_, _, cx| pick(Some(chosen.clone()), cx)),
                ),
        );
    }
    let pick = on_pick.clone();
    row.child(
        kit::pill(t, format!("{id}-none"), "No profile", false, busy || current.is_none()).on_click(move |_, _, cx| pick(None, cx)),
    )
    .into_any_element()
}

fn repo_card(v: &ProfilesTab, t: &Theme, cx: &mut Context<ProfilesTab>) -> Option<AnyElement> {
    let root = v.repo.as_ref()?;
    let name = kit::repo_name(&root.display().to_string());
    let mut card = kit::card(t, "prof-repo", false).child(kit::small(t, name));
    let Some(identity) = v.identity.as_ref() else {
        return Some(card.child(kit::hint(t, "Could not read this repo.")).into_any_element());
    };
    let found = identity.profile_id.as_ref().and_then(|id| v.profiles.iter().find(|p| &p.id == id));
    card = match (found, identity.profile_id.as_ref()) {
        (Some(p), _) => card.child(
            kit::row()
                .child(kit::dot(kit::profile_color(t, &p.color), 8.))
                .child(kit::strong(t, format!("This repo uses: {}", p.label)))
                .child(kit::small(t, prof::profile_summary(p)).flex_1()),
        ),
        (None, Some(id)) => card
            .child(kit::strong(t, "This repo uses: no profile yet"))
            .child(kit::warn_text(t, format!("Its profile \"{id}\" was deleted."))),
        (None, None) => card.child(kit::strong(t, "This repo uses: no profile yet")),
    };
    if found.is_none()
        && let Some(email) = identity.local_email.as_ref()
    {
        card = card.child(kit::hint(
            t,
            format!("It commits as {} <{email}>.", identity.local_name.clone().unwrap_or_else(|| "(no name)".into())),
        ));
    }
    let picking = v.picking;
    card = card.child(
        kit::row().pt(px(4.)).child(
            kit::act(t, "prof-repo-change", if picking { "Close" } else { "Change" }, ButtonKind::Secondary, v.busy).on_click(
                cx.listener(move |v, _, _, cx| {
                    v.picking = !picking;
                    cx.notify();
                }),
            ),
        ),
    );
    if picking {
        let me = cx.entity().downgrade();
        let current = identity.profile_id.clone();
        card = card.child(picker(t, "prof-pick", &v.profiles, current.as_deref(), v.busy, move |p, cx| {
            me.update(cx, |v, cx| use_profile(v, p, cx)).ok();
        }));
    }
    Some(card.into_any_element())
}

fn account_picker(v: &ProfilesTab, t: &Theme, cx: &mut Context<ProfilesTab>) -> AnyElement {
    let Some(edit) = v.editing.as_ref() else { return div().into_any_element() };
    let options = account_options(&v.accounts, &edit.accounts);
    if options.is_empty() {
        return kit::hint(t, "No GitHub accounts yet. Add one in the Accounts tab, then pick it here.").into_any_element();
    }
    let mut hosts: Vec<String> = Vec::new();
    for c in &options {
        if !hosts.contains(&c.host) {
            hosts.push(c.host.clone());
        }
    }
    let mut col = div().flex().flex_col().gap(px(6.));
    for host in hosts {
        let chosen = edit.accounts.get(&host).cloned();
        let mut row = kit::wrap_row().child(kit::small(t, host.clone()).w(px(84.)).flex_none());
        let list: Vec<&Choice> = options.iter().filter(|c| c.host == host).collect();
        for c in list {
            let on = chosen.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(&c.username));
            let (h, u) = (c.host.clone(), c.username.clone());
            row = row.child(
                kit::pill(t, format!("prof-acct-{}-{}", kit::safe_id(&h), kit::safe_id(&u)), format!("@{}", c.username), on, false)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        if let Some(e) = v.editing.as_mut() {
                            if on {
                                e.accounts.remove(&h);
                            } else {
                                e.accounts.insert(h.clone(), u.clone());
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        col = col.child(row);
    }
    col.into_any_element()
}

fn editor(v: &ProfilesTab, t: &Theme, cx: &mut Context<ProfilesTab>) -> AnyElement {
    let Some(edit) = v.editing.as_ref() else { return div().into_any_element() };
    let color = edit.color.clone();
    let mut colors = kit::wrap_row();
    for c in COLORS {
        let on = color == *c;
        let pick = c.to_string();
        colors = colors.child(
            kit::row()
                .w_auto()
                .gap(px(5.))
                .child(kit::dot(kit::profile_color(t, c), 9.))
                .child(kit::pill(t, format!("prof-color-{c}"), kit::color_name(c), on, false).on_click(cx.listener(move |v, _, _, cx| {
                    if let Some(e) = v.editing.as_mut() {
                        e.color = pick.clone();
                    }
                    cx.notify();
                }))),
        );
    }
    kit::card(t, "prof-editor", true)
        .child(kit::strong(t, if edit.id.is_some() { "Edit profile" } else { "New profile" }))
        .child(kit::label(t, "Label").pt(px(6.)))
        .child(kit::field(&v.label, "prof-label"))
        .child(kit::label(t, "Name for commits").pt(px(6.)))
        .child(kit::field(&v.name, "prof-name"))
        .child(kit::label(t, "Email for commits").pt(px(6.)))
        .child(kit::field(&v.email, "prof-email"))
        .child(kit::label(t, "GitHub account").pt(px(6.)))
        .child(account_picker(v, t, cx))
        .child(kit::label(t, "Folders (one per line). Repos inside them use this profile.").pt(px(6.)))
        .child(div().w_full().h(px(70.)).flex().flex_col().debug_selector(|| "prof-folders".into()).child(v.folders.clone()))
        .child(kit::label(t, "GitHub owners (comma list). Repos of these owners use this profile.").pt(px(6.)))
        .child(kit::field(&v.owners, "prof-owners"))
        .child(kit::label(t, "Colour").pt(px(6.)))
        .child(colors)
        .when_some(v.form_error.clone(), |el, e| el.child(kit::error_text(t, e)))
        .child(
            kit::row()
                .pt(px(8.))
                .justify_end()
                .child(
                    kit::act(t, "prof-cancel", "Cancel", ButtonKind::Secondary, v.busy).on_click(cx.listener(|v, _, _, cx| {
                        v.editing = None;
                        cx.notify();
                    })),
                )
                .child(
                    kit::act(t, "prof-save", if v.busy { "Saving…" } else { "Save profile" }, ButtonKind::Primary, v.busy)
                        .on_click(cx.listener(|v, _, _, cx| save(v, cx))),
                ),
        )
        .into_any_element()
}

fn profile_card(v: &ProfilesTab, t: &Theme, p: &Profile, cx: &mut Context<ProfilesTab>) -> AnyElement {
    let sid = kit::safe_id(&p.id);
    let in_use = v.identity.as_ref().and_then(|i| i.profile_id.as_deref()) == Some(p.id.as_str());
    let confirm = v.confirm_delete.as_deref() == Some(p.id.as_str());
    let (e1, d1, d2, d3) = (p.clone(), p.id.clone(), p.clone(), p.id.clone());
    kit::card(t, SharedString::from(format!("prof-{sid}")), in_use)
        .child(
            kit::row()
                .child(kit::dot(kit::profile_color(t, &p.color), 9.))
                .child(kit::strong(t, p.label.clone()))
                .when(in_use, |el| el.child(div().flex_none().text_size(px(11.5)).text_color(t.accent).child("In use here")))
                .child(div().flex_1())
                .child(
                    kit::act(t, format!("prof-{sid}-edit"), "Edit", ButtonKind::Secondary, v.busy)
                        .on_click(cx.listener(move |v, _, window, cx| start_edit(v, Some(&e1), window, cx))),
                )
                .child(
                    kit::act(t, format!("prof-{sid}-delete"), "Delete", ButtonKind::Danger, v.busy).on_click(cx.listener(
                        move |v, _, _, cx| {
                            v.confirm_delete = if v.confirm_delete.as_deref() == Some(d1.as_str()) { None } else { Some(d1.clone()) };
                            cx.notify();
                        },
                    )),
                ),
        )
        .child(kit::small(t, format!("{} <{}>", p.name, p.email)))
        .when(!describe_accounts(p).is_empty(), |el| el.child(kit::hint(t, describe_accounts(p))))
        .when(!p.folders.is_empty(), |el| el.child(kit::hint(t, format!("Folders: {}", p.folders.join(", ")))))
        .when(!p.owners.is_empty(), |el| el.child(kit::hint(t, format!("Owners: {}", p.owners.join(", ")))))
        .when(confirm, |el| {
            el.child(kit::warn_text(
                t,
                format!("Delete {}? Repos keep their name and email, but will ask for a profile again.", p.label),
            ))
            .child(
                kit::row()
                    .justify_end()
                    .child(
                        kit::act(t, format!("prof-{sid}-delete-cancel"), "Cancel", ButtonKind::Secondary, false).on_click(
                            cx.listener(move |v, _, _, cx| {
                                if v.confirm_delete.as_deref() == Some(d3.as_str()) {
                                    v.confirm_delete = None;
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        kit::act(t, format!("prof-{sid}-delete-yes"), "Delete", ButtonKind::Danger, false)
                            .on_click(cx.listener(move |v, _, _, cx| remove(v, &d2, cx))),
                    ),
            )
        })
        .into_any_element()
}

fn parse_import(bytes: &[u8]) -> Result<Vec<Profile>, String> {
    #[derive(serde::Deserialize)]
    struct IdentityFields {
        label: String,
        name: String,
        email: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Export {
        List(Vec<IdentityFields>),
        Wrapped { profiles: Vec<IdentityFields> },
    }
    let export: Export = serde_json::from_slice(bytes).map_err(|_| "Expected a JSON list of profiles with label, name and email fields.".to_string())?;
    let rows = match export { Export::List(rows) | Export::Wrapped { profiles: rows } => rows };
    if rows.is_empty() || rows.len() > 100 {
        return Err("The export must contain 1 to 100 profiles.".into());
    }
    rows.into_iter().enumerate().map(|(index, row)| {
        if [&row.label, &row.name, &row.email].iter().any(|field| field.len() > 256 || field.chars().any(char::is_control)) {
            return Err(format!("Profile {} has an invalid identity field.", index + 1));
        }
        if let Some(error) = validate_profile(&row.label, &row.name, &row.email) {
            return Err(format!("Profile {}: {error}", index + 1));
        }
        Ok(normalize_profile(&Profile { label: row.label, name: row.name, email: row.email, ..Profile::default() }))
    }).collect()
}

fn choose_import(v: &mut ProfilesTab, cx: &mut Context<ProfilesTab>) {
    v.busy = true;
    v.error = None;
    v.note = None;
    let answer = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose a Git identity export".into()) });
    cx.spawn(async move |this, cx| {
        let path = match answer.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            _ => None,
        };
        this.update(cx, |v, cx| {
            let Some(path) = path else {
                v.busy = false;
                cx.notify();
                return;
            };
            kit::job(cx, move || {
                use std::io::Read;
                const LIMIT: u64 = 1024 * 1024;
                if !std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                    return Err("Choose a regular export file.".into());
                }
                let mut bytes = Vec::new();
                std::fs::File::open(path).and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
                    .map_err(|_| "Could not read the export file.".to_string())?;
                if bytes.len() as u64 > LIMIT {
                    return Err("The export exceeds 1 MiB.".into());
                }
                parse_import(&bytes)
            }, |v, result, _| {
                v.busy = false;
                match result {
                    Ok(profiles) => v.import = profiles,
                    Err(error) => v.error = Some(error),
                }
            });
        }).ok();
    }).detach();
    cx.notify();
}

fn apply_import(v: &mut ProfilesTab, cx: &mut Context<ProfilesTab>) {
    if v.busy || v.import.is_empty() { return; }
    v.busy = true;
    v.error = None;
    let env = v.ctx.env.clone();
    let incoming = v.import.clone();
    kit::job(cx, move || {
        let mut current = load_profiles(&env);
        let before = current.len();
        for profile in incoming {
            if !current.iter().any(|p| p.label == profile.label && p.name == profile.name && p.email == profile.email) {
                current.push(profile);
            }
        }
        let added = current.len() - before;
        if added > 0 { save_profiles(&env, &current)?; }
        Ok::<_, String>(added)
    }, |v, result, cx| match result {
        Ok(added) => {
            v.import.clear();
            finish(v, format!("Imported {added} profiles. Existing identities were kept."), cx);
        }
        Err(error) => {
            v.busy = false;
            v.error = Some(error);
        }
    });
    cx.notify();
}

fn import_card(v: &ProfilesTab, t: &Theme, cx: &mut Context<ProfilesTab>) -> AnyElement {
    kit::card(t, "prof-import-preview", true)
        .child(kit::strong(t, "Preview Git identities"))
        .child(kit::hint(t, "Only labels, commit names and emails are imported. Existing identities are kept. No repositories, GitHub accounts or credentials are changed."))
        .children(v.import.iter().map(|p| kit::small(t, format!("{}: {} <{}>", p.label, p.name, p.email))))
        .child(kit::row().justify_end()
            .child(kit::act(t, "prof-import-cancel", "Cancel", ButtonKind::Secondary, v.busy).on_click(cx.listener(|v, _, _, cx| { v.import.clear(); cx.notify(); })))
            .child(kit::act(t, "prof-import-apply", "Import identities", ButtonKind::Primary, v.busy).on_click(cx.listener(|v, _, _, cx| apply_import(v, cx)))))
        .into_any_element()
}

impl Render for ProfilesTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let repo = repo_card(self, &t, cx);
        let editor = self.editing.as_ref().map(|_| editor(self, &t, cx));
        let list: AnyElement = if self.loading && self.profiles.is_empty() {
            kit::busy_line(&t, "prof-loading", "Loading profiles…").into_any_element()
        } else if self.profiles.is_empty() {
            kit::hint(&t, "No profiles yet. Add Work and Personal once, then each repo picks the right one.").into_any_element()
        } else {
            let profiles = self.profiles.clone();
            div()
                .flex()
                .flex_col()
                .children(profiles.iter().map(|p| profile_card(self, &t, p, cx)))
                .into_any_element()
        };
        kit::scroll_body("prof-tab")
            .child(kit::hint(
                &t,
                format!(
                    "A profile is a name, an email and a GitHub account. Each repo uses one profile. {} sets it in the repo only, never in your global git config.",
                    super::accounts_tab::APP
                ),
            ))
            .when_some(repo, |el, r| el.child(div().pt(px(10.)).child(r)))
            .when_some(self.note.clone(), |el, n| el.child(kit::ok_text(&t, n)))
            .when_some(self.error.clone(), |el, e| el.child(kit::error_text(&t, e)))
            .child(
                kit::row().pt(px(10.)).child(kit::label(&t, "Profiles").pt(px(0.)).pb(px(0.))).child(div().flex_1()).when(
                    self.editing.is_none(),
                    |el| {
                        el.child(
                            kit::act(&t, "prof-import", "Import export…", ButtonKind::Secondary, self.busy || !self.import.is_empty())
                                .on_click(cx.listener(|v, _, _, cx| choose_import(v, cx))),
                        ).child(
                            kit::act(&t, "prof-new", "New profile", ButtonKind::Primary, self.busy)
                                .on_click(cx.listener(|v, _, window, cx| start_edit(v, None, window, cx))),
                        )
                    },
                ),
            )
            .child(div().pt(px(8.)))
            .when_some(editor, |el, e| el.child(e))
            .when(!self.import.is_empty(), |el| el.child(import_card(self, &t, cx)))
            .child(list)
    }
}

#[cfg(test)]
mod import_tests {
    use super::parse_import;

    #[test]
    fn imports_only_identity_fields() {
        let profiles = parse_import(br#"{"profiles":[{"label":"Work","name":"Example User","email":"user@example.test","id":"old","accounts":{"example.test":"account"},"token":"ignored","folders":["/tmp/example"]}]}"#).unwrap();
        let profile = &profiles[0];
        assert_eq!(profile.label, "Work");
        assert_eq!(profile.name, "Example User");
        assert_eq!(profile.email, "user@example.test");
        assert!(profile.id.is_empty() && profile.accounts.is_empty() && profile.folders.is_empty());
    }

    #[test]
    fn rejects_invalid_identity_fields() {
        for export in [
            br#"[]"#.as_slice(),
            br#"[{"label":"Work","name":"Example User","email":"invalid"}]"#,
            br#"[{"label":"Work","name":"Example\nUser","email":"user@example.test"}]"#,
        ] {
            assert!(parse_import(export).is_err());
        }
    }
}
