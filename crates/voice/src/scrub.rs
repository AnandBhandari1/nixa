use regex::{Captures, Regex};
use std::sync::OnceLock;

struct Rules {
    secret: Regex,
    long_token: Regex,
    fence: Regex,
    tick: Regex,
    table: Regex,
    image: Regex,
    link: Regex,
    url: Regex,
    uuid: Regex,
    hex: Regex,
    path: Regex,
    tail_path: Regex,
    heading: Regex,
    emphasis: Regex,
    bullet: Regex,
    number: Regex,
    marks: Regex,
    spaces: Regex,
    gaps: Regex,
    dots: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |p: &str| Regex::new(p).expect("scrub rule");
        Rules {
            secret: r(r"sk-[A-Za-z0-9_-]{16,}|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{30,}|eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"),
            long_token: r(r"[A-Za-z0-9+=_-]{32,}"),
            fence: r(r"(?s)```.*?```"),
            tick: r(r"`([^`]*)`"),
            table: r(r"(?m)^[ \t]*\|.*$"),
            image: r(r"!\[[^\]]*\]\([^)]*\)"),
            link: r(r"\[([^\]]*)\]\([^)]*\)"),
            url: r(r"https?://\S+"),
            uuid: r(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F-]{20,}\b"),
            hex: r(r"\b[0-9a-fA-F]{12,}\b"),
            path: r(r"(^|[^\w/])/[\w.-]+(?:/[\w.-]+)+"),
            tail_path: r(r"(\w)/[\w./-]{8,}"),
            heading: r(r"(?m)^[ \t]{0,3}#{1,6}[ \t]*"),
            emphasis: r(r"[*_]{1,3}([^*_]+)[*_]{1,3}"),
            bullet: r(r"(?m)^[ \t]*[-*+][ \t]+"),
            number: r(r"(?m)^[ \t]*\d+\.[ \t]+"),
            marks: r(r"[|>#`~]"),
            spaces: r(r"[ \t]+"),
            gaps: r(r"\n{2,}"),
            dots: r(r"\s*\.\s*(\.\s*)+"),
        }
    })
}

pub fn scrub_for_speech(text: &str) -> String {
    let r = rules();
    let t = r.secret.replace_all(text, " secret ");
    let t = r.long_token.replace_all(&t, |c: &Captures| {
        let s = &c[0];
        if s.chars().any(|ch| ch.is_ascii_digit()) && s.chars().any(|ch| ch.is_ascii_alphabetic()) {
            " secret ".to_string()
        } else {
            s.to_string()
        }
    });
    let t = r.fence.replace_all(&t, " . code block . ");
    let t = r.tick.replace_all(&t, "$1");
    let t = r.table.replace_all(&t, "");
    let t = r.image.replace_all(&t, " ");
    let t = r.link.replace_all(&t, "$1");
    let t = r.url.replace_all(&t, " link ");
    let t = r.uuid.replace_all(&t, " identifier ");
    let t = r.hex.replace_all(&t, " identifier ");
    let t = r.path.replace_all(&t, "${1} path ");
    let t = r.tail_path.replace_all(&t, "${1} path ");
    let t = r.heading.replace_all(&t, "");
    let t = r.emphasis.replace_all(&t, "$1");
    let t = r.bullet.replace_all(&t, "");
    let t = r.number.replace_all(&t, "");
    let t = r.marks.replace_all(&t, " ");
    let t = r.spaces.replace_all(&t, " ");
    let t = r.gaps.replace_all(&t, ". ");
    let t = r.dots.replace_all(&t, ". ");
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_markdown_links_and_paths() {
        let out = scrub_for_speech("## Done\n\n- See [the docs](https://x.y/z) and `cargo test` in /Users/me/project/src now.");
        assert_eq!(out, "Done. See the docs and cargo test in path now.");
    }

    #[test]
    fn hides_secrets_and_code() {
        let out = scrub_for_speech("key sk-abcdefghijklmnopqrstu here ```\nlet x = 1;\n``` end");
        assert!(out.contains("secret"));
        assert!(out.contains("code block"));
        assert!(!out.contains("sk-"));
    }

    #[test]
    fn keeps_plain_words() {
        assert_eq!(scrub_for_speech("All tests pass."), "All tests pass.");
    }
}
