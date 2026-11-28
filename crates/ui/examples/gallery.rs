use gpui::{
    App, AppContext, Application, Axis, Bounds, Context, Corner, DragMoveEvent, Entity,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    point, prelude::FluentBuilder, px, size,
};
use nixa_protocol::Status;
use nixa_ui::button::{ButtonKind, button, disabled, icon_label_button};
use nixa_ui::chip::{chip, count_chip, model_chip, outline_chip, status_chip, status_dot};
use nixa_ui::icon_button::{IconSize, attention_dot, count_badge, icon_button, tinted_icon_button};
use nixa_ui::icons::{Icon, icon};
use nixa_ui::list::{icon_lead, list, row, selection_bar};
use nixa_ui::menu::{MenuItem, MenuKey, OnPick, menu, menu_key};
use nixa_ui::metrics::{
    BRAND_HEIGHT, PANE_GAP, RAIL_WIDTH, SIDEBAR_FOOTER_HEIGHT, SIDEBAR_MAX, SIDEBAR_MIN,
    SIDEBAR_WIDTH, STATUS_BAR_HEIGHT, TRAFFIC_LIGHTS_SPACE, TRAFFIC_LIGHTS_X, TRAFFIC_LIGHTS_Y,
};
use nixa_ui::modal::{dialog, overlay};
use nixa_ui::popover::{float_at, popover, tooltip};
use nixa_ui::scroll::scroll_area;
use nixa_ui::section::{actions, group_title, section_header, short_rule};
use nixa_ui::split::{Edge, SplitDrag, size_from_drag, split_handle};
use nixa_ui::tab::{closable_tab, pane, pane_header, segment, segmented, tab, tab_bar};
use nixa_ui::text::{self, caption, heading, kbd, label, mono, muted, strong, title};
use nixa_ui::text_input::{InputEvent, TextInput};
use nixa_ui::theme::{Theme, status_color, theme};
use nixa_ui::toast::{ToastKind, Toasts, toast_card, toast_layer, toast_stack};
use nixa_ui::toggle::toggle_row;
use std::rc::Rc;
use std::time::Duration;

struct Space {
    name: &'static str,
    shape: Icon,
    color: fn(&Theme) -> gpui::Hsla,
    blocked: usize,
    done: usize,
}

struct Agent {
    name: &'static str,
    status: Status,
    when: &'static str,
}

const SPACES: [Space; 4] = [
    Space { name: "nixa", shape: Icon::Hexagon, color: |t| t.cyan, blocked: 1, done: 2 },
    Space { name: "nixa", shape: Icon::Circle, color: |t| t.magenta, blocked: 0, done: 1 },
    Space { name: "my-web-agent", shape: Icon::Triangle, color: |t| t.orange, blocked: 0, done: 0 },
    Space { name: "shop-app", shape: Icon::Diamond, color: |t| t.green, blocked: 2, done: 0 },
];

const AGENTS: [Agent; 5] = [
    Agent { name: "Senior Developer", status: Status::Working, when: "2m" },
    Agent { name: "Tester", status: Status::Blocked, when: "5m" },
    Agent { name: "Reviewer", status: Status::Done, when: "12m" },
    Agent { name: "Docs writer", status: Status::Idle, when: "1h" },
    Agent { name: "Builder", status: Status::Error, when: "3h" },
];

fn context_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("Split right").icon(Icon::PanelRight).shortcut("⌘D"),
        MenuItem::new("Split down").icon(Icon::PanelRight).shortcut("⇧⌘D"),
        MenuItem::new("Zoom").icon(Icon::Square).checked(true),
        MenuItem::new("Rename").icon(Icon::SquarePen),
        MenuItem::new("Send back to parent").icon(Icon::Undo).disabled(true),
        MenuItem::new("Quick actions").icon(Icon::Zap).separated(),
        MenuItem::new("Close pane").icon(Icon::X).danger().separated(),
    ]
}

struct Gallery {
    focus: FocusHandle,
    sidebar_width: f32,
    space: usize,
    agent: usize,
    tab: usize,
    segment: usize,
    toggles: [bool; 3],
    menu_at: Option<usize>,
    context: Option<Point<Pixels>>,
    modal_open: bool,
    toasts: Toasts,
    input: Entity<TextInput>,
    search: Entity<TextInput>,
    scroll: ScrollHandle,
    list_scroll: ScrollHandle,
    note: SharedString,
}

