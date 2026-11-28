use super::{
    Choice, DAYS, Item, PickState, REPEATS, Schedules, ask, ask_work, clip, extras, folders,
    is_member, is_pr_team, is_team, job_line, open_form, pick_agent, pick_folder, request_picks,
    save_form, send, summary,
};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, div, px,
};
use nixa_ui::popover::tooltip;
use nixa_ui::{Icon, Theme, icon, theme};
use serde_json::json;

pub struct Look {
    pub on: bool,
    pub enabled: bool,
    pub color: Option<Hsla>,
    pub icon: Option<Icon>,
}

pub fn plain() -> Look {
    Look {
        on: false,
        enabled: true,
        color: None,
        icon: None,
    }
}

pub fn chip(
    t: &Theme,
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    tip: impl Into<SharedString>,
    look: Look,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let selector = id.to_string();
    let label: SharedString = label.into();
    let color = if !look.enabled {
        t.dim
    } else {
        look.color
            .unwrap_or(if look.on { t.green } else { t.muted })
    };
    let hover = t.hover;
    div()
        .id(ElementId::Name(id))
        .debug_selector(move || selector)
        .min_h(px(25.))
        .px(px(8.))
        .py(px(4.))
        .gap(px(5.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(if look.on {
            color.opacity(0.65)
        } else {
            t.border
        })
        .when(look.on, |el| el.bg(color.opacity(0.08)))
        .text_size(px(11.5))
        .text_color(color)
        .tooltip(tooltip(tip.into()))
        .when(look.enabled, |el| {
            el.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .when(!look.enabled, |el| el.opacity(0.6))
        .when_some(look.icon, |el, which| {
            el.child(icon(which).size(px(12.)).text_color(color))
        })
        .when(!label.is_empty(), |el| el.child(label))
}

fn caption(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .text_color(t.muted)
        .child(text.into())
}

fn line(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    div()
        .text_size(px(size))
        .text_color(color)
        .whitespace_normal()
        .child(text.into())
}

fn card(t: &Theme, border: Hsla) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(11.))
        .rounded(px(8.))
        .border_1()
        .border_color(border)
        .bg(t.chrome)
}

fn wrap() -> Div {
    div().w_full().flex().flex_wrap().gap(px(5.))
}

fn field(t: &Theme, label: &str, input: AnyElement) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(caption(t, label.to_string()))
        .child(input)
}

fn safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub struct Lists<'a> {
    pub teams: &'a [Choice],
    pub team: &'a [Choice],
    pub agents: &'a [Choice],
}

fn choices(
    t: &Theme,
    prefix: &'static str,
    chosen: Option<&str>,
    lists: Lists,
    follow: bool,
    cx: &mut Context<Schedules>,
) -> Div {
    let Lists {
        teams,
        team,
        agents,
    } = lists;
    let mut out = div().w_full().flex().flex_col().gap(px(5.));
    let group = |title: &str, list: &[Choice], out: Div| -> Div {
        if list.is_empty() {
            return out;
        }
        let row = list.iter().map(|c| {
            let id = c.id.clone();
            let tip = if follow {
                format!("{} works on the tasks", c.name)
            } else {
                format!("Use {}", c.name)
            };
            chip(
                t,
                format!("{prefix}-{}", safe(&c.id)),
                c.name.clone(),
                tip,
                Look {
                    on: chosen == Some(c.id.as_str()),
                    ..plain()
                },
            )
            .on_click(cx.listener(move |part, _, _, cx| {
                if follow {
                    if let Some(f) = part.form.as_mut() {
                        f.follow_agent = Some(id.clone());
                    }
                    cx.notify();
                } else {
                    pick_agent(part, &id, cx);
                }
            }))
            .into_any_element()
        });
        out.child(line(title.to_string(), t.blue, 11.))
            .child(wrap().children(row))
    };
    if !follow {
        out = group("Teams (members work one after another)", teams, out);
    }
    out = group("Team", team, out);
    group("Agents", agents, out)
}

const PR_LOAD: &str = "Load the repos";

