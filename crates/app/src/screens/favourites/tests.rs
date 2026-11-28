use super::{data, place};
use super::store::{self, Favourite};
use super::view::{AskStep, Favourites, FavouritesEvent, Mode};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mo-w36-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn project(name: &str) -> (PathBuf, PathBuf) {
    let base = temp(name);
    let proj = base.join("proj");
    fs::create_dir_all(proj.join("docs")).expect("project");
    fs::write(proj.join("docs/plan.md"), "plan").expect("file");
    (base.join("fav.json"), fs::canonicalize(proj).expect("real"))
}

fn linked(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replacen("/private/", "/", 1))
}

#[test]
fn save_load_and_remove_per_project() {
    let (file, proj) = project("store");
    let other = proj.with_file_name("other");
    assert!(store::load(&file, &proj).expect("empty").is_empty());
    store::save(&file, &proj, "Plan", "docs/plan.md").expect("save");
    store::save(&file, &proj, "", "https://example.com").expect("save");
    store::save(&file, &other, "Other", "a.txt").expect("save");
    store::save(&file, &proj, "The plan", "docs/plan.md").expect("rename");
    assert_eq!(
        store::load(&file, &proj).expect("load"),
        vec![
            Favourite {
                title: "The plan".into(),
                url: "docs/plan.md".into()
            },
            Favourite {
                title: "https://example.com".into(),
                url: "https://example.com".into()
            },
        ]
    );
    let mode = fs::metadata(&file).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(store::remove(&file, &proj, "docs/plan.md").expect("remove"));
    assert!(!store::remove(&file, &proj, "docs/plan.md").expect("again"));
    assert_eq!(store::load(&file, &proj).expect("load").len(), 1);
    assert_eq!(store::load(&file, &other).expect("other").len(), 1);
    fs::write(&file, "{ broken").expect("write");
    assert!(store::load(&file, &proj).is_err());
}

#[test]
fn saves_at_the_same_time_do_not_fail() {
    let (file, proj) = project("race");
    let workers: Vec<_> = (0..8)
        .map(|n| {
            let file = file.clone();
            let proj = proj.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    store::save(&file, &proj, "t", &format!("docs/{n}-{i}.md")).expect("save");
                }
            })
        })
        .collect();
    for w in workers {
        w.join().expect("worker");
    }
    assert!(!store::load(&file, &proj).expect("valid json").is_empty());
    let left: Vec<_> = fs::read_dir(file.parent().expect("dir"))
        .expect("list")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(left.is_empty());
}

#[test]
fn paths_stay_inside_the_project() {
    let (_, real_proj) = project("place");
    let proj = linked(&real_proj);
    assert_ne!(proj, real_proj);
    assert_eq!(
        place::resolve("docs/plan.md", &proj).expect("inside"),
        real_proj.join("docs/plan.md")
    );
    assert_eq!(
        place::resolve("docs/new/../later.md", &proj).expect("missing is fine"),
        real_proj.join("docs/later.md")
    );
    assert!(place::resolve("../escape.txt", &proj).is_err());
    assert!(place::resolve("/etc/hosts", &proj).is_err());
    let twin = proj.with_file_name("proj2");
    fs::create_dir_all(&twin).expect("twin");
    assert!(place::resolve(&format!("{}/x", twin.display()), &proj).is_err());
    std::os::unix::fs::symlink(proj.parent().expect("parent"), proj.join("up")).expect("link");
    assert!(place::resolve("up/fav.json", &proj).is_err());
    let url = format!("file://{}/docs/my%20plan.md", proj.display());
    assert_eq!(
        place::resolve(&url, &proj).expect("file url"),
        real_proj.join("docs/my plan.md")
    );
    assert_eq!(place::percent_decode("a%2Fb%zz%4"), "a/b%zz%4");
    assert_eq!(
        place::stored_value(&format!("{}/docs/plan.md", proj.display()), &proj).expect("abs"),
        "docs/plan.md"
    );
    assert_eq!(place::stored_value(".", &proj).expect("root"), ".");
    assert_eq!(
        place::stored_value(" HTTPS://x.dev/a ", &proj).expect("web"),
        "HTTPS://x.dev/a"
    );
    assert_eq!(
        place::stored_value("../x", &proj).unwrap_err().to_string(),
        place::OUTSIDE
    );
    assert!(place::stored_value("  ", &proj).is_err());
}

