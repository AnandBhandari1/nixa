use super::git::{self, GitInfo, Worktree};
use super::{Action, Form, SpacesEvent, SpacesView, git_of, place, set_form};
use crate::spaces;
use gpui::prelude::*;
use gpui::{App, Context, Entity, Focusable, PathPromptOptions, Window};
use nixa_ui::text_input::{InputEvent, TextInput};

pub fn run(view: &Entity<SpacesView>, action: Action, space: Option<&str>, window: &mut Window, cx: &mut App) {
    view.update(cx, |v, cx| {
        let target = space.map(str::to_string).or_else(|| v.input.active.clone());
        match (action, target) {
            (Action::NewSpace, _) => cx.emit(SpacesEvent::New(None)),
            (Action::NewSpaceInFolder, _) => pick_folder(cx),
            (_, None) => {}
            (Action::Rename, Some(id)) => start_rename(v, &id, window, cx),
            (Action::Lock, Some(id)) => toggle_lock(v, &id, cx),
            (Action::NewWorktree, Some(id)) => start_branch(v, &id, window, cx),
            (Action::OpenWorktree, Some(id)) => open_worktrees(v, &id, cx),
            (Action::RemoveWorktree, Some(id)) => ask_remove(v, &id, cx),
            (Action::Close, Some(id)) => ask_close(v, &id, cx),
        }
        cx.notify();
    });
}

pub fn pick_folder(cx: &mut Context<SpacesView>) {
    let answer = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("New space".into()),
    });
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = answer.await
            && let Some(path) = paths.first()
        {
            let folder = path.display().to_string();
            this.update(cx, |_, cx| cx.emit(SpacesEvent::New(Some(folder)))).ok();
        }
    })
    .detach();
}

fn name_of(v: &SpacesView, id: &str) -> String {
    v.input
        .spaces
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.name.clone())
        .unwrap_or_default()
}

fn notice(cx: &mut Context<SpacesView>, text: String) {
    cx.emit(SpacesEvent::Notice(text));
}

fn text_input(
    placeholder: &'static str,
    value: String,
    window: &mut Window,
    cx: &mut Context<SpacesView>,
) -> Entity<TextInput> {
    let input = cx.new(|cx| TextInput::new(cx, placeholder));
    input.update(cx, |i, cx| i.set_value(value, cx));
    window.focus(&input.focus_handle(cx));
    input
}

pub fn start_rename(v: &mut SpacesView, id: &str, window: &mut Window, cx: &mut Context<SpacesView>) {
    if v.input.spaces.iter().all(|s| s.id != id) {
        return;
    }
    let input = text_input("Space name", name_of(v, id), window, cx);
    let space = id.to_string();
    let events = cx.subscribe(&input, move |v: &mut SpacesView, _, event: &InputEvent, cx| match event {
        InputEvent::Submit(value) => {
            let name = value.trim().to_string();
            if !name.is_empty() {
                cx.emit(SpacesEvent::Rename { id: space.clone(), name: name.clone() });
                if let Some(s) = v.input.spaces.iter_mut().find(|s| s.id == space) {
                    s.name = name;
                }
            }
            set_form(v, Form::None, None);
            cx.notify();
        }
        InputEvent::Cancel => {
            set_form(v, Form::None, None);
            cx.notify();
        }
        InputEvent::Changed(_) => {}
    });
    set_form(v, Form::Rename { id: id.to_string(), input }, Some(events));
}

pub fn toggle_lock(v: &mut SpacesView, id: &str, cx: &mut Context<SpacesView>) {
    let Some(space) = v.input.spaces.iter_mut().find(|s| s.id == id) else {
        return;
    };
    space.locked = !space.locked;
    cx.emit(SpacesEvent::Lock { id: id.to_string(), locked: space.locked });
}

fn need_git(v: &SpacesView, id: &str, cx: &mut Context<SpacesView>) -> Option<GitInfo> {
    let found = git_of(v, id);
    if found.is_none() {
        notice(cx, format!("Space {} is not in a git repo, or its git is not read yet.", name_of(v, id)));
    }
    found
}

pub fn start_branch(v: &mut SpacesView, id: &str, window: &mut Window, cx: &mut Context<SpacesView>) {
    let Some(info) = need_git(v, id, cx) else {
        return;
    };
    let input = text_input("New branch name", String::new(), window, cx);
    let space = id.to_string();
    let events = cx.subscribe(&input, move |v: &mut SpacesView, _, event: &InputEvent, cx| match event {
        InputEvent::Submit(value) => add_branch(v, &space, info.clone(), value.trim().to_string(), cx),
        InputEvent::Cancel => {
            set_form(v, Form::None, None);
            cx.notify();
        }
        InputEvent::Changed(_) => {}
    });
    set_form(v, Form::Branch { id: id.to_string(), input, busy: false }, Some(events));
}