fn job_rows(
    part: &Schedules,
    t: &Theme,
    folder: &str,
    pr: bool,
    cx: &mut Context<Schedules>,
) -> Div {
    let Some(form) = part.form.as_ref() else {
        return div();
    };
    let ready = part.picks.as_ref().filter(|p| p.folder == folder);
    let mut out = div().w_full().flex().flex_col().gap(px(6.));
    let Some(picks) = ready else {
        let wanted = folder.to_string();
        let label = if pr { PR_LOAD } else { "Load the repos" };
        let note = if folder.is_empty() {
            "Pick a folder first."
        } else {
            "The folder changed. Load the list again."
        };
        return out.child(
            div()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(
                    chip(
                        t,
                        "schedule-picks-load",
                        label,
                        "Ask Nixa for the repos in this folder",
                        Look {
                            enabled: !folder.is_empty(),
                            icon: Some(Icon::Check),
                            ..plain()
                        },
                    )
                    .on_click(cx.listener(move |part, _, _, cx| request_picks(part, &wanted, cx))),
                )
                .child(line(note, t.dim, 11.)),
        );
    };
    match &picks.state {
        PickState::Loading => return out.child(line("Loading the repos…", t.dim, 11.)),
        PickState::Failed(e) => {
            return out.child(line(format!("Cannot load the repos: {e}"), t.red, 11.));
        }
        PickState::Ready => {}
    }
    out = out.child(caption(
        t,
        if picks.repos.is_empty() {
            "Repo: no git repo in this folder"
        } else {
            "Repo"
        },
    ));
    out = out.child(wrap().children(picks.repos.iter().map(|r| {
        let (folder, current) = (r.folder.clone(), r.current.clone());
        chip(
            t,
            format!("schedule-repo-{}", safe(&r.folder)),
            r.name.clone(),
            r.folder.clone(),
            Look {
                on: form.job.repo == r.folder,
                ..plain()
            },
        )
        .on_click(cx.listener(move |part, _, _, cx| {
            if let Some(f) = part.form.as_mut() {
                f.job.repo = folder.clone();
                f.job.branch = current.clone();
            }
            cx.notify();
        }))
        .into_any_element()
    })));
    if let Some(repo) = picks.repos.iter().find(|r| r.folder == form.job.repo) {
        out = out
            .child(caption(t, "Branch"))
            .child(wrap().children(repo.branches.iter().map(|b| {
                let name = b.clone();
                let tip = if *b == repo.current {
                    format!("{b} (current branch)")
                } else {
                    b.clone()
                };
                chip(
                    t,
                    format!("schedule-branch-{}", safe(b)),
                    b.clone(),
                    tip,
                    Look {
                        on: form.job.branch == *b,
                        ..plain()
                    },
                )
                .on_click(cx.listener(move |part, _, _, cx| {
                    if let Some(f) = part.form.as_mut() {
                        f.job.branch = name.clone();
                    }
                    cx.notify();
                }))
                .into_any_element()
            })));
        if !form.job.branch.is_empty()
            && !repo.current.is_empty()
            && form.job.branch != repo.current
        {
            out = out.child(line(
                format!(
                    "{} is on {} now. The PR team works only on the current branch.",
                    repo.name, repo.current
                ),
                t.yellow,
                11.,
            ));
        }
    }
    out
}

const PR_PROMPT: &str = "Only this PR number (optional). Empty: every open PR in this repo, oldest first.";