#[test]
fn project_root_finds_the_git_top() {
    let (_, real) = project("git");
    let proj = linked(&real);
    assert_eq!(
        place::project_root(&proj.join("docs")),
        fs::canonicalize(proj.join("docs")).expect("real")
    );
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(&proj)
        .args(["init", "-q"])
        .status()
        .expect("git")
        .success();
    assert!(ok);
    assert_eq!(
        fs::canonicalize(place::project_root(&proj.join("docs"))).expect("top"),
        fs::canonicalize(&proj).expect("proj")
    );
}

type Seen = Rc<RefCell<Vec<FavouritesEvent>>>;

struct Host {
    view: Entity<Favourites>,
}

impl gpui::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div().size_full().relative().child(self.view.clone())
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    folder: Option<&Path>,
    file: &Path,
) -> (Entity<Favourites>, &'a mut VisualTestContext, Seen) {
    cx.update(nixa_ui::init);
    let holder: Rc<RefCell<Option<Entity<Favourites>>>> = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let folder = folder.map(Path::to_path_buf);
    let file = file.to_path_buf();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let view = super::view::new(folder, file, window, cx);
        *slot.borrow_mut() = Some(view.clone());
        Host { view }
    });
    let view = holder.borrow().clone().expect("view");
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &FavouritesEvent, _| {
            log.borrow_mut().push(e.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, seen)
}

fn events(seen: &Seen) -> Vec<FavouritesEvent> {
    std::mem::take(&mut *seen.borrow_mut())
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.update(|window, app| window.draw(app).clear());
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn rows_open_with_keys_and_clicks(cx: &mut TestAppContext) {
    let (file, proj) = project("open");
    store::save(&file, &proj, "Plan", "docs/plan.md").expect("save");
    store::save(&file, &proj, "Site", "https://example.com").expect("save");
    store::save(&file, &proj, "Gone", "docs/gone.md").expect("save");
    store::save(&file, &proj, "Out", "../fav.json").expect("save");
    let (view, vcx, seen) = open(cx, Some(&proj), &file);
    assert_eq!(vcx.update(|_, cx| view.read(cx).rows.len()), 4);
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).project.clone()),
        Some(fs::canonicalize(&proj).expect("real"))
    );
    vcx.simulate_keystrokes("enter");
    let real = fs::canonicalize(proj.join("docs/plan.md")).expect("real");
    assert_eq!(events(&seen), vec![FavouritesEvent::EditFile(real)]);
    vcx.simulate_keystrokes("down enter");
    assert_eq!(
        events(&seen),
        vec![FavouritesEvent::OpenUrl("https://example.com".into())]
    );
    click(vcx, "fav-2");
    assert!(matches!(
        events(&seen).as_slice(),
        [FavouritesEvent::Notice { error: true, .. }]
    ));
    click(vcx, "fav-3");
    assert_eq!(
        events(&seen),
        vec![FavouritesEvent::Notice {
            text: place::OUTSIDE.into(),
            error: true
        }]
    );
    vcx.simulate_keystrokes("escape");
    assert_eq!(events(&seen), vec![FavouritesEvent::Close]);
}

#[gpui::test]
fn add_asks_for_a_link_then_a_title(cx: &mut TestAppContext) {
    let (file, proj) = project("add");
    let (view, vcx, seen) = open(cx, Some(&proj), &file);
    click(vcx, "fav-add");
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).mode.clone()),
        Mode::Ask(AskStep::Link)
    );
    vcx.simulate_input("../secret.txt");
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).ask_error.clone()),
        Some(place::OUTSIDE.to_string())
    );
    vcx.simulate_keystrokes("cmd-a backspace");
    vcx.simulate_input(&format!("{}/docs/plan.md", proj.display()));
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).mode.clone()),
        Mode::Ask(AskStep::Title("docs/plan.md".into()))
    );
    vcx.simulate_input("Plan");
    click(vcx, "ask-save");
    assert_eq!(vcx.update(|_, cx| view.read(cx).mode.clone()), Mode::List);
    assert_eq!(
        store::load(&file, &proj).expect("load"),
        vec![Favourite {
            title: "Plan".into(),
            url: "docs/plan.md".into()
        }]
    );
    assert_eq!(
        events(&seen),
        vec![FavouritesEvent::Notice {
            text: "Saved".into(),
            error: false
        }]
    );
    click(vcx, "fav-add");
    vcx.simulate_input("https://nixa.dev");
    vcx.simulate_keystrokes("enter enter");
    assert_eq!(vcx.update(|_, cx| view.read(cx).rows.len()), 2);
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).rows[1].title.clone()),
        "https://nixa.dev"
    );
    click(vcx, "fav-add");
    vcx.simulate_keystrokes("escape");
    assert_eq!(vcx.update(|_, cx| view.read(cx).mode.clone()), Mode::List);
}

