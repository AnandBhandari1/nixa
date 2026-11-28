use super::ai::voice::{self, Report};
use super::changes::words::{Level as NoteLevel, Note};
use crate::links::brain::Link;
use crate::screens::events::{self, Notice, Screens};
use crate::workspace::Workspace;
use gpui::App;

pub fn level_of(level: NoteLevel) -> events::Level {
    match level {
        NoteLevel::Info => events::Level::Info,
        NoteLevel::Success => events::Level::Success,
        NoteLevel::Warning => events::Level::Warning,
        NoteLevel::Error => events::Level::Error,
    }
}

pub fn voice_level(level: events::Level) -> voice::Level {
    match level {
        events::Level::Info => voice::Level::Info,
        events::Level::Success => voice::Level::Success,
        events::Level::Warning => voice::Level::Warning,
        events::Level::Error => voice::Level::Error,
    }
}

pub fn notice_of(n: &Note) -> Notice {
    Notice::new(level_of(n.level), n.title.clone()).body(n.body.clone())
}

pub fn report_of(n: &Note) -> Report {
    voice::report(n.title.clone(), n.body.clone(), voice_level(level_of(n.level))).spoken(n.spoken.clone())
}

pub fn spoken_result(title: &str, body: &str, level: events::Level) -> Option<Report> {
    (level != events::Level::Info).then(|| voice::report(title, body, voice_level(level)))
}

pub fn say_later(link: Option<Link>, report: Report, cx: &mut App) {
    let Some(link) = link else { return };
    if report.spoken.trim().is_empty() {
        return;
    }
    let data = nixa_protocol::paths::data_dir();
    cx.background_executor()
        .spawn(async move {
            let _ = voice::say(&link, &data, &report);
        })
        .detach();
}

pub fn show_note(screens: &Screens, link: Option<Link>, n: &Note, cx: &mut App) {
    events::notify(screens, notice_of(n), cx);
    say_later(link, report_of(n), cx);
}

pub fn post(title: &str, body: &str, level: events::Level, cx: &mut App) {
    let notice = Notice::new(level, title).body(body);
    let report = spoken_result(title, body, level);
    let Some(ws) = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()) else { return };
    cx.defer(move |cx| {
        ws.update(cx, |ws, _, cx| {
            events::notify(&ws.hub.events, notice, cx);
            if let Some(report) = report {
                say_later(ws.hub.link.clone(), report, cx);
            }
        })
        .ok();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::git::changes::words::note;

    #[test]
    fn a_panel_note_becomes_a_toast_and_its_spoken_words() {
        let n = note("Pushed", "main is now on origin.", NoteLevel::Success, "Pushed to origin");
        let shown = notice_of(&n);
        assert_eq!(shown.title, "Pushed");
        assert_eq!(shown.body, "main is now on origin.");
        assert_eq!(shown.level, events::Level::Success);
        let said = report_of(&n);
        assert_eq!(said.spoken, "Pushed to origin");
        assert_eq!(said.level, voice::Level::Success);
        assert_eq!(said.title, "Pushed");
    }

    #[test]
    fn levels_keep_their_weight() {
        assert_eq!(level_of(NoteLevel::Info), events::Level::Info);
        assert_eq!(level_of(NoteLevel::Warning), events::Level::Warning);
        assert_eq!(level_of(NoteLevel::Error), events::Level::Error);
        assert_eq!(voice_level(events::Level::Error), voice::Level::Error);
        assert_eq!(voice_level(events::Level::Info), voice::Level::Info);
    }

    #[test]
    fn window_results_speak_only_when_they_matter() {
        assert_eq!(spoken_result("Commit id copied", "abc", events::Level::Info), None);
        let r = spoken_result("Branch deleted", "", events::Level::Success).unwrap();
        assert_eq!(r.spoken, "Branch deleted");
        assert_eq!(r.level, voice::Level::Success);
        assert!(spoken_result("Stash failed", "why", events::Level::Warning).is_some());
    }
}