fn form_box(part: &Schedules, t: &Theme, cx: &mut Context<Schedules>) -> AnyElement {
    let Some(form) = part.form.clone() else {
        return div().into_any_element();
    };
    let snap = part.snap.clone().unwrap_or_default();
    let busy = part.pending.is_some();
    let folder = part.inputs.folder.read(cx).value().trim().to_string();
    let extra = extras(&snap, &form);
    let mut agents = snap.agents.clone();
    agents.extend(
        extra
            .iter()
            .filter(|c| !is_member(&c.id) && !is_team(&c.id))
            .cloned(),
    );
    let mut team = snap.members.clone();
    team.extend(extra.iter().filter(|c| is_member(&c.id)).cloned());
    let mut teams = snap.teams.clone();
    teams.extend(extra.iter().filter(|c| is_team(&c.id)).cloned());
    let agent = form.agent.clone().unwrap_or_default();
    let runs_team = is_team(&agent);
    let pr = is_pr_team(Some(&snap), &agent);
    let prompt_title = if pr {
        PR_PROMPT
    } else if runs_team {
        "Task for the team"
    } else {
        "Instructions (the agent's first message)"
    };
    let space_chips = folders(&snap)
        .into_iter()
        .map(|space| {
            let label = space
                .folder
                .rsplit('/')
                .find(|p| !p.is_empty())
                .unwrap_or(&space.folder)
                .to_string();
            let on = folder == space.folder;
            let tip = space.folder.clone();
            chip(
                t,
                format!("schedule-folder-{}", safe(&space.folder)),
                label,
                tip,
                Look { on, ..plain() },
            )
            .on_click(cx.listener(move |part, _, _, cx| pick_folder(part, &space, cx)))
            .into_any_element()
        })
        .collect::<Vec<_>>();
    let repeat_chips = REPEATS
        .iter()
        .map(|(repeat, label)| {
            let r = *repeat;
            chip(
                t,
                format!("schedule-repeat-{}", r.as_str()),
                *label,
                *label,
                Look {
                    on: form.repeat == r,
                    ..plain()
                },
            )
            .on_click(cx.listener(move |part, _, _, cx| {
                if let Some(f) = part.form.as_mut() {
                    f.repeat = r;
                }
                cx.notify();
            }))
            .into_any_element()
        })
        .collect::<Vec<_>>();
    let day_chips = DAYS
        .iter()
        .enumerate()
        .map(|(i, day)| {
            let n = i as i64;
            chip(
                t,
                format!("schedule-day-{i}"),
                *day,
                format!("Run on {day}"),
                Look {
                    on: form.days.contains(&n),
                    ..plain()
                },
            )
            .on_click(cx.listener(move |part, _, _, cx| {
                if let Some(f) = part.form.as_mut() {
                    if f.days.contains(&n) {
                        f.days.retain(|d| *d != n);
                    } else {
                        f.days.push(n);
                    }
                }
                cx.notify();
            }))
            .into_any_element()
        })
        .collect::<Vec<_>>();
    let time_label = if form.repeat == super::Repeat::Hourly {
        "Minute (HH:mm, the hour is not used)"
    } else {
        "Time (24-hour)"
    };
    let mut body = card(t, t.blue.opacity(0.45))
        .id("schedule-form")
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.bright)
                .child(if form.id.is_some() {
                    "Edit schedule"
                } else {
                    "New schedule"
                }),
        )
        .child(field(
            t,
            "Name",
            part.inputs.name.clone().into_any_element(),
        ))
        .child(caption(t, "Agent"))
        .child(choices(
            t,
            "schedule-agent",
            form.agent.as_deref(),
            Lists {
                teams: &teams,
                team: &team,
                agents: &agents,
            },
            false,
            cx,
        ))
        .child(field(
            t,
            if runs_team {
                "Folder (the space)"
            } else {
                "Folder"
            },
            part.inputs.folder.clone().into_any_element(),
        ))
        .child(wrap().children(space_chips));
    if runs_team {
        body = body.child(job_rows(part, t, &folder, pr, cx));
    }
    body = body
        .child(field(
            t,
            prompt_title,
            part.inputs.prompt.clone().into_any_element(),
        ))
        .child(caption(t, "Repeat"))
        .child(wrap().children(repeat_chips))
        .child(
            div()
                .w_full()
                .flex()
                .gap(px(10.))
                .child(div().flex_1().child(field(
                    t,
                    time_label,
                    part.inputs.time.clone().into_any_element(),
                )))
                .when(form.repeat == super::Repeat::Once, |el| {
                    el.child(div().flex_1().child(field(
                        t,
                        "Date",
                        part.inputs.date.clone().into_any_element(),
                    )))
                }),
        )
        .when(form.repeat == super::Repeat::Weekly, |el| {
            el.child(wrap().children(day_chips))
        });
    if runs_team {
        let text = if pr {
            "The PR team commits the old work first, then each PR's picked parts, locally, one commit per PR. Nothing is pushed. Nixa posts the comments itself."
        } else {
            "A team run has no follow-up agent: its own steps check and fix the work."
        };
        body = body.child(line(text, t.dim, 11.5));
    } else {
        body = body
            .child(caption(
                t,
                "Follow-up agent: starts by itself on the report's open tasks when a run is done",
            ))
            .child(
                wrap().child(
                    chip(
                        t,
                        "schedule-follow-none",
                        "None",
                        "No agent starts by itself. Use Work on tasks when you want.",
                        Look {
                            on: form.follow_agent.is_none(),
                            ..plain()
                        },
                    )
                    .on_click(cx.listener(|part, _, _, cx| {
                        if let Some(f) = part.form.as_mut() {
                            f.follow_agent = None;
                        }
                        cx.notify();
                    })),
                ),
            )
            .child(choices(
                t,
                "schedule-follow",
                form.follow_agent.as_deref(),
                Lists {
                    teams: &[],
                    team: &team,
                    agents: &agents,
                },
                true,
                cx,
            ));
        if form.follow_agent.is_some() {
            body = body.child(part.inputs.follow_prompt.clone());
        }
    }
    let skip_label = if form.skip_unchanged {
        "Skips if nothing changed"
    } else {
        "Runs every time"
    };
    body = body
        .child(
            div()
                .flex()
                .gap(px(5.))
                .child(
                    chip(t, "schedule-enabled", if form.enabled { "On" } else { "Paused" }, "Turn this schedule on or pause it", Look { on: form.enabled, icon: Some(if form.enabled { Icon::Check } else { Icon::Square }), ..plain() })
                        .on_click(cx.listener(|part, _, _, cx| {
                            if let Some(f) = part.form.as_mut() {
                                f.enabled = !f.enabled;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    chip(t, "schedule-skip-unchanged", skip_label, "Skip a timed run when the folder's git state is the same as after the last run. Run now always runs.", Look { on: form.skip_unchanged, icon: Some(if form.skip_unchanged { Icon::Check } else { Icon::Square }), ..plain() })
                        .on_click(cx.listener(|part, _, _, cx| {
                            if let Some(f) = part.form.as_mut() {
                                f.skip_unchanged = !f.skip_unchanged;
                            }
                            cx.notify();
                        })),
                ),
        )
        .when_some(form.error.clone(), |el, e| el.child(line(e, t.red, 12.).id("schedule-form-error").debug_selector(|| "schedule-form-error".into())))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(
                    chip(t, "schedule-cancel", "Cancel", "Close without saving", Look { icon: Some(Icon::X), ..plain() }).on_click(cx.listener(|part, _, _, cx| {
                        part.form = None;
                        cx.notify();
                    })),
                )
                .child(
                    chip(t, "schedule-save", if busy { "Saving…" } else { "Save" }, "Save this schedule", Look { enabled: !busy, color: Some(t.green), icon: Some(Icon::Check), ..plain() })
                        .when(!busy, |el| el.on_click(cx.listener(|part, _, _, cx| save_form(part, cx)))),
                ),
        );
    body.into_any_element()
}

fn row(part: &Schedules, t: &Theme, item: &Item, cx: &mut Context<Schedules>) -> AnyElement {
    let s = &item.schedule;
    let busy = part.pending.is_some();
    let id = safe(&s.id);
    let badge = if s.enabled { t.green } else { t.dim };
    let mut body = card(t, t.border)
        .child(
            div()
                .w_full()
                .flex()
                .gap(px(6.))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.bright)
                        .child(s.name.clone()),
                )
                .child(
                    div()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .bg(badge.opacity(0.1))
                        .text_size(px(10.5))
                        .text_color(badge)
                        .child(if s.enabled { "On" } else { "Paused" }),
                ),
        )
        .child(line(summary(item), t.muted, 11.5));
    let job = job_line(s);
    if !job.is_empty() {
        body = body.child(line(job, t.muted, 11.));
    }
    if !s.prompt.is_empty() {
        body = body.child(line(clip(&s.prompt), t.cyan, 12.));
    }
    body = body
        .child(line(format!("Next: {}", item.view.next), t.dim, 11.))
        .child(line(
            item.view.last.clone(),
            if item.view.failed { t.yellow } else { t.dim },
            11.,
        ));
    let run_item = item.clone();
    let del_item = item.clone();
    let edit_item = s.clone();
    let work_item = item.clone();
    let toggle_id = s.id.clone();
    let skip_id = s.id.clone();
    let on = s.enabled;
    let actions = wrap()
        .child(
            chip(
                t,
                format!("schedule-run-{id}"),
                "Run now",
                "Open the agent tab with these instructions now",
                Look {
                    enabled: !busy,
                    color: Some(t.green),
                    icon: Some(Icon::Play),
                    ..plain()
                },
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(move |part, _, _, cx| {
                    let title = format!("Run {} now?", run_item.schedule.name);
                    ask(part, &run_item, "schedule.run", title, "Run now", false);
                    cx.notify();
                }))
            }),
        )
        .child(
            chip(
                t,
                format!("schedule-toggle-{id}"),
                if on { "Pause" } else { "Turn on" },
                if on {
                    "Stop timed runs"
                } else {
                    "Start timed runs"
                },
                Look {
                    enabled: !busy,
                    icon: Some(if on { Icon::X } else { Icon::Check }),
                    ..plain()
                },
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(move |part, _, _, cx| {
                    send(
                        part,
                        "schedule.toggle",
                        json!({"id": toggle_id, "enabled": !on}),
                        if on { "Pause" } else { "Turn on" },
                        cx,
                    );
                }))
            }),
        )
        .when(item.view.report_done, |el| {
            el.child(
                chip(
                    t,
                    format!("schedule-work-{id}"),
                    "Work on tasks",
                    "Start an agent on the open tasks of the last report",
                    Look {
                        enabled: !busy,
                        icon: Some(Icon::Terminal),
                        ..plain()
                    },
                )
                .when(!busy, |el| {
                    el.on_click(cx.listener(move |part, _, _, cx| {
                        ask_work(part, &work_item);
                        cx.notify();
                    }))
                }),
            )
        })
        .when(s.next_run.is_some(), |el| {
            el.child(
                chip(
                    t,
                    format!("schedule-skip-{id}"),
                    "Skip next",
                    "Skip only the next run",
                    Look {
                        enabled: !busy,
                        icon: Some(Icon::X),
                        ..plain()
                    },
                )
                .when(!busy, |el| {
                    el.on_click(cx.listener(move |part, _, _, cx| {
                        send(
                            part,
                            "schedule.skip",
                            json!({"id": skip_id}),
                            "Skip next",
                            cx,
                        )
                    }))
                }),
            )
        })
        .child(
            chip(
                t,
                format!("schedule-edit-{id}"),
                "Edit",
                "Edit this schedule",
                Look {
                    enabled: !busy,
                    icon: Some(Icon::SquarePen),
                    ..plain()
                },
            )
            .when(!busy, |el| {
                el.on_click(
                    cx.listener(move |part, _, _, cx| open_form(part, Some(&edit_item), cx)),
                )
            }),
        )
        .child(
            chip(
                t,
                format!("schedule-delete-{id}"),
                "Delete",
                "Delete this schedule",
                Look {
                    enabled: !busy,
                    color: Some(t.red),
                    icon: Some(Icon::Trash),
                    ..plain()
                },
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(move |part, _, _, cx| {
                    let title = format!("Delete {}?", del_item.schedule.name);
                    ask(part, &del_item, "schedule.delete", title, "Delete", true);
                    cx.notify();
                }))
            }),
        );
    body.child(actions).into_any_element()
}

