use super::context::BrainContext;
use super::view::{chat_tip, lights_pad, nixa_tip, shown, voice_tip, window_buttons};
use super::{Seen, agent_voice_of, build, menu_point, nixa_on_of, parse_offset, take, waiting_of};
use gpui::{point, px};
use serde_json::json;

#[test]
fn only_a_double_click_on_a_windowed_titlebar_dispatches_the_native_action() {
    use super::view::titlebar_double_click;

    assert!(!titlebar_double_click(1, false));
    assert!(titlebar_double_click(2, false));
    assert!(!titlebar_double_click(3, false));
    assert!(!titlebar_double_click(2, true));
}

#[test]
fn nixa_is_on_when_the_mic_or_voice_is_on() {
    let msg = |mic: bool, voice: bool| json!({"type": "voice", "voice": {"settings": {"micOn": mic, "voiceOn": voice}}});
    assert_eq!(nixa_on_of(&msg(true, false)), Some(true));
    assert_eq!(nixa_on_of(&msg(false, true)), Some(true));
    assert_eq!(nixa_on_of(&msg(false, false)), Some(false));
    assert_eq!(nixa_on_of(&json!({"ok": true})), None);
}

#[test]
fn waiting_counts_the_open_card() {
    assert_eq!(
        waiting_of(&json!({"type": "prompts", "waiting": 3, "card": {"id": "p1"}})),
        3
    );
    assert_eq!(
        waiting_of(&json!({"type": "prompts", "waiting": 0, "card": {"id": "p1"}})),
        1
    );
    assert_eq!(
        waiting_of(&json!({"type": "prompts", "waiting": 0, "card": null})),
        0
    );
}

#[test]
fn offsets_are_read() {
    assert_eq!(parse_offset("+0800\n"), Some(480));
    assert_eq!(parse_offset("-0530"), Some(-330));
    assert_eq!(parse_offset("0800"), None);
    assert_eq!(parse_offset("+08"), None);
}

#[test]
fn the_bar_leaves_room_for_the_window_lights() {
    assert_eq!(lights_pad(true, false, 268.0), 10.0);
    assert_eq!(lights_pad(true, false, 44.0), 34.0);
    assert_eq!(lights_pad(true, true, 44.0), 10.0);
    assert_eq!(lights_pad(false, false, 44.0), 10.0);
    assert!(!window_buttons(true));
    assert!(window_buttons(false));
}

#[test]
fn the_menu_opens_under_the_plus_button() {
    assert_eq!(
        menu_point(Some(point(px(900.0), px(20.0))), px(1280.0)),
        point(px(913.0), px(38.0))
    );
    assert_eq!(menu_point(None, px(1280.0)), point(px(1110.0), px(38.0)));
    assert_eq!(
        menu_point(Some(point(px(0.0), px(0.0))), px(1000.0)),
        point(px(830.0), px(38.0))
    );
}

#[test]
fn tips_follow_the_state() {
    let mut bar = build(None, 0);
    assert!(nixa_tip(&shown(&bar)).starts_with("Nixa: off. No mic and no voice: Nixa and agents are silent."));
    take(
        &mut bar,
        Seen {
            context: Some(Some(BrainContext {
                level: "ok".into(),
                used: 2000,
                budget: 100_000,
                left: 98.0,
                compact_at: 75,
                ..BrainContext::default()
            })),
            nixa_on: Some(true),
            agent_voice: Some(false),
            quick: None,
        },
    );
    let tip = nixa_tip(&shown(&bar));
    assert!(tip.starts_with("Nixa: on. Nixa listens and talks with you."));
    assert!(tip.contains("\nBrain context: 2k of 100k used, 98% left."));
    assert!(!bar.agent_voice);
    take(&mut bar, Seen::default());
    assert!(bar.nixa_on && bar.context.is_some());
    assert_eq!(
        chat_tip("Nixa", false, true),
        "Something waits for your answer. Show Nixa chat. Mic and voice stay unchanged."
    );
    assert!(chat_tip("Nixa", true, false).starts_with("Hide Nixa chat."));
    assert!(voice_tip("Nixa", false).starts_with("Agent voice: off."));
}

#[test]
fn agent_voice_comes_from_saved_settings_and_the_brain() {
    let conn = nixa_store::open_memory().unwrap();
    assert!(super::saved_agent_voice(Some(&conn)));
    nixa_store::settings::set_setting_bool(&conn, super::AGENT_VOICE_KEY, false).unwrap();
    assert!(!super::saved_agent_voice(Some(&conn)));
    assert_eq!(
        agent_voice_of(&json!({"type": "agent_voice", "on": false})),
        Some(false)
    );
    assert_eq!(
        agent_voice_of(&json!({"ok": true, "on": true, "events": []})),
        Some(true)
    );
    assert_eq!(agent_voice_of(&json!({"type": "agent_voice"})), None);
}

