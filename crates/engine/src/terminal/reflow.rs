use super::{Inner, Terminal, lock};
use crate::engine::Engine;
use anyhow::Result;
use portable_pty::PtySize;
use std::time::{Duration, Instant};

const REDRAW_WAIT: Duration = Duration::from_millis(60);
const MAX_PREFIX: usize = 4096;
const CURSOR_UP: &[u8] = b"\x1b[A";
const CLEAR_ZLE: &[u8] = b"\x1b[0m\x1b[27m\x1b[24m\x1b[J";

#[derive(Debug, PartialEq, Eq)]
enum Prefix {
    Partial,
    Clear(usize),
    Other,
}

fn redraw_prefix(bytes: &[u8]) -> Prefix {
    if bytes.is_empty() {
        return Prefix::Partial;
    }
    let resets = bytes.iter().take_while(|&&byte| byte == b'\r').count();
    if resets == 0 {
        return Prefix::Other;
    }
    let mut at = resets;
    while bytes[at..].starts_with(CURSOR_UP) {
        at += CURSOR_UP.len();
    }
    let rest = &bytes[at..];
    if rest.starts_with(CLEAR_ZLE) {
        Prefix::Clear(at + CLEAR_ZLE.len())
    } else if CLEAR_ZLE.starts_with(rest) || CURSOR_UP.starts_with(rest) {
        Prefix::Partial
    } else {
        Prefix::Other
    }
}

pub(super) struct Pending {
    rows: u16,
    cols: u16,
    prefix: Vec<u8>,
}

impl Pending {
    fn finish(self, term: &mut Inner, clear: usize) {
        term.screen.feed(&self.prefix[..clear]);
        term.screen.resize(self.rows, self.cols);
        term.screen.feed(&self.prefix[clear..]);
    }
}

pub(super) fn feed(term: &mut Inner, bytes: &[u8]) {
    let Some(mut pending) = term.resize_pending.take() else {
        term.screen.feed(bytes);
        return;
    };
    let inspect = bytes.len().min(MAX_PREFIX - pending.prefix.len());
    pending.prefix.extend_from_slice(&bytes[..inspect]);
    match redraw_prefix(&pending.prefix) {
        Prefix::Partial if pending.prefix.len() < MAX_PREFIX => {
            term.resize_pending = Some(pending);
            return;
        }
        Prefix::Clear(end) => pending.finish(term, end),
        _ => pending.finish(term, 0),
    }
    term.screen.feed(&bytes[inspect..]);
}

pub fn resize(engine: &Engine, term: &Terminal, rows: u16, cols: u16) -> Result<Option<u64>> {
    let _serial = term.resize_serial.lock().unwrap_or_else(|e| e.into_inner());
    {
        let mut inner = lock(term);
        if (rows, cols) == (inner.screen.rows(), inner.screen.cols()) {
            return Ok(None);
        }
        let mut wait_for_redraw = false;
        if let Some(pty) = &inner.pty {
            let modes = inner.screen.modes();
            wait_for_redraw = pty.zsh
                && modes.bracketed_paste
                && !modes.alt_screen
                && pty
                    .master
                    .process_group_leader()
                    .and_then(|group| u32::try_from(group).ok())
                    == inner.pid;
            pty.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }
        if !wait_for_redraw {
            inner.screen.resize(rows, cols);
            inner.seq += 1;
            return Ok(Some(inner.seq));
        }
        inner.resize_pending = Some(Pending {
            rows,
            cols,
            prefix: Vec::new(),
        });
    }
    let _ = engine;
    let deadline = Instant::now() + REDRAW_WAIT;
    loop {
        let seen = term.tick();
        {
            let mut inner = lock(term);
            if inner.resize_pending.is_none() {
                return Ok(Some(inner.seq));
            }
            if Instant::now() >= deadline {
                if let Some(pending) = inner.resize_pending.take() {
                    pending.finish(&mut inner, 0);
                }
                inner.seq += 1;
                return Ok(Some(inner.seq));
            }
        }
        term.wait_change(seen, deadline);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zle_cleanup_must_be_complete_and_can_precede_text_in_the_same_read() {
        let prefix = b"\r\r\x1b[A\x1b[A\x1b[0m\x1b[27m\x1b[24m\x1b[J";
        for end in 0..prefix.len() {
            assert_eq!(redraw_prefix(&prefix[..end]), Prefix::Partial);
        }
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(b"prompt");
        assert_eq!(redraw_prefix(&bytes), Prefix::Clear(prefix.len()));
        assert_eq!(redraw_prefix(b"\r\r\x1b[Jlog"), Prefix::Other);
    }
}
