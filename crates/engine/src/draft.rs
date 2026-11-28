use std::time::{Duration, Instant};

pub const STALE_MS: u64 = 10_000;
const SHOWN_CHARS: usize = 8;
const SHORT_CHARS: usize = 4;

fn ends_line(line: &str, needle: &str) -> bool {
    let flat: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    flat.trim_end_matches(|c: char| ('\u{2500}'..='\u{257f}').contains(&c)).ends_with(needle)
}

#[derive(Default)]
enum Esc {
    #[default]
    None,
    Start,
    Ss3,
    Csi(String),
}

#[derive(Default)]
pub struct Draft {
    text: String,
    esc: Esc,
    paste: bool,
    pasted: bool,
    typed_at: Option<Instant>,
}

impl Draft {
    pub fn active(&self) -> bool {
        !self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.paste = false;
        self.pasted = false;
        self.esc = Esc::None;
        self.typed_at = None;
    }

    pub fn stale_at(&self) -> Option<Instant> {
        if !self.active() || self.pasted {
            return None;
        }
        self.typed_at.map(|at| at + Duration::from_millis(STALE_MS))
    }

    pub fn stale(&self, tail: &str, now: Instant) -> bool {
        self.stale_at().is_some_and(|at| now >= at) && !self.shown_in(tail)
    }

    fn shown_in(&self, tail: &str) -> bool {
        let chars: Vec<char> = self.text.chars().filter(|c| !c.is_whitespace()).collect();
        if chars.is_empty() {
            return false;
        }
        let needle: String = chars[chars.len().saturating_sub(SHOWN_CHARS)..].iter().collect();
        if tail.lines().any(|line| ends_line(line, &needle)) {
            return true;
        }
        let hay: String = tail.chars().filter(|c| !c.is_whitespace()).collect();
        needle.chars().count() >= SHORT_CHARS && hay.contains(&needle)
    }

    pub fn feed(&mut self, input: &str) {
        self.typed_at = Some(Instant::now());
        for c in input.chars() {
            match std::mem::take(&mut self.esc) {
                Esc::None => self.plain(c),
                Esc::Start => match c {
                    '[' => self.esc = Esc::Csi(String::new()),
                    'O' => self.esc = Esc::Ss3,
                    _ => {}
                },
                Esc::Ss3 => {}
                Esc::Csi(mut params) => {
                    if ('@'..='~').contains(&c) {
                        if c == '~' {
                            match params.as_str() {
                                "200" => {
                                    self.paste = true;
                                    self.pasted = true;
                                }
                                "201" => self.paste = false,
                                _ => {}
                            }
                        }
                    } else {
                        params.push(c);
                        self.esc = Esc::Csi(params);
                    }
                }
            }
        }
        if self.text.is_empty() && !self.paste {
            self.pasted = false;
        }
    }

    pub fn program_keys(&mut self, bytes: &[u8]) {
        if bytes.iter().any(|b| matches!(b, b'\r' | b'\n' | 0x03 | 0x15)) {
            self.clear();
        }
    }

    fn plain(&mut self, c: char) {
        match c {
            '\x1b' => self.esc = Esc::Start,
            '\r' | '\n' if self.paste => self.text.push('\n'),
            '\r' | '\n' | '\x03' | '\x15' => self.text.clear(),
            '\x7f' | '\x08' => {
                self.text.pop();
            }
            '\x17' => {
                let kept = self.text.trim_end().len();
                self.text.truncate(kept);
                let cut = self.text.rfind(' ').map(|i| i + 1).unwrap_or(0);
                self.text.truncate(cut);
            }
            c if c.is_control() => {}
            c => self.text.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_opens_and_enter_closes() {
        let mut d = Draft::default();
        d.feed("ls -la");
        assert!(d.active());
        d.feed("\r");
        assert!(!d.active());
    }

    #[test]
    fn backspace_and_ctrl_u_clear() {
        let mut d = Draft::default();
        d.feed("ab\x7f\x7f");
        assert!(!d.active());
        d.feed("hello world\x15");
        assert!(!d.active());
        d.feed("one two\x17");
        assert!(d.active());
        d.feed("\x17");
        assert!(!d.active());
    }

    #[test]
    fn arrows_do_not_count_and_paste_keeps_newlines() {
        let mut d = Draft::default();
        d.feed("\x1b[A\x1b[B\x1bOA");
        assert!(!d.active());
        d.feed("\x1b[200~line one\nline two\x1b[201~");
        assert!(d.active());
        d.feed("\x03");
        assert!(!d.active());
    }

    #[test]
    fn a_draft_not_on_screen_goes_stale_after_a_quiet_time() {
        let mut d = Draft::default();
        d.feed("please check this");
        let later = Instant::now() + Duration::from_millis(STALE_MS + 1);
        assert!(!d.stale("\u{276f}", Instant::now()));
        assert!(d.stale("\u{276f}", later));
        assert!(!d.stale("\u{276f} please check\nthis", later));
        d.clear();
        assert!(!d.stale("\u{276f}", later));
    }

    #[test]
    fn a_pasted_draft_never_goes_stale() {
        let mut d = Draft::default();
        d.feed("\x1b[200~long text\x1b[201~");
        let later = Instant::now() + Duration::from_millis(STALE_MS + 1);
        assert!(d.active());
        assert!(!d.stale("[Pasted text #1]", later));
    }

    #[test]
    fn a_short_draft_is_shown_only_at_a_line_end() {
        let mut d = Draft::default();
        d.feed("1");
        let later = Instant::now() + Duration::from_millis(STALE_MS + 1);
        assert!(d.stale("\u{276f}\n1 file changed, 2 lines\n\u{23f5}\u{23f5} bypass permissions on", later));
        assert!(!d.stale("\u{2502} \u{276f} 1   \u{2502}\n\u{23f5}\u{23f5} bypass permissions on", later));
    }

    #[test]
    fn a_sent_paste_does_not_keep_later_typing_from_going_stale() {
        let mut d = Draft::default();
        d.feed("\x1b[200~long text\x1b[201~");
        d.feed("\r");
        d.feed("y");
        let later = Instant::now() + Duration::from_millis(STALE_MS + 1);
        assert!(d.stale("\u{276f}", later));
    }

    #[test]
    fn program_enter_clears() {
        let mut d = Draft::default();
        d.feed("abc");
        d.program_keys(b"x");
        assert!(d.active());
        d.program_keys(b"\r");
        assert!(!d.active());
    }
}
