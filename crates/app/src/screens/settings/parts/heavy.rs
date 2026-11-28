use super::outside::{card, note, row, title};
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, SharedString, Window, div, px};
use nixa_protocol::heavy::{self, Job, Settings};
use nixa_ui::button::{ButtonKind, button};
use nixa_ui::toggle::toggle_row;
use nixa_ui::{Theme, theme};
use std::path::PathBuf;

pub const PART: &str = "Heavy checks";
pub const ABOUT: &str = "Builds and tests in Nixa terminals take turns, so the Mac does not slow down. Other commands run at once.";
pub const SLOT_CHOICES: [usize; 6] = [1, 2, 3, 4, 6, 8];

pub struct HeavyView {
    pub data: PathBuf,
    pub settings: Settings,
    pub busy: Vec<Job>,
    pub waiting: Vec<Job>,
    pub ready: bool,
    pub error: Option<String>,
    pub note: Option<String>,
}

pub fn build(data: PathBuf) -> HeavyView {
    let mut v = HeavyView { data, settings: Settings::default(), busy: Vec::new(), waiting: Vec::new(), ready: false, error: None, note: None };
    reload(&mut v);
    v
}

pub fn reload(v: &mut HeavyView) {
    v.settings = heavy::load(&v.data);
    v.busy = heavy::busy(&v.data);
    v.waiting = heavy::waiting(&v.data);
    v.ready = heavy::ready(&v.data);
}

pub fn save(v: &mut HeavyView, settings: Settings, cx: &mut Context<HeavyView>) {
    match heavy::save(&v.data, &settings) {
        Ok(_) => {
            v.error = None;
            v.note = Some("Saved.".into());
        }
        Err(e) => {
            v.error = Some(e);
            v.note = None;
        }
    }
    reload(v);
    cx.notify();
}

pub struct Shared(pub Entity<HeavyView>);

impl gpui::Global for Shared {}

pub fn shared(cx: &App) -> Option<Entity<HeavyView>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}

pub fn show(cx: &mut App) -> Entity<HeavyView> {
    if let Some(view) = shared(cx) {
        view.update(cx, |v, cx| {
            reload(v);
            cx.notify();
        });
        return view;
    }
    let view = cx.new(|_| build(nixa_protocol::paths::data_dir()));
    cx.set_global(Shared(view.clone()));
    view
}

pub fn job_line(job: &Job, now: u64) -> String {
    let secs = now.saturating_sub(job.since) / 1000;
    let place = if job.terminal.is_empty() { String::new() } else { format!(" in {}", job.terminal) };
    format!("{}{place}, {secs} s", job.command)
}

pub fn status_line(v: &HeavyView) -> String {
    let helpers = if v.ready { "Helpers are ready." } else { "Helpers are not written yet. They are written when Nixa starts." };
    format!("{} running, {} waiting. {helpers}", v.busy.len(), v.waiting.len())
}

pub fn body(v: &HeavyView, t: &Theme, cx: &mut Context<HeavyView>) -> AnyElement {
    let s = v.settings.clone();
    let on = s.on;
    let mut slots = row();
    for n in SLOT_CHOICES {
        let tag = format!("heavy-slots-{n}");
        let selector = tag.clone();
        let next = Settings { slots: n, ..s.clone() };
        slots = slots.child(
            button(t, SharedString::from(tag), n.to_string(), if s.slots == n { ButtonKind::Primary } else { ButtonKind::Secondary })
                .debug_selector(move || selector.clone())
                .on_click(cx.listener(move |v, _, _, cx| save(v, next.clone(), cx))),
        );
    }
    let flip = Settings { on: !on, ..s.clone() };
    let main = card(t)
        .child(
            toggle_row(t, "heavy-on", "Take turns for builds and tests", Some(ABOUT.into()), on)
                .debug_selector(|| "heavy-on".into())
                .on_click(cx.listener(move |v, _, _, cx| save(v, flip.clone(), cx))),
        )
        .child(note(t.muted, 11., "Heavy runs at the same time"))
        .child(slots)
        .child(note(t.dim, 11.5, format!("Tools: {}.", heavy::TOOLS.join(", "))));
    let now = heavy::now_ms();
    let mut jobs = card(t).child(
        row()
            .justify_between()
            .child(title(t, "Now"))
            .child(
                button(t, "heavy-refresh", "Refresh", ButtonKind::Secondary)
                    .debug_selector(|| "heavy-refresh".into())
                    .on_click(cx.listener(|v, _, _, cx| {
                        reload(v);
                        cx.notify();
                    })),
            ),
    );
    jobs = jobs.child(note(t.dim, 11.5, status_line(v)));
    for job in &v.busy {
        jobs = jobs.child(note(t.foreground, 11.5, format!("Slot {}: {}", job.slot, job_line(job, now))));
    }
    for (n, job) in v.waiting.iter().enumerate() {
        jobs = jobs.child(note(t.muted, 11.5, format!("Waiting {}: {}", n + 1, job_line(job, now))));
    }
    div()
        .id("heavy-part")
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(main)
        .child(jobs)
        .when_some(v.note.clone(), |el, n| el.child(note(t.green, 12., n)))
        .when_some(v.error.clone(), |el, e| el.child(note(t.yellow, 12., e)))
        .into_any_element()
}

impl Render for HeavyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div().id("settings-heavy-scroll").size_full().min_h_0().overflow_y_scroll().child(body(self, &t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};

    #[gpui::test]
    fn the_part_turns_slots_off_and_changes_the_count(cx: &mut TestAppContext) {
        cx.update(nixa_ui::init);
        let data = std::env::temp_dir().join(format!("nixa-heavy-part-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let (view, vcx) = cx.add_window_view(|_, _| build(data.clone()));
        vcx.run_until_parked();
        for tag in ["heavy-on", "heavy-slots-4", "heavy-refresh"] {
            let at = vcx.debug_bounds(tag).unwrap_or_else(|| panic!("{tag} is not drawn"));
            vcx.simulate_click(at.center(), Modifiers::none());
            vcx.run_until_parked();
        }
        assert_eq!(heavy::load(&data), Settings { on: false, slots: 4 });
        view.read_with(vcx, |v, _| {
            assert_eq!(v.settings, Settings { on: false, slots: 4 });
            assert!(status_line(v).starts_with("0 running, 0 waiting."));
        });
        std::fs::remove_dir_all(&data).ok();
    }
}