fn fake_bins(name: &str, bins: &[&str]) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("nixa-quick-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    for bin in bins {
        let path = dir.join(bin);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

#[test]
fn quick_agents_can_use_normal_checks() {
    use super::quick::{QUICK_AGENTS, run_of, tip};
    for agent in QUICK_AGENTS {
        let run = run_of(agent, false);
        assert!(!run.is_empty(), "{}", agent.id);
        assert_eq!(
            Some(run.join(" ").as_str()),
            nixa_store::agents::start_command_with(agent.provider, false),
            "{}",
            agent.id
        );
        assert!(
            !run.iter().any(|w| w.contains("dangerously") || w == "--yolo" || w == "-y" || w == "full-access"),
            "{}",
            agent.id
        );
        assert_eq!(tip(agent, false), format!("Open {}", agent.name));
    }
    let claude = &QUICK_AGENTS[0];
    assert_eq!(run_of(claude, false), ["claude"]);
    assert_eq!(run_of(claude, true), ["claude", "--dangerously-skip-permissions"]);
    assert_eq!(super::quick::off_flags(claude), "--dangerously-skip-permissions");
    assert_eq!(tip(claude, true), "Open Claude Code · checks off");
    let jcode = QUICK_AGENTS.iter().find(|a| a.id == "jcode").unwrap();
    assert!(!super::quick::can_turn_off(jcode));
    assert_eq!(tip(jcode, true), "Open Jcode");
}

#[test]
fn quick_buttons_show_only_installed_agents_and_their_switch() {
    use super::quick::{Button, QUICK_AGENTS, buttons, buttons_in, start};
    let dir = fake_bins("buttons", &["claude", "codex"]);
    let dirs = vec![dir.clone()];
    let at = |id: &str| QUICK_AGENTS.iter().position(|a| a.id == id).unwrap();
    assert_eq!(
        buttons(None, &dirs),
        vec![Button { index: at("claude"), off: true }, Button { index: at("codex"), off: true }]
    );
    let text = r#"{"programs": {}, "checksOff": ["claude", "codex"], "checksOn": ["codex", "hermes"]}"#;
    assert_eq!(
        buttons(Some(text), &dirs),
        vec![Button { index: at("claude"), off: true }, Button { index: at("codex"), off: false }]
    );
    assert!(buttons(Some(text), &[]).is_empty());
    let file = dir.join("programs.json");
    assert_eq!(buttons_in(&file, &dirs).len(), 2);
    std::fs::write(&file, text).unwrap();
    assert_eq!(buttons_in(&file, &dirs), buttons(Some(text), &dirs));
    assert_eq!(start(Button { index: 999, off: false }, None), None);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn quick_launch_preserves_checks_and_the_model_choice() {
    use super::quick::{Button, QUICK_AGENTS, start};
    let button = |id, off| Button { index: QUICK_AGENTS.iter().position(|a| a.id == id).unwrap(), off };
    assert_eq!(start(button("claude", false), None).unwrap().1, ["claude"]);
    assert_eq!(start(button("opencode", false), Some("test/model-v2")).unwrap().1,
        ["opencode", "--model", "test/model-v2"]);
    assert_eq!(start(button("codex", true), Some("test-model-v2")).unwrap().1,
        ["codex", "--dangerously-bypass-approvals-and-sandbox", "--model", "test-model-v2"]);
    assert!(start(button("codex", false), Some("--help")).is_none());
    assert!(start(button("jcode", false), Some("test-model-v2")).is_none());
}

#[gpui::test]
fn clicking_an_agent_launches_it_and_shows_no_model_menu(cx: &mut gpui::TestAppContext) {
    use super::quick::{Button, QUICK_AGENTS};
    use crate::workspace::Workspace;
    use gpui::{Modifiers, size};
    cx.update(nixa_ui::init);
    let (ws, cx) = cx.add_window_view(Workspace::new);
    cx.simulate_resize(size(px(1280.), px(800.)));
    ws.update(cx, |ws, cx| {
        ws.tab_bar.update(cx, |bar, _| {
            bar.quick = ["claude", "jcode"].into_iter().map(|id| Button {
                index: QUICK_AGENTS.iter().position(|a| a.id == id).unwrap(),
                off: false,
            }).collect();
        });
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("quick-agent-claude").is_some());
    assert!(cx.debug_bounds("quick-agent-jcode").is_some());
    assert!(cx.debug_bounds("quick-model-open-claude").is_none());
    assert!(cx.debug_bounds("quick-model-open-jcode").is_none());
    let at = cx.debug_bounds("quick-agent-claude").unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("quick-model-strip").is_none());
}

#[test]
fn saved_model_is_kept_per_agent_across_reads() {
    use super::prefs;
    let dir = std::env::temp_dir().join(format!("nixa-quick-prefs-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let file = dir.join("quick_models.json");
    assert_eq!(prefs::model_for(&file, "codex"), None);
    prefs::set_model(&file, "codex", Some("gpt-6.2-sol")).unwrap();
    prefs::set_model(&file, "claude", Some("claude-opus-5-5")).unwrap();
    assert_eq!(prefs::model_for(&file, "codex").as_deref(), Some("gpt-6.2-sol"));
    assert_eq!(prefs::model_for(&file, "claude").as_deref(), Some("claude-opus-5-5"));
    assert_eq!(prefs::model_for(&file, "opencode"), None);
    prefs::set_model(&file, "codex", None).unwrap();
    assert_eq!(prefs::model_for(&file, "codex"), None);
    assert_eq!(prefs::model_for(&file, "claude").as_deref(), Some("claude-opus-5-5"));
    std::fs::write(&file, "not json").unwrap();
    assert_eq!(prefs::model_for(&file, "claude"), None);
    assert!(prefs::set_model(&file, "claude", Some("claude-haiku-5-5")).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saved_model_is_the_command_and_no_saved_model_uses_the_default() {
    use super::models::saved_command;
    use super::prefs;
    use super::quick::QUICK_AGENTS;
    let dir = std::env::temp_dir().join(format!("nixa-quick-chip-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let file = dir.join("quick_models.json");
    let codex = QUICK_AGENTS.iter().position(|a| a.id == "codex").unwrap();
    let claude = QUICK_AGENTS.iter().position(|a| a.id == "claude").unwrap();
    let last_two = |run: &[String]| (run[run.len() - 2].clone(), run[run.len() - 1].clone());

    prefs::set_model(&file, "codex", Some("gpt-6.2-sol")).unwrap();
    let (name, run) = saved_command(&file, codex).unwrap();
    assert_eq!(name, "Codex");
    assert_eq!(last_two(&run), ("--model".into(), "gpt-6.2-sol".into()));
    let (_, again) = saved_command(&file, codex).unwrap();
    assert_eq!(last_two(&again), ("--model".into(), "gpt-6.2-sol".into()));

    let (_, claude_run) = saved_command(&file, claude).unwrap();
    assert!(!claude_run.iter().any(|word| word == "--model"));

    prefs::set_model(&file, "codex", None).unwrap();
    let (_, default_run) = saved_command(&file, codex).unwrap();
    assert!(!default_run.iter().any(|word| word == "--model"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saved_model_is_the_launch_model_and_no_preference_uses_the_agent_default() {
    use super::quick::{Button, QUICK_AGENTS, start};
    let codex = Button { index: QUICK_AGENTS.iter().position(|a| a.id == "codex").unwrap(), off: false };
    assert_eq!(start(codex, Some("gpt-6.2-sol")).unwrap().1, ["codex", "--model", "gpt-6.2-sol"]);
    assert_eq!(start(codex, None).unwrap().1, ["codex"]);
    let claude = Button { index: QUICK_AGENTS.iter().position(|a| a.id == "claude").unwrap(), off: false };
    assert_eq!(start(claude, Some("claude-opus-5-5")).unwrap().1, ["claude", "--model", "claude-opus-5-5"]);
    assert_eq!(start(claude, None).unwrap().1, ["claude"]);
}

#[gpui::test]
fn toolbar_bounds_and_double_click_routing(cx: &mut gpui::TestAppContext) {
    use crate::workspace::Workspace;
    use gpui::{AppContext, Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render, Window, div, size};
    use gpui::prelude::*;
    use std::{cell::Cell, rc::Rc};

    struct Toolbar {
        workspace: Entity<Workspace>,
        dispatched: Rc<Cell<usize>>,
    }

    impl Render for Toolbar {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let dispatched = self.dispatched.clone();
            let toolbar = self.workspace.update(cx, |workspace, cx| {
                super::view::render_with_titlebar_action(workspace, window, cx, move |_| {
                    dispatched.set(dispatched.get() + 1);
                })
            });
            div().flex().flex_col().size_full().child(toolbar).child(
                div().debug_selector(|| "fixture-tab".into()).h(px(30.)).child("Tab"),
            )
        }
    }

    cx.update(nixa_ui::init);
    let dispatched = Rc::new(Cell::new(0));
    let recorded = dispatched.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.tab_bar.update(cx, |bar, _| {
                bar.quick = vec![super::quick::Button { index: 0, off: false }];
            });
        });
        Toolbar { workspace, dispatched: recorded }
    });
    cx.simulate_resize(size(px(1280.), px(800.)));
    cx.run_until_parked();
    let bar = cx.debug_bounds("tab-bar").unwrap();
    let blank = cx.debug_bounds("titlebar-drag-space").unwrap();
    for id in ["tab-settings", "tab-nixa", "tab-nixa-card", "tab-nixa-core", "tab-voice"] {
        let bounds = cx.debug_bounds(id).unwrap();
        assert!(bounds.right() <= blank.left(), "{id} must be left of the drag space");
    }
    let quick_id = format!("quick-agent-{}", super::quick::QUICK_AGENTS[0].id);
    let quick = cx.debug_bounds(Box::leak(quick_id.into_boxed_str())).unwrap();
    let plus = cx.debug_bounds("tab-new").unwrap();
    assert!(quick.left() >= blank.right());
    assert!(quick.left() >= bar.center().x);
    assert!(quick.right() <= plus.left());
    assert!(plus.right() <= bar.right());

    let click = |cx: &mut gpui::VisualTestContext, position, click_count| {
        let button = MouseButton::Left;
        let modifiers = Modifiers::none();
        cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count, first_mouse: false });
        cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count });
    };
    click(cx, blank.center(), 1);
    assert_eq!(dispatched.get(), 0);
    click(cx, blank.center(), 2);
    assert_eq!(dispatched.get(), 1);
    click(cx, quick.center(), 2);
    assert_eq!(dispatched.get(), 1);
    let tab = cx.debug_bounds("fixture-tab").unwrap();
    click(cx, tab.center(), 2);
    assert_eq!(dispatched.get(), 1);
}

#[test]
fn seen_quick_list_replaces_the_bar_list() {
    use super::quick::Button;
    let mut bar = build(None, 0);
    assert!(bar.quick.is_empty());
    let list = vec![Button { index: 1, off: true }];
    take(&mut bar, Seen { quick: Some(list.clone()), ..Seen::default() });
    assert_eq!(bar.quick, list);
    take(&mut bar, Seen::default());
    assert_eq!(bar.quick, list);
}

#[gpui::test]
#[ignore]
fn live_engine_icon_click_uses_the_saved_model_or_the_default(cx: &mut gpui::TestAppContext) {
    use super::quick::{Button, QUICK_AGENTS};
    use crate::workspace::{EngineState, Workspace};
    use gpui::{Modifiers, size};
    use nixa_protocol::paths;
    let Some(dir) = std::env::var_os("NIXA_T79_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    unsafe {
        std::env::set_var(paths::ENV_DATA_DIR, &dir);
        std::env::set_var(paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
    }
    let folder = dir.join("space");
    cx.update(nixa_ui::init);
    let (ws, cx) = cx.add_window_view(Workspace::new);
    cx.simulate_resize(size(px(1280.), px(800.)));
    let at = |id: &str| QUICK_AGENTS.iter().position(|a| a.id == id).unwrap();
    ws.update(cx, |ws, cx| {
        ws.engine = EngineState::Up;
        ws.tabs_path = dir.join("tabs.json");
        ws.spaces = Default::default();
        crate::spaces::add(&mut ws.spaces, folder.to_str().unwrap());
        ws.spaces.active = Some(ws.spaces.spaces[0].id.clone());
        ws.tab_bar.update(cx, |bar, _| {
            bar.quick = ["claude", "codex"].into_iter().map(|id| Button { index: at(id), off: false }).collect();
        });
        cx.notify();
    });
    cx.run_until_parked();
    let click = |cx: &mut gpui::VisualTestContext, key: &'static str| {
        let point = cx.debug_bounds(key).unwrap_or_else(|| panic!("no element {key}")).center();
        cx.simulate_click(point, Modifiers::none());
        cx.run_until_parked();
    };
    let tabs = |ws: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        ws.read_with(cx, |ws, _| (ws.tabs.list.len(), ws.tabs.pending.len(), ws.notice.clone()))
    };
    let prefs = dir.join("quick_models.json");
    assert!(!prefs.exists());
    super::prefs::set_model(&prefs, "codex", Some("gpt-6.2-sol")).unwrap();

    click(cx, "quick-agent-codex");
    for _ in 0..50 {
        if tabs(&ws, cx).0 >= 1 { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
        cx.run_until_parked();
    }
    eprintln!("T79 after codex icon: tabs {:?}", tabs(&ws, cx));

    click(cx, "quick-agent-claude");
    for _ in 0..50 {
        if tabs(&ws, cx).0 >= 2 { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
        cx.run_until_parked();
    }
    eprintln!("T79 after claude icon: tabs {:?} prefs {}", tabs(&ws, cx), std::fs::read_to_string(&prefs).unwrap());
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&prefs).unwrap()).unwrap();
    assert_eq!(saved, json!({"codex": "gpt-6.2-sol"}));
    assert_eq!(tabs(&ws, cx).0, 2);
}
