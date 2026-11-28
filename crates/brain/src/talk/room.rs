use crate::app::App;
use crate::voice::Listen;

pub const HEAD: &str = "Heard (no name)";

pub fn is_room(typed: bool, always: bool, called: bool, open: bool) -> bool {
    !typed && always && !called && !open
}

pub fn heard_in_room(app: &App, text: &str, typed: bool, called: bool, started_at: u64, spoke_at: u64, asked_at: u64) -> bool {
    if typed || called {
        return false;
    }
    let always = crate::voice::settings(app).listen == Listen::Always;
    let card = super::flow::prompts_open(app) > 0 && super::talk(app).is_card_reply(text);
    let open = card || nixa_voice::wake_open(started_at, spoke_at, asked_at, 0);
    is_room(typed, always, called, open)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unnamed_speech_in_always_mode_is_room_talk() {
        assert!(is_room(false, true, false, false));
        assert!(!is_room(true, true, false, false));
        assert!(!is_room(false, false, false, false));
        assert!(!is_room(false, true, true, false));
        assert!(!is_room(false, true, false, true));
    }
}
