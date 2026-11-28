use super::{ChangesView, git, refresh, report, view::tiny, words::Level};
use gpui::prelude::*;
use gpui::{AnyElement, Context};
use nixa_ui::{Icon, Theme};

pub(super) fn folder_buttons(
    v: &ChangesView,
    t: &Theme,
    folder: &str,
    index: usize,
    cx: &mut Context<ChangesView>,
) -> Vec<AnyElement> {
    let files = v.files.iter().filter(|file| file.folder == folder);
    let stage: Vec<_> = files
        .clone()
        .filter(|file| !file.conflict && file.y != ' ')
        .map(|file| file.path.clone())
        .collect();
    let unstage: Vec<_> = files
        .clone()
        .filter(|file| !file.conflict && file.staged)
        .map(|file| file.path.clone())
        .collect();
    let discard: Vec<_> = files.map(|file| file.path.clone()).collect();
    let label = if folder.is_empty() {
        "files at the repo root"
    } else {
        folder
    }
    .to_string();
    let busy = v.busy || super::local_job(v);
    let mut buttons = Vec::new();
    if !stage.is_empty() {
        let label = label.clone();
        buttons.push(
            nixa_ui::button::disabled(
                tiny(
                    t,
                    format!("changes-folder-stage-{index}"),
                    Icon::Plus,
                    "Stage changes in this folder",
                    None,
                ),
                busy,
            )
            .on_click(cx.listener(move |v, _, _, cx| super::stage(v, stage.clone(), &label, cx)))
            .into_any_element(),
        );
    }
    if !unstage.is_empty() {
        let label = label.clone();
        buttons.push(
            nixa_ui::button::disabled(
                tiny(
                    t,
                    format!("changes-folder-unstage-{index}"),
                    Icon::Minus,
                    "Unstage changes in this folder",
                    None,
                ),
                busy,
            )
            .on_click(
                cx.listener(move |v, _, _, cx| super::unstage(v, unstage.clone(), &label, cx)),
            )
            .into_any_element(),
        );
    }
    buttons.push(
        nixa_ui::button::disabled(
            tiny(
                t,
                format!("changes-folder-discard-{index}"),
                Icon::Undo,
                "Discard changes in this folder (asks first)",
                Some(t.red),
            ),
            busy,
        )
        .on_click(
            cx.listener(move |v, _, _, cx| super::ask_discard(v, discard.clone(), &label, cx)),
        )
        .into_any_element(),
    );
    buttons
}

pub(super) fn init_repo(v: &mut ChangesView, cx: &mut Context<ChangesView>) {
    let Some(folder) = v.folder.clone().or_else(|| v.cwd.clone()) else {
        return;
    };
    if v.busy || v.root.is_some() || v.loading {
        return;
    }
    v.busy = true;
    cx.notify();
    super::super::accounts::kit::job(
        cx,
        move || git::run(&folder, &["init"]),
        |v, out, cx| {
            v.busy = false;
            if !out.ok() {
                report(
                    cx,
                    "Could not initialize this repo",
                    git::detail_of(&out.said()),
                    Level::Error,
                );
            }
            refresh(v, cx);
        },
    );
}