pub fn submit_branch(v: &mut SpacesView, cx: &mut Context<SpacesView>) {
    let Form::Branch { id, input, busy: false } = &v.form else {
        return;
    };
    let (id, value) = (id.clone(), input.read(cx).value().trim().to_string());
    if let Some(info) = git_of(v, &id) {
        add_branch(v, &id, info, value, cx);
    }
}

fn add_branch(v: &mut SpacesView, id: &str, info: GitInfo, branch: String, cx: &mut Context<SpacesView>) {
    if let Form::Branch { busy, .. } = &mut v.form {
        if *busy {
            return;
        }
        *busy = true;
    }
    let id = id.to_string();
    let task = cx.background_executor().spawn({
        let branch = branch.clone();
        async move { git::add_worktree(&info, &branch) }
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            match result {
                Ok(folder) => {
                    set_form(v, Form::None, None);
                    cx.emit(SpacesEvent::Add { folder, name: branch });
                }
                Err(e) => {
                    if let Form::Branch { busy, .. } = &mut v.form {
                        *busy = false;
                    }
                    notice(cx, format!("Cannot make the worktree for {}: {e}", name_of(v, &id)));
                }
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn open_worktrees(v: &mut SpacesView, id: &str, cx: &mut Context<SpacesView>) {
    let Some(info) = need_git(v, id, cx) else {
        return;
    };
    let taken: Vec<String> = v
        .input
        .spaces
        .iter()
        .flat_map(|s| [s.folder.trim_end_matches('/').to_string(), place(v, s)])
        .collect();
    let id = id.to_string();
    let top = info.top.clone();
    let task = cx.background_executor().spawn(async move {
        let real = |p: &String| {
            std::fs::canonicalize(p)
                .map(|r| r.display().to_string())
                .unwrap_or_else(|_| p.clone())
        };
        let taken: Vec<String> = taken.iter().flat_map(|p| [p.clone(), real(p)]).collect();
        git::worktrees(&top).map(|list| (list, taken))
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            match result {
                Ok((list, taken)) => {
                    let list: Vec<Worktree> = list
                        .into_iter()
                        .filter(|w| !w.bare && !w.prunable && !taken.contains(&w.path))
                        .collect();
                    if list.is_empty() {
                        notice(cx, format!("{} has no other worktree to open.", info.name));
                    } else {
                        set_form(v, Form::Open { id, list }, None);
                    }
                }
                Err(e) => notice(cx, format!("Cannot list the worktrees of {}: {e}", info.name)),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn open_worktree(v: &mut SpacesView, index: usize, cx: &mut Context<SpacesView>) {
    let Form::Open { list, .. } = &v.form else {
        return;
    };
    let Some(w) = list.get(index).cloned() else {
        return;
    };
    set_form(v, Form::None, None);
    let name = w.branch.clone().unwrap_or_else(|| spaces::folder_name(&w.path));
    cx.emit(SpacesEvent::Add { folder: w.path, name });
}

pub fn ask_remove(v: &mut SpacesView, id: &str, cx: &mut Context<SpacesView>) {
    let Some(info) = need_git(v, id, cx) else {
        return;
    };
    if !info.linked {
        notice(cx, format!("Space {} is the main checkout. Only a worktree can be removed.", name_of(v, id)));
        return;
    }
    set_form(v, Form::Remove { id: id.to_string(), busy: false }, None);
}

pub fn confirm_remove(v: &mut SpacesView, cx: &mut Context<SpacesView>) {
    let Form::Remove { id, busy } = &mut v.form else {
        return;
    };
    if *busy {
        return;
    }
    *busy = true;
    let id = id.clone();
    let Some(info) = git_of(v, &id) else {
        set_form(v, Form::None, None);
        return;
    };
    let task = cx.background_executor().spawn(async move { git::remove_worktree(&info) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |v, cx| {
            set_form(v, Form::None, None);
            match result {
                Ok(()) => cx.emit(SpacesEvent::Close(id)),
                Err(e) => notice(cx, format!("Cannot remove the worktree of {}: {e}", name_of(v, &id))),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn ask_close(v: &mut SpacesView, id: &str, cx: &mut Context<SpacesView>) {
    let tabs = v.input.tabs.get(id).copied().unwrap_or(0);
    if tabs == 0 {
        set_form(v, Form::None, None);
        cx.emit(SpacesEvent::Close(id.to_string()));
    } else {
        set_form(v, Form::Close { id: id.to_string(), tabs }, None);
    }
}

pub fn confirm_close(v: &mut SpacesView, cx: &mut Context<SpacesView>) {
    if let Form::Close { id, .. } = &v.form {
        let id = id.clone();
        set_form(v, Form::None, None);
        cx.emit(SpacesEvent::Close(id));
    }
}