#[gpui::test]
fn remove_asks_first(cx: &mut TestAppContext) {
    let (file, proj) = project("remove");
    store::save(&file, &proj, "Plan", "docs/plan.md").expect("save");
    store::save(&file, &proj, "Site", "https://example.com").expect("save");
    let (view, vcx, seen) = open(cx, Some(&proj), &file);
    vcx.simulate_keystrokes("down");
    click(vcx, "fav-remove");
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).mode.clone()),
        Mode::Confirm(1)
    );
    click(vcx, "rm-cancel");
    assert_eq!(store::load(&file, &proj).expect("load").len(), 2);
    click(vcx, "fav-remove");
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        store::load(&file, &proj).expect("load"),
        vec![Favourite {
            title: "Plan".into(),
            url: "docs/plan.md".into()
        }]
    );
    assert_eq!(
        events(&seen),
        vec![FavouritesEvent::Notice {
            text: "Removed".into(),
            error: false
        }]
    );
    assert_eq!(vcx.update(|_, cx| view.read(cx).selected), 0);
}

#[gpui::test]
fn no_folder_means_no_project(cx: &mut TestAppContext) {
    let (file, _) = project("none");
    let (view, vcx, seen) = open(cx, None, &file);
    assert_eq!(vcx.update(|_, cx| view.read(cx).project.clone()), None);
    vcx.update(|window, cx| view.update(cx, |v, cx| v.begin_add(window, cx)));
    assert_eq!(vcx.update(|_, cx| view.read(cx).mode.clone()), Mode::List);
    assert!(matches!(
        events(&seen).as_slice(),
        [FavouritesEvent::Notice { error: true, .. }]
    ));
}

#[test]
fn text_files_are_told_apart_from_others() {
    let base = temp("text");
    let write = |name: &str, bytes: &[u8]| {
        let p = base.join(name);
        fs::write(&p, bytes).expect("file");
        p
    };
    assert!(place::is_text(&write("a.md", b"# plan\n")));
    assert!(place::is_text(&write("empty.txt", b"")));
    assert!(place::is_text(&write("uni.txt", "caf\u{e9} \u{1f600}".as_bytes())));
    let mut long = format!("a{}", "\u{e9}".repeat(place::SNIFF as usize)).into_bytes();
    long.truncate(place::SNIFF as usize + 1);
    assert!(place::is_text(&write("long.txt", &long)));
    assert!(!place::is_text(&write("pic.png", &[0x89, b'P', b'N', b'G', 0, 0, 0, 13])));
    assert!(!place::is_text(&write("latin.txt", &[b'c', b'a', b'f', 0xe9, b'!'])));
    assert!(!place::is_text(&base.join("missing.txt")));
    assert!(!place::is_text(&base));
}

#[gpui::test]
fn text_favourite_opens_in_the_editor_and_others_open_outside(cx: &mut TestAppContext) {
    let (file, proj) = project("edit");
    fs::write(proj.join("logo.png"), [0x89, b'P', b'N', b'G', 0, 0]).expect("file");
    fs::create_dir_all(proj.join("assets")).expect("folder");
    store::save(&file, &proj, "Plan", "docs/plan.md").expect("save");
    store::save(&file, &proj, "Logo", "logo.png").expect("save");
    store::save(&file, &proj, "Assets", "assets").expect("save");
    let (_, vcx, seen) = open(cx, Some(&proj), &file);
    vcx.simulate_keystrokes("enter");
    let plan = fs::canonicalize(proj.join("docs/plan.md")).expect("real");
    assert_eq!(events(&seen), vec![FavouritesEvent::EditFile(plan.clone())]);
    vcx.simulate_keystrokes("down enter");
    let logo = fs::canonicalize(proj.join("logo.png")).expect("real");
    assert_eq!(events(&seen), vec![FavouritesEvent::OpenFile(logo)]);
    vcx.simulate_keystrokes("down enter");
    let assets = fs::canonicalize(proj.join("assets")).expect("real");
    assert_eq!(events(&seen), vec![FavouritesEvent::OpenFile(assets)]);
    let before = vcx.update(|_, cx| cx.windows().len());
    vcx.update(|_, cx| data::edit(&plan, cx));
    vcx.run_until_parked();
    let editors: Vec<String> = vcx.update(|_, cx| {
        cx.windows()
            .into_iter()
            .filter_map(|w| w.downcast::<nixa_editor::Editor>())
            .filter_map(|w| w.read(cx).ok().and_then(|ed| ed.path().map(str::to_string)))
            .collect()
    });
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), before + 1);
    assert_eq!(editors, vec![plan.to_string_lossy().to_string()]);
}
