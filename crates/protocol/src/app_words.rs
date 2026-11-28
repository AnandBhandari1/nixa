#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Word {
    pub say: &'static str,
    pub command: &'static str,
    pub arg: Option<&'static str>,
}

const fn word(say: &'static str, command: &'static str, arg: Option<&'static str>) -> Word {
    Word { say, command, arg }
}

pub const WORDS: [Word; 26] = [
    word("tab new", "tab.new", None),
    word("tab close", "app.close-tab", None),
    word("tab rename", "tab.rename", Some("name")),
    word("tab next", "app.next-tab", None),
    word("tab prev", "app.previous-tab", None),
    word("tab go", "tab.select", Some("id")),
    word("pane split right", "app.split-right", None),
    word("pane split down", "app.split-down", None),
    word("pane close", "app.close-pane", None),
    word("pane zoom", "app.zoom-pane", None),
    word("sidebar", "app.show-or-hide-the-sidebar", None),
    word("layout save", "layout.save", Some("name")),
    word("layout use", "layout.use", Some("name")),
    word("layout rm", "layout.delete", Some("name")),
    word("space next", "app.next-space", None),
    word("space prev", "app.previous-space", None),
    word("space rename", "space.rename", Some("name")),
    word("space lock", "space.lock", None),
    word("space open", "space.open", None),
    word("space close", "space.close", None),
    word("theme next", "theme.next-theme", None),
    word("theme prev", "theme.previous-theme", None),
    word("chat show", "chat.show", None),
    word("chat hide", "chat.hide", None),
    word("events clear", "events.clear-events", None),
    word("events", "events.show-events", None),
];

pub fn find(argv: &[String]) -> Option<(Word, &[String])> {
    WORDS
        .iter()
        .filter_map(|w| {
            let parts: Vec<&str> = w.say.split(' ').collect();
            let fits = argv.len() >= parts.len() && parts.iter().zip(argv).all(|(p, a)| p == a);
            fits.then(|| (*w, &argv[parts.len()..]))
        })
        .max_by_key(|(w, _)| w.say.split(' ').count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_longest_words_win_and_the_rest_is_kept() {
        let argv = v(&["events", "clear"]);
        let (w, rest) = find(&argv).unwrap();
        assert_eq!(w.command, "events.clear-events");
        assert!(rest.is_empty());
        let argv = v(&["events"]);
        assert_eq!(find(&argv).unwrap().0.command, "events.show-events");
        let argv = v(&["tab", "rename", "my", "tab"]);
        let (w, rest) = find(&argv).unwrap();
        assert_eq!((w.command, w.arg), ("tab.rename", Some("name")));
        assert_eq!(rest, v(&["my", "tab"]).as_slice());
        assert!(find(&v(&["pane", "split"])).is_none());
        assert!(find(&v(&["nope"])).is_none());
    }

    #[test]
    fn every_word_and_command_is_used_once() {
        for (i, a) in WORDS.iter().enumerate() {
            for b in &WORDS[i + 1..] {
                assert_ne!(a.say, b.say);
                assert_ne!(a.command, b.command);
            }
        }
    }
}