fn confirm_box(part: &Schedules, t: &Theme, cx: &mut Context<Schedules>) -> AnyElement {
    let Some(value) = part.confirm.clone() else {
        return div().into_any_element();
    };
    let color = if value.danger { t.red } else { t.yellow };
    let busy = part.pending.is_some();
    let label = if busy {
        "Sending…".to_string()
    } else {
        value.label.clone()
    };
    card(t, color.opacity(0.55))
        .id("schedule-confirm")
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(value.title.clone()),
        )
        .child(line(value.detail.clone(), t.foreground, 12.))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.))
                .child(
                    chip(
                        t,
                        "schedule-confirm-no",
                        "Cancel",
                        "Do nothing",
                        Look {
                            icon: Some(Icon::X),
                            ..plain()
                        },
                    )
                    .on_click(cx.listener(|part, _, _, cx| {
                        part.confirm = None;
                        cx.notify();
                    })),
                )
                .child(
                    chip(
                        t,
                        "schedule-confirm-yes",
                        label,
                        value.title.clone(),
                        Look {
                            enabled: !busy,
                            color: Some(if value.danger { t.red } else { t.green }),
                            icon: Some(Icon::Check),
                            ..plain()
                        },
                    )
                    .when(!busy, |el| {
                        el.on_click(cx.listener(move |part, _, _, cx| {
                            send(part, &value.action, value.payload.clone(), &value.label, cx)
                        }))
                    }),
                ),
        )
        .into_any_element()
}

