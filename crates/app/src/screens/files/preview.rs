use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub file: String,
    pub tab: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Previews {
    pub preview: Option<Preview>,
    pub kept: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    Select(String),
    Open { keep: bool, replace: Option<String> },
}

pub fn forget_gone(p: &mut Previews, alive: &dyn Fn(&str) -> bool) {
    p.kept.retain(|_, tab| alive(tab));
    if p.preview.as_ref().is_some_and(|v| !alive(&v.tab)) {
        p.preview = None;
    }
}

pub fn plan(p: &mut Previews, file: &str, keep: bool, alive: &dyn Fn(&str) -> bool) -> Plan {
    forget_gone(p, alive);
    if let Some(tab) = p.kept.get(file) {
        return Plan::Select(tab.clone());
    }
    if let Some(v) = p.preview.clone() {
        if v.file == file {
            if keep {
                keep_preview(p);
            }
            return Plan::Select(v.tab);
        }
        if keep {
            return Plan::Open {
                keep,
                replace: None,
            };
        }
        return Plan::Open {
            keep,
            replace: Some(v.tab),
        };
    }
    Plan::Open {
        keep,
        replace: None,
    }
}

pub fn opened(p: &mut Previews, file: &str, keep: bool, tab: &str) {
    if keep {
        p.kept.insert(file.to_string(), tab.to_string());
    } else {
        p.preview = Some(Preview {
            file: file.to_string(),
            tab: tab.to_string(),
        });
    }
}

pub fn keep_preview(p: &mut Previews) {
    if let Some(v) = p.preview.take() {
        p.kept.insert(v.file, v.tab);
    }
}

pub fn renamed(p: &mut Previews, from: &str, to: &str) {
    let moved = |path: &str| -> Option<String> {
        if path == from {
            Some(to.to_string())
        } else {
            path.strip_prefix(&format!("{from}/"))
                .map(|rest| format!("{to}/{rest}"))
        }
    };
    if let Some(v) = p.preview.as_mut()
        && let Some(next) = moved(&v.file)
    {
        v.file = next;
    }
    p.kept = p
        .kept
        .drain()
        .map(|(file, tab)| (moved(&file).unwrap_or(file), tab))
        .collect();
}

pub const EDITORS: [&str; 3] = ["nano", "vi", "less"];

pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|dir| dir.join(name).is_file()))
}

pub fn pick_editor(found: &dyn Fn(&str) -> bool) -> &'static str {
    EDITORS.iter().copied().find(|e| found(e)).unwrap_or("less")
}

pub fn editor_argv(editor: &str, file: &str) -> Vec<String> {
    let file = if file.starts_with('-') {
        format!("./{file}")
    } else {
        file.to_string()
    };
    if editor == "less" {
        vec!["less".into(), "-R".into(), file]
    } else {
        vec![editor.to_string(), file]
    }
}

pub fn preview_argv(file: &str) -> Vec<String> {
    editor_argv(pick_editor(&on_path), file)
}

pub fn quit_keys(cmd: &[String]) -> Option<&'static str> {
    let name = std::path::Path::new(cmd.first()?).file_name()?.to_str()?;
    match name {
        "nano" | "pico" => Some("\u{18}"),
        "vi" | "vim" | "nvim" => Some("\u{1b}:q\r"),
        "less" => Some("q"),
        _ => None,
    }
}

