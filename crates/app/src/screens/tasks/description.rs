use super::model::{entity, safe_link, tag_name};
use crate::screens::settings::parts::memory::text_area::PreviewSpan;

#[derive(Default)]
struct Preview {
    text: String,
    spans: Vec<PreviewSpan>,
    bold: usize,
    italic: usize,
    code: usize,
    pre: bool,
    hidden: Option<&'static str>,
    pending_space: bool,
    link: Option<String>,
}

impl Preview {
    fn text(&mut self, input: &str) {
        if self.hidden.is_some() {
            return;
        }
        let start = self.text.len();
        let input = entity(input);
        if self.pre {
            self.text.push_str(&input);
        } else {
            for c in input.chars() {
                if c.is_ascii_whitespace() {
                    self.pending_space = true;
                } else {
                    if self.pending_space
                        && !self.text.is_empty()
                        && !self.text.ends_with(['\n', ' '])
                    {
                        self.text.push(' ');
                    }
                    self.pending_space = false;
                    self.text.push(c);
                }
            }
        }
        let end = self.text.len();
        if end > start && (self.bold > 0 || self.italic > 0 || self.code > 0 || self.link.is_some())
        {
            self.spans.push(PreviewSpan {
                range: start..end,
                bold: self.bold > 0,
                italic: self.italic > 0,
                code: self.code > 0,
                link: self.link.clone(),
            });
        }
    }

    fn newline(&mut self) {
        self.pending_space = false;
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
    }

    fn tag(&mut self, raw: &str) {
        let raw = raw.trim();
        let name = tag_name(raw);
        let closed = raw.starts_with('/');
        if let Some(hidden) = self.hidden {
            if closed && name == hidden {
                self.hidden = None;
            }
            return;
        }
        let depth = |value: usize| {
            if closed {
                value.saturating_sub(1)
            } else {
                value + 1
            }
        };
        match name.as_str() {
            "script" if !closed => self.hidden = Some("script"),
            "style" if !closed => self.hidden = Some("style"),
            "strong" | "b" => self.bold = depth(self.bold),
            "em" | "i" => self.italic = depth(self.italic),
            "code" => self.code = depth(self.code),
            "a" => self.link = if closed { None } else { href(raw) },
            "pre" => {
                self.newline();
                self.pre = !closed;
                self.code = depth(self.code);
            }
            "li" => {
                self.newline();
                if !closed {
                    self.text.push_str("• ");
                }
            }
            "td" | "th" if closed => self.pending_space = true,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.newline();
                self.bold = depth(self.bold);
            }
            "br" | "hr" | "p" | "div" | "ul" | "ol" | "tr" | "table" | "blockquote" => {
                self.newline()
            }
            _ => {}
        }
    }
}

fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => return Some(i),
            _ => {}
        }
    }
    None
}

fn href(tag: &str) -> Option<String> {
    let mut rest = tag.get(1..)?.trim_start();
    while !rest.is_empty() {
        let end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let key = &rest[..end];
        rest = rest[end..].trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let (href, tail) = if let Some(quote @ ('"' | '\'')) = value.chars().next() {
            let content = &value[1..];
            let end = content.find(quote)?;
            (&content[..end], &content[end + 1..])
        } else {
            let end = value.find(char::is_whitespace).unwrap_or(value.len());
            (&value[..end], &value[end..])
        };
        rest = tail.trim_start();
        if key.eq_ignore_ascii_case("href") {
            let url = entity(href.trim());
            return (safe_link(&url) && !url.chars().any(char::is_control)).then_some(url);
        }
    }
    None
}

pub(super) fn parse(mut input: &str) -> (String, Vec<PreviewSpan>) {
    let mut preview = Preview::default();
    while let Some(start) = input.find('<') {
        preview.text(&input[..start]);
        let after = &input[start + 1..];
        if let Some(comment) = after.strip_prefix("!--") {
            let Some(end) = comment.find("-->") else {
                input = "";
                break;
            };
            input = &comment[end + 3..];
            continue;
        }
        let Some(end) = tag_end(after) else {
            input = &input[start..];
            break;
        };
        preview.tag(&after[..end]);
        input = &after[end + 1..];
    }
    preview.text(input);
    let end = preview.text.trim_end().len();
    preview.text.truncate(end);
    preview.spans.retain_mut(|span| {
        span.range.end = span.range.end.min(end);
        !span.range.is_empty()
    });
    (preview.text, preview.spans)
}