fn empty_state(t: &Theme, text: String) -> AnyElement {
    div()
        .id("schedules")
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(20.))
        .child(line(text, t.dim, 12.))
        .into_any_element()
}

pub fn render(part: &mut Schedules, cx: &mut Context<Schedules>) -> AnyElement {
    let t = theme(cx).clone();
    let Some(snap) = part.snap.clone() else {
        let text = match &part.load_error {
            Some(e) => format!("Nixa is not connected ({e}). It reconnects by itself."),
            None => "Loading schedules…".into(),
        };
        return empty_state(&t, text);
    };
    let busy = part.pending.is_some();
    let all = snap.enabled;
    let header = div()
        .w_full()
        .flex_none()
        .px(px(4.))
        .py(px(10.))
        .gap(px(8.))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(t.border)
        .child(icon(Icon::Clock).size(px(15.)).text_color(t.blue))
        .child(div().flex_1().min_w_0().child(line(
            "Each schedule opens an agent tab in its project with your instructions.",
            t.dim,
            11.5,
        )))
        .child(
            chip(
                &t,
                "scheduler-toggle",
                if all { "All on" } else { "All paused" },
                if all {
                    "Pause every schedule"
                } else {
                    "Turn timed runs back on"
                },
                Look {
                    on: all,
                    enabled: !busy,
                    icon: Some(if all { Icon::Check } else { Icon::Square }),
                    ..plain()
                },
            )
            .when(!busy, |el| {
                el.on_click(cx.listener(move |part, _, _, cx| {
                    send(
                        part,
                        "scheduler.set",
                        json!({"enabled": !all}),
                        if all { "Pause all" } else { "Turn all on" },
                        cx,
                    )
                }))
            }),
        )
        .child(
            chip(
                &t,
                "schedule-new",
                "New schedule",
                "Add a schedule",
                Look {
                    color: Some(t.green),
                    icon: Some(Icon::Plus),
                    ..plain()
                },
            )
            .on_click(cx.listener(|part, _, _, cx| open_form(part, None, cx))),
        );
    let mut list = div()
        .id("schedules-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.));
    if let Some(error) = &snap.error {
        list = list.child(card(&t, t.red.opacity(0.4)).child(line(error.clone(), t.red, 12.)));
    }
    if let Some(notice) = part.notice.clone() {
        let color = if notice.ok { t.green } else { t.red };
        list = list.child(
            card(&t, color.opacity(0.4))
                .id("schedule-notice")
                .debug_selector(|| "schedule-notice".into())
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .items_center()
                        .child(div().flex_1().child(line(notice.text, color, 12.)))
                        .child(
                            chip(
                                &t,
                                "schedule-notice-close",
                                "",
                                "Dismiss",
                                Look {
                                    icon: Some(Icon::X),
                                    ..plain()
                                },
                            )
                            .on_click(cx.listener(|part, _, _, cx| {
                                part.notice = None;
                                cx.notify();
                            })),
                        ),
                ),
        );
    }
    if part.confirm.is_some() {
        list = list.child(confirm_box(part, &t, cx));
    }
    if part.form.is_some() {
        list = list.child(form_box(part, &t, cx));
    }
    for item in &snap.schedules {
        list = list.child(row(part, &t, item, cx));
    }
    if snap.schedules.is_empty() && part.form.is_none() {
        list = list.child(
            card(&t, t.border)
                .id("schedules-empty")
                .debug_selector(|| "schedules-empty".into())
                .child(line("No schedules yet. Click New schedule.", t.dim, 12.)),
        );
    }
    if !part.events.is_empty() {
        let mut box_ = card(&t, t.border).child(caption(&t, "Latest from the schedules"));
        for text in &part.events {
            box_ = box_.child(line(text.clone(), t.muted, 11.));
        }
        list = list.child(box_);
    }
    div()
        .id("schedules")
        .track_focus(&part.focus)
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .text_size(px(12.8))
        .child(header)
        .child(list)
        .into_any_element()
}