pub fn keep_tab(p: &mut Previews, tab: &str) {
    if p.preview.as_ref().is_some_and(|v| v.tab == tab) {
        keep_preview(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn one_click_replaces_the_preview() {
        let mut p = Previews::default();
        assert_eq!(
            plan(&mut p, "/r/a", false, &all),
            Plan::Open {
                keep: false,
                replace: None
            }
        );
        opened(&mut p, "/r/a", false, "t1");
        assert_eq!(plan(&mut p, "/r/a", false, &all), Plan::Select("t1".into()));
        assert_eq!(
            plan(&mut p, "/r/b", false, &all),
            Plan::Open {
                keep: false,
                replace: Some("t1".into())
            }
        );
        opened(&mut p, "/r/b", false, "t2");
        assert_eq!(
            p.preview,
            Some(Preview {
                file: "/r/b".into(),
                tab: "t2".into()
            })
        );
        assert!(p.kept.is_empty());
    }

    #[test]
    fn double_click_keeps_it() {
        let mut p = Previews::default();
        opened(&mut p, "/r/a", false, "t1");
        assert_eq!(plan(&mut p, "/r/a", true, &all), Plan::Select("t1".into()));
        assert_eq!(p.preview, None);
        assert_eq!(p.kept.get("/r/a"), Some(&"t1".to_string()));
        assert_eq!(
            plan(&mut p, "/r/b", false, &all),
            Plan::Open {
                keep: false,
                replace: None
            }
        );
        opened(&mut p, "/r/b", false, "t2");
        assert_eq!(
            plan(&mut p, "/r/c", true, &all),
            Plan::Open {
                keep: true,
                replace: None
            }
        );
        opened(&mut p, "/r/c", true, "t3");
        assert_eq!(plan(&mut p, "/r/c", false, &all), Plan::Select("t3".into()));
        assert_eq!(p.preview.as_ref().map(|v| v.tab.as_str()), Some("t2"));
        assert_eq!(p.kept.len(), 2);
    }

    #[test]
    fn closed_tabs_are_forgotten() {
        let mut p = Previews::default();
        opened(&mut p, "/r/a", false, "t1");
        opened(&mut p, "/r/b", true, "t2");
        let none = |_: &str| false;
        assert_eq!(
            plan(&mut p, "/r/b", false, &none),
            Plan::Open {
                keep: false,
                replace: None
            }
        );
        assert_eq!(p, Previews::default());
    }

    #[test]
    fn preview_picks_the_first_editor_found() {
        assert_eq!(pick_editor(&|_: &str| true), "nano");
        assert_eq!(pick_editor(&|e: &str| e == "vi"), "vi");
        assert_eq!(pick_editor(&|_: &str| false), "less");
        assert_eq!(editor_argv("nano", "/r/a.rs"), vec!["nano", "/r/a.rs"]);
        assert_eq!(
            editor_argv("less", "/r/a.rs"),
            vec!["less", "-R", "/r/a.rs"]
        );
        assert_eq!(editor_argv("vi", "-x"), vec!["vi", "./-x"]);
    }

    #[test]
    fn quit_keys_ask_the_editor_to_close_itself() {
        let cmd = |c: &str| vec![c.to_string(), "/r/a".to_string()];
        assert_eq!(quit_keys(&cmd("nano")), Some("\u{18}"));
        assert_eq!(quit_keys(&cmd("/usr/bin/pico")), Some("\u{18}"));
        assert_eq!(quit_keys(&cmd("vi")), Some("\u{1b}:q\r"));
        assert_eq!(quit_keys(&cmd("less")), Some("q"));
        assert_eq!(quit_keys(&cmd("sh")), None);
        assert_eq!(quit_keys(&[]), None);
    }

    #[test]
    fn an_edited_preview_is_kept_not_replaced() {
        let mut p = Previews::default();
        opened(&mut p, "/r/a", false, "t1");
        keep_tab(&mut p, "t9");
        assert_eq!(p.preview.as_ref().map(|v| v.tab.as_str()), Some("t1"));
        keep_tab(&mut p, "t1");
        assert_eq!(p.preview, None);
        assert_eq!(p.kept.get("/r/a"), Some(&"t1".to_string()));
        opened(&mut p, "/r/b", false, "t2");
        assert_eq!(plan(&mut p, "/r/a", false, &all), Plan::Select("t1".into()));
    }

    #[test]
    fn rename_moves_records() {
        let mut p = Previews::default();
        opened(&mut p, "/r/src/a.rs", false, "t1");
        opened(&mut p, "/r/src/b.rs", true, "t2");
        opened(&mut p, "/r/srcx", true, "t3");
        renamed(&mut p, "/r/src", "/r/lib");
        assert_eq!(p.preview.as_ref().unwrap().file, "/r/lib/a.rs");
        assert_eq!(p.kept.get("/r/lib/b.rs"), Some(&"t2".to_string()));
        assert_eq!(p.kept.get("/r/srcx"), Some(&"t3".to_string()));
    }
}