impl Focusable for Gallery {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Gallery {
    fn new(cx: &mut Context<Self>, overlays: bool) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Ask Nixa or type a command");
            input.set_value("Split the pane and start Codex", cx);
            input
        });
        let search = cx.new(|cx| TextInput::new(cx, "Search agents").with_icon(Icon::Search));
        cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            this.note = match event {
                InputEvent::Changed(v) => format!("Typing: {v}").into(),
                InputEvent::Submit(v) => format!("Sent: {v}").into(),
                InputEvent::Cancel => "Input cancelled".into(),
            };
            cx.notify();
        })
        .detach();
        let mut toasts = Toasts::default();
        toasts.push(ToastKind::Success, "Reviewer finished", Some("All 14 checks passed in nixa.".into()));
        toasts.push(ToastKind::Warning, "Tester needs you", Some("Waiting for a yes on the data reset.".into()));
        Gallery {
            focus: cx.focus_handle(),
            sidebar_width: SIDEBAR_WIDTH,
            space: 0,
            agent: 1,
            tab: 0,
            segment: 0,
            toggles: [true, false, true],
            menu_at: Some(2),
            context: overlays.then(|| point(px(760.0), px(250.0))),
            modal_open: overlays,
            toasts,
            input,
            search,
            scroll: ScrollHandle::new(),
            list_scroll: ScrollHandle::new(),
            note: "Right-click anywhere for the context menu".into(),
        }
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        let items = context_items();
        self.note = format!("Picked: {}", items[index].label).into();
        self.context = None;
        self.menu_at = Some(index);
        cx.notify();
    }

    fn picker(cx: &mut Context<Self>) -> OnPick {
        let this = cx.entity().downgrade();
        Rc::new(move |index, _window, cx| {
            this.update(cx, |g, cx| g.pick(index, cx)).ok();
        })
    }

    fn key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let items = context_items();
        match menu_key(&items, self.menu_at, event.keystroke.key.as_str()) {
            MenuKey::Move(i) => self.menu_at = Some(i),
            MenuKey::Choose(i) => self.pick(i, cx),
            MenuKey::Close => {
                self.context = None;
                self.modal_open = false;
            }
            MenuKey::Ignore => return,
        }
        cx.notify();
    }

    fn sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let spaces = SPACES.iter().enumerate().map(|(i, s)| {
            let color = (s.color)(t);
            let trailing = div()
                .flex()
                .gap(px(4.0))
                .when(s.blocked > 0, |el| el.child(count_chip(s.blocked, t.yellow)))
                .when(s.done > 0, |el| el.child(count_chip(s.done, t.green)));
            row(
                t,
                ("space", i),
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .child(icon_lead(s.shape, color))
                        .child(div().text_size(px(11.0)).text_color(t.magenta).child((i + 1).to_string()))
                        .into_any_element(),
                ),
                s.name,
                Some(trailing.into_any_element()),
                i == self.space,
            )
            .when(i == self.space, |el| el.child(selection_bar(t.accent)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.space = i;
                cx.notify();
            }))
        });
        let agents = AGENTS.iter().enumerate().map(|(i, a)| {
            let trailing = if a.status == Status::Blocked {
                status_chip(t, a.status).into_any_element()
            } else {
                caption(t, a.when).text_size(px(11.0)).into_any_element()
            };
            row(
                t,
                ("agent", i),
                Some(div().w(px(14.0)).flex().justify_center().child(status_dot(t, ("agent-dot", i), a.status)).into_any_element()),
                a.name,
                Some(trailing),
                i == self.agent,
            )
            .tooltip(tooltip(format!("{}: {}", a.name, nixa_ui::theme::status_label(a.status))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.agent = i;
                cx.notify();
            }))
        });
        div()
            .flex_none()
            .w(px(self.sidebar_width))
            .h_full()
            .flex()
            .flex_col()
            .bg(t.chrome)
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .h(px(BRAND_HEIGHT))
                    .pl(px(TRAFFIC_LIGHTS_SPACE))
                    .pr(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(Icon::Core).size(px(18.0)).text_color(t.brand))
                    .child(div().text_size(px(text::MD)).font_weight(FontWeight::BOLD).text_color(t.bright).child("Nixa"))
                    .child(caption(t, "OSS")),
            )
            .child(
                section_header(t, "Spaces").child(
                    actions()
                        .child(icon_button(t, "space-folder", Icon::FolderPlus, IconSize::Toolbar, false).tooltip(tooltip("New space from a folder")))
                        .child(icon_button(t, "space-new", Icon::Plus, IconSize::Toolbar, false).tooltip(tooltip("New space"))),
                ),
            )
            .child(list(2.0).children(spaces))
            .child(
                section_header(t, "Agents").child(
                    actions().child(icon_button(t, "agent-sort", Icon::Layers, IconSize::Toolbar, false).tooltip(tooltip("Sort agents"))),
                ),
            )
            .child(list(2.0).children(agents))
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .h(px(SIDEBAR_FOOTER_HEIGHT))
                    .px(px(8.0))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        row(
                            t,
                            "footer-settings",
                            Some(icon_lead(Icon::Settings, t.muted)),
                            "Settings",
                            Some(kbd(t, "⌘,").into_any_element()),
                            false,
                        ),
                    ),
            )
    }

    fn top_bar(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let tabs = [("Senior Developer", Status::Working), ("Tester", Status::Blocked), ("Shell", Status::Idle)];
        tab_bar(t)
            .children(tabs.iter().enumerate().map(|(i, (name, status))| {
                closable_tab(t, ("tab", i), *name, Some(status_dot(t, ("tab-dot", i), *status)), i == self.tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = i;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(icon_button(t, "tab-new", Icon::Plus, IconSize::Toolbar, false).tooltip(tooltip("New tab")))
            .child(
                div()
                    .id("tab-nixa")
                    .h(px(26.0))
                    .px(px(7.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .rounded(px(6.0))
                    .text_size(px(text::SM))
                    .text_color(t.cyan)
                    .cursor_pointer()
                    .child(div().size(px(7.0)).rounded_full().bg(t.cyan))
                    .child(icon(Icon::Mic).size(px(13.0)).text_color(t.cyan))
                    .child("Nixa")
                    .child(div().text_size(px(10.5)).text_color(t.muted).child("82%")),
            )
            .child(
                icon_button(t, "tab-chat", Icon::Chat, IconSize::Toolbar, false)
                    .border_1()
                    .border_color(t.border)
                    .child(attention_dot(t.yellow)),
            )
            .child(tinted_icon_button(t, "tab-core", Icon::Core, IconSize::Toolbar, false, t.cyan))
            .child(icon_button(t, "tab-voice", Icon::VoiceOn, IconSize::Toolbar, false))
    }

    fn rail(&self, t: &Theme) -> gpui::Div {
        div()
            .flex_none()
            .w(px(RAIL_WIDTH))
            .h_full()
            .py(px(8.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.0))
            .bg(t.chrome)
            .border_l_1()
            .border_color(t.border)
            .child(icon_button(t, "rail-commands", Icon::Commands, IconSize::Rail, false).tooltip(tooltip("Commands")))
            .child(
                tinted_icon_button(t, "rail-tasks", Icon::Tasks, IconSize::Rail, false, t.yellow)
                    .child(count_badge(t, 3, t.yellow))
                    .tooltip(tooltip("Tasks: 3 waiting for you")),
            )
            .child(short_rule(t))
            .child(icon_button(t, "rail-files", Icon::Files, IconSize::Rail, true).tooltip(tooltip("Files panel")))
            .child(icon_button(t, "rail-git", Icon::GitBranch, IconSize::Rail, false).tooltip(tooltip("Git panel")))
            .child(icon_button(t, "rail-phone", Icon::Smartphone, IconSize::Rail, false).tooltip(tooltip("Universe")))
            .child(div().flex_1())
            .child(icon_button(t, "rail-agents", Icon::Users, IconSize::Rail, false).tooltip(tooltip("Agents")))
            .child(icon_button(t, "rail-settings", Icon::Settings, IconSize::Rail, false).tooltip(tooltip("Nixa settings")))
    }

    fn content(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let col1 = column()
            .child(card(t, "Text styles", div().flex().flex_col().gap(px(5.0))
                .child(title(t, "Nixa title"))
                .child(heading(t, "Section heading"))
                .child(strong(t, "Strong label"))
                .child(label(t, "Body label text for rows and menus"))
                .child(muted(t, "Muted note under a setting"))
                .child(caption(t, "Caption · 2 min ago"))
                .child(div().flex().items_center().gap(px(6.0)).child(mono(t, "nixa term list")).child(kbd(t, "⌘K")))))
            .child(card(t, "Buttons", wrap()
                .child(button(t, "b-primary", "Start agent", ButtonKind::Primary))
                .child(button(t, "b-secondary", "Cancel", ButtonKind::Secondary))
                .child(button(t, "b-ghost", "Skip", ButtonKind::Ghost))
                .child(button(t, "b-danger", "Close pane", ButtonKind::Danger))
                .child(icon_label_button(t, "b-icon", Icon::Send, "Send", ButtonKind::Secondary))
                .child(disabled(button(t, "b-off", "Disabled", ButtonKind::Primary), true))))
            .child(card(t, "Icon buttons", wrap()
                .child(icon_button(t, "ib-s", Icon::X, IconSize::Small, false))
                .child(icon_button(t, "ib-t", Icon::Plus, IconSize::Toolbar, false))
                .child(icon_button(t, "ib-ts", Icon::Pin, IconSize::Toolbar, true))
                .child(icon_button(t, "ib-r", Icon::Files, IconSize::Rail, false))
                .child(icon_button(t, "ib-rs", Icon::GitBranch, IconSize::Rail, true))
                .child(icon_button(t, "ib-badge", Icon::Bell, IconSize::Rail, false).child(count_badge(t, 12, t.red)))))
            .child(card(t, "Icons", div().flex().flex_wrap().gap(px(9.0)).children(Icon::ALL.iter().map(|which| {
                div()
                    .id(SharedString::from(format!("icon-{}", which.name())))
                    .child(icon(*which).size(px(16.0)).text_color(t.muted))
                    .tooltip(tooltip(which.name()))
            }))));
        let col2 = column()
            .child(card(t, "Chips and status", div().flex().flex_col().gap(px(8.0))
                .child(wrap().children(Status::ALL.iter().map(|s| status_chip(t, *s))))
                .child(wrap().children(Status::ALL.iter().enumerate().map(|(i, s)| {
                    div().flex().items_center().gap(px(5.0))
                        .child(status_dot(t, ("dot", i), *s))
                        .child(caption(t, s.as_str()).text_color(status_color(t, *s)))
                })))
                .child(wrap()
                    .child(count_chip(2, t.yellow))
                    .child(count_chip(7, t.green))
                    .child(chip("Beta", t.magenta))
                    .child(outline_chip(t, "main"))
                    .child(model_chip(t, "model-chip", "Opus 5.5")))))
            .child(card(t, "Toggles", div().flex().flex_col()
                .children([
                    ("Wake word", "Say Nixa to start talking"),
                    ("Read replies aloud", "Agent replies are spoken"),
                    ("Automatic decisions", "The deep brain decides alone"),
                ].iter().enumerate().map(|(i, (name, note))| {
                    toggle_row(t, ("toggle", i), *name, Some((*note).into()), self.toggles[i])
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggles[i] = !this.toggles[i];
                            cx.notify();
                        }))
                }))))
            .child(card(t, "Tabs and segments", div().flex().flex_col().gap(px(10.0))
                .child(segmented(t).children(["Manual", "Auto", "Ask"].iter().enumerate().map(|(i, name)| {
                    segment(t, ("seg", i), *name, i == self.segment).on_click(cx.listener(move |this, _, _, cx| {
                        this.segment = i;
                        cx.notify();
                    }))
                })))
                .child(wrap()
                    .child(tab(t, "tab-a", "Files", Some(icon(Icon::Files).size(px(13.0)).text_color(t.accent).into_any_element()), true))
                    .child(tab(t, "tab-b", "Git", Some(icon(Icon::GitBranch).size(px(13.0)).text_color(t.muted).into_any_element()), false)))))
            .child(card(t, "Text input", div().flex().flex_col().gap(px(8.0))
                .child(self.search.clone())
                .child(self.input.clone())
                .child(caption(t, self.note.clone()))))
            .child(card(t, "Scroll area", scroll_area(t, "scroll-demo", &self.list_scroll,
                div().flex().flex_col().pr(px(8.0)).children((1..=24).map(|n| {
                    div().h(px(24.0)).flex().items_center().text_size(px(12.0)).text_color(t.muted)
                        .child(format!("Line {n}: m{} printed a result", n + 3))
                }))).h(px(120.0))));
        let this = cx.entity().downgrade();
        let close_preview = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |g, cx| {
                g.note = "Dialog closed".into();
                cx.notify();
            })
            .ok();
        };
        let col3 = column()
            .child(card(t, "Menu", menu(t, "menu-preview", Some("Actions".into()), &context_items(), self.menu_at, Self::picker(cx))))
            .child(card(t, "Modal", div().flex().flex_col().gap(px(10.0))
                .child(dialog(t, "dialog-preview", "Rename space", 300.0,
                    div().flex().flex_col().gap(px(6.0)).child(muted(t, "New name")).child(
                        div().h(px(30.0)).px(px(10.0)).flex().items_center().rounded(px(7.0)).bg(t.input).border_1().border_color(t.accent)
                            .text_size(px(13.0)).child("nixa")),
                    Some(div().flex().gap(px(8.0))
                        .child(button(t, "dlg-cancel", "Cancel", ButtonKind::Ghost))
                        .child(button(t, "dlg-ok", "Save", ButtonKind::Primary)).into_any_element()),
                    close_preview))
                .child(wrap().child(button(t, "open-modal", "Open as overlay", ButtonKind::Secondary)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.modal_open = true;
                        cx.notify();
                    }))))))
            .child(card(t, "Popover and tooltip", div().flex().flex_col().gap(px(10.0))
                .child(popover(t).w(px(230.0)).p(px(10.0)).gap(px(6.0))
                    .child(strong(t, "Senior Developer"))
                    .child(muted(t, "claude · opus 5.5 · nixa"))
                    .child(wrap().child(status_chip(t, Status::Working)).child(outline_chip(t, "m4"))))
                .child(wrap().child(button(t, "tip-button", "Hover for a tooltip", ButtonKind::Secondary)
                    .tooltip(tooltip("Tooltips use the raised color and a soft shadow"))))))
            .child(card(t, "Toast", wrap()
                .children([(ToastKind::Info, "Info"), (ToastKind::Success, "Success"), (ToastKind::Warning, "Warning"), (ToastKind::Error, "Error")]
                    .into_iter().map(|(kind, name)| {
                        button(t, SharedString::from(format!("toast-{name}")), name, ButtonKind::Secondary)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toasts.push(kind, format!("{name} toast"), Some("Shown at the bottom right.".into()));
                                cx.notify();
                            }))
                    }))))
            .child(card(t, "Split handle", div().flex().flex_col().gap(px(6.0))
                .child(muted(t, "Drag the line next to the sidebar."))
                .child(caption(t, format!("Sidebar width {:.0} px (min {SIDEBAR_MIN}, max {SIDEBAR_MAX})", self.sidebar_width)))
                .child(div().h(px(48.0)).flex().rounded(px(8.0)).border_1().border_color(t.border).overflow_hidden()
                    .child(div().w(px(90.0)).bg(t.chrome))
                    .child(split_handle(t, "demo", Axis::Horizontal))
                    .child(div().flex_1().bg(t.background)))));
        div().flex().gap(px(12.0)).p(px(12.0)).child(col1).child(col2).child(col3)
    }
}

