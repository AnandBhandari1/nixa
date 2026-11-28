use alacritty_terminal::vte::{Params, Parser, Perform};
use nixa_protocol::Status;
use nixa_protocol::osc::parse_status_osc;

const KEEP_BYTES: usize = 1 << 20;

#[derive(Default)]
struct Sink {
    text: String,
    base: usize,
    statuses: Vec<(Status, Option<String>)>,
}

impl Sink {
    fn newline(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
    }
}

impl Perform for Sink {
    fn print(&mut self, c: char) {
        self.text.push(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.text.push('\n'),
            b'\t' => self.text.push('\t'),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, action: char) {
        match action {
            'C' => self.text.push(' '),
            'H' | 'f' | 'd' | 'E' | 'F' => self.newline(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if let Some(found) = parse_status_osc(params) {
            self.statuses.push(found);
        }
    }
}

pub struct Output {
    parser: Parser,
    sink: Sink,
}

pub fn new_output() -> Output {
    Output {
        parser: Parser::new(),
        sink: Sink::default(),
    }
}

impl Output {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.sink, bytes);
        let len = self.sink.text.len();
        if len > 2 * KEEP_BYTES {
            let mut cut = len - KEEP_BYTES;
            while !self.sink.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.sink.text.drain(..cut);
            self.sink.base += cut;
        }
    }

    pub fn end(&self) -> usize {
        self.sink.base + self.sink.text.len()
    }

    pub fn since(&self, pos: usize) -> &str {
        let start = pos.saturating_sub(self.sink.base);
        self.sink.text.get(start..).unwrap_or("")
    }

    pub fn take_statuses(&mut self) -> Vec<(Status, Option<String>)> {
        std::mem::take(&mut self.sink.statuses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::osc::status_osc;

    #[test]
    fn printed_text_is_kept_in_order() {
        let mut out = new_output();
        let start = out.end();
        out.feed(b"echo hi\r\nhi\r\n\x1b[1;32mgreen\x1b[0m\x1b[2Cx");
        assert_eq!(out.since(start), "echo hi\nhi\ngreen x");
        let mid = out.end();
        out.feed(b"more");
        assert_eq!(out.since(mid), "more");
    }

    #[test]
    fn status_escapes_are_read() {
        let mut out = new_output();
        out.feed(status_osc(Status::Blocked, Some("say yes; or no")).as_bytes());
        out.feed(b"\x1b]7421;status=done\x1b\\");
        out.feed(b"\x1b]7421;status=error;note=disk full\x07");
        out.feed(b"\x1b]7700;working\x07");
        out.feed(b"\x1b]0;a title\x07");
        assert_eq!(
            out.take_statuses(),
            vec![
                (Status::Blocked, Some("say yes; or no".to_string())),
                (Status::Done, None),
                (Status::Error, Some("disk full".to_string())),
            ]
        );
        assert!(out.take_statuses().is_empty());
    }

    #[test]
    fn long_output_is_trimmed() {
        let mut out = new_output();
        let chunk = vec![b'a'; 64 * 1024];
        for _ in 0..40 {
            out.feed(&chunk);
        }
        assert_eq!(out.end(), 40 * 64 * 1024);
        assert!(out.since(0).len() <= 2 * KEEP_BYTES);
    }
}