fn column() -> gpui::Div {
    div().flex_1().min_w(px(0.0)).flex().flex_col().gap(px(12.0))
}

fn wrap() -> gpui::Div {
    div().flex().flex_wrap().items_center().gap(px(8.0))
}

fn card(t: &Theme, name: &str, body: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .child(group_title(t, name.to_uppercase()))
        .child(body)
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let sidebar = self.sidebar(&t, cx);
        let top = self.top_bar(&t, cx);
        let content = self.content(&t, cx);
        let toasts: Vec<_> = self
            .toasts
            .items()
            .iter()
            .map(|toast| {
                let id = toast.id;
                let this = cx.entity().downgrade();
                toast_card(&t, toast, move |_, cx| {
                    this.update(cx, |g, cx| {
                        g.toasts.dismiss(id);
                        cx.notify();
                    })
                    .ok();
                })
            })
            .collect();
        let picker = Self::picker(cx);
        let this = cx.entity().downgrade();
        let close_modal = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |g, cx| {
                g.modal_open = false;
                cx.notify();
            })
            .ok();
        };
        let close_modal_too = close_modal.clone();
        div()
            .id("gallery-root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .relative()
            .size_full()
            .flex()
            .bg(t.canvas)
            .font_family(text::UI_FONT)
            .text_color(t.foreground)
            .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.context = Some(event.position);
                cx.notify();
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<SplitDrag>, _, cx| {
                if event.drag(cx).id.as_ref() == "sidebar" {
                    this.sidebar_width = size_from_drag(Axis::Horizontal, Edge::Start, event.bounds, event.event.position, SIDEBAR_MIN, SIDEBAR_MAX);
                    cx.notify();
                }
            }))
            .child(sidebar)
            .child(split_handle(&t, "sidebar", Axis::Horizontal))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(top)
                    .child(
                        div().flex_1().min_h(px(0.0)).p(px(PANE_GAP)).flex().child(
                            pane(&t, true)
                                .flex_1()
                                .child(pane_header(&t, "nixa-ui gallery · every part", true))
                                .child(scroll_area(&t, "gallery-scroll", &self.scroll, content).flex_1()),
                        ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .h(px(STATUS_BAR_HEIGHT))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .bg(t.chrome)
                            .border_t_1()
                            .border_color(t.border)
                            .child(caption(&t, "m4 · working"))
                            .child(caption(&t, "engine 127.0.0.1:7421"))
                            .child(div().flex_1())
                            .child(caption(&t, self.note.clone())),
                    ),
            )
            .child(self.rail(&t))
            .child(toast_layer(toast_stack().children(toasts)))
            .when_some(self.context, |el, at| {
                el.child(float_at(
                    at,
                    Corner::TopLeft,
                    menu(&t, "context-menu", Some("Actions".into()), &context_items(), self.menu_at, picker)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.context = None;
                            cx.notify();
                        })),
                ))
            })
            .when(self.modal_open, |el| {
                el.child(overlay(
                    &t,
                    "modal-overlay",
                    dialog(
                        &t,
                        "modal",
                        "Start a custom agent",
                        420.0,
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(label(&t, "Senior Developer will start in nixa with Opus 5.5."))
                            .child(muted(&t, "It keeps its session, so you can resume it later.")),
                        Some(
                            div()
                                .flex()
                                .gap(px(8.0))
                                .child(button(&t, "modal-cancel", "Cancel", ButtonKind::Ghost))
                                .child(button(&t, "modal-start", "Start", ButtonKind::Primary))
                                .into_any_element(),
                        ),
                        close_modal,
                    ),
                    close_modal_too,
                ))
            })
    }
}


fn main() {
    let quit_after = std::env::var("NIXA_GALLERY_QUIT_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok());
    let overlays = std::env::var("NIXA_GALLERY_OVERLAYS").is_ok_and(|v| v == "1");
    Application::new().with_assets(nixa_ui::Assets).run(move |cx: &mut App| {
        nixa_ui::init(cx);
        let bounds = Bounds::new(point(px(60.0), px(28.0)), size(px(1480.0), px(860.0)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Nixa UI gallery".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(TRAFFIC_LIGHTS_X), px(TRAFFIC_LIGHTS_Y))),
            }),
            ..WindowOptions::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| Gallery::new(cx, overlays));
            window.focus(&view.focus_handle(cx));
            view
        });
        if opened.is_err() {
            cx.quit();
            return;
        }
        cx.activate(true);
        if let Some(ms) = quit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
}
