use crate::args::{Parsed, usage};
use crate::ctx::{self, CALL_TIMEOUT, Ctx};
use anyhow::{Result, bail};
use nixa_protocol::{Cell, Color, Event, Reply, Request, Snapshot, TermId, TermInfo, flags, paths};
use std::fs::File;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

pub const DETACH_BYTE: u8 = 0x1d;
pub const DETACH_NAME: &str = "ctrl+]";
const TTY: &str = "/dev/tty";
const TICK: Duration = Duration::from_millis(200);

enum Msg {
    Output,
    Keys(Vec<u8>),
    Detach,
    Ended(Option<i32>),
    Gone,
}

struct Tty {
    saved: String,
}

impl Drop for Tty {
    fn drop(&mut self) {
        let _ = stty(&[&self.saved]);
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[0m\x1b[?1l\x1b[?2004l\x1b[?25h\x1b[?1049l");
        let _ = out.flush();
    }
}

fn stty(args: &[&str]) -> Result<String> {
    let tty = File::open(TTY)?;
    let done = Command::new("stty").args(args).stdin(Stdio::from(tty)).stderr(Stdio::null()).output()?;
    if !done.status.success() {
        bail!("stty {} failed", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&done.stdout).trim().to_string())
}

fn raw_mode() -> Result<Tty> {
    let saved = stty(&["-g"])?;
    stty(&["raw", "-echo"])?;
    Ok(Tty { saved })
}

pub fn parse_size(text: &str) -> Option<(u16, u16)> {
    let mut parts = text.split_whitespace();
    let rows = parts.next()?.parse::<u16>().ok()?;
    let cols = parts.next()?.parse::<u16>().ok()?;
    (rows > 1 && cols > 1).then_some((rows, cols))
}

fn tty_size() -> Option<(u16, u16)> {
    stty(&["size"]).ok().and_then(|s| parse_size(&s))
}

pub fn pick_index(line: &str, count: usize) -> Option<usize> {
    let n = line.trim().parse::<usize>().ok()?;
    (1..=count).contains(&n).then(|| n - 1)
}

fn choose(ctx: &Ctx, out: &mut dyn Write) -> Result<Option<TermId>> {
    let terms = match ctx::call(ctx, &Request::List { parent: None, label: None })? {
        Reply::Terms { terms } => terms,
        other => return Err(ctx::unexpected(&other)),
    };
    let own = paths::current_term_id();
    let live: Vec<TermInfo> = terms.into_iter().filter(|t| t.running && Some(&t.id) != own.as_ref()).collect();
    if live.is_empty() {
        writeln!(out, "no live terminals to attach")?;
        return Ok(None);
    }
    for (n, t) in live.iter().enumerate() {
        writeln!(out, "{:>3}  {}  {}  {}", n + 1, t.id, t.name, status_text(t))?;
    }
    write!(out, "pick a number (enter to cancel): ")?;
    out.flush()?;
    let mut line = String::new();
    BufReader::new(File::open(TTY)?).read_line(&mut line)?;
    if line.trim().is_empty() {
        return Ok(None);
    }
    match pick_index(&line, live.len()) {
        Some(i) => Ok(Some(live[i].id.clone())),
        None => Err(usage(format!("{:?} is not a number from the list", line.trim()))),
    }
}

fn status_text(t: &TermInfo) -> String {
    t.status.map(|s| s.to_string()).unwrap_or_else(|| "no status".to_string())
}

pub fn header(t: &TermInfo, cols: u16) -> String {
    let text = format!(" {}  {}  {}  |  detach: {DETACH_NAME} ", t.id, t.name, status_text(t));
    let mut line: String = text.chars().take(cols as usize).collect();
    let pad = (cols as usize).saturating_sub(line.chars().count());
    line.push_str(&" ".repeat(pad));
    line
}

pub fn run(ctx: &Ctx, p: Parsed, out: &mut dyn Write) -> Result<i32> {
    p.finish(1, false)?;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(usage("nixa term attach needs a real terminal for input and output"));
    }
    let id = match p.pos.first() {
        Some(s) => super::term_id(s)?,
        None => match choose(ctx, out)? {
            Some(id) => id,
            None => return Ok(0),
        },
    };
    if paths::current_term_id().as_ref() == Some(&id) {
        return Err(usage(format!("{id} is this terminal. Attach to another terminal.")));
    }
    let term = match ctx::call(ctx, &Request::Get { id: id.clone() })? {
        Reply::Term { term } => term,
        other => return Err(ctx::unexpected(&other)),
    };
    if !term.running {
        bail!("{id} has ended. Read it with: nixa term read {id} --history");
    }
    writeln!(out, "attaching to {id} ({}). Press {DETACH_NAME} to detach.", term.name)?;
    out.flush()?;
    let before = (term.rows, term.cols);
    let result = {
        let _tty = raw_mode()?;
        session(ctx, &term, out)
    };
    if let Ok(mut client) = ctx::connect(ctx, Some(CALL_TIMEOUT)) {
        let _ = client.call(&Request::Resize { id: id.clone(), rows: before.0, cols: before.1 });
    }
    match result? {
        Some(code) => {
            writeln!(out, "{id} ended (code {})", code.map(|c| c.to_string()).unwrap_or_else(|| "-".into()))?;
            Ok(0)
        }
        None => {
            writeln!(out, "detached from {id}")?;
            Ok(0)
        }
    }
}

fn watch(ctx: &Ctx, id: &TermId, tx: Sender<Msg>) -> Result<()> {
    let client = ctx::connect(ctx, None)?;
    let id = id.clone();
    std::thread::spawn(move || {
        let _ = client.subscribe(vec![id], true, |event| {
            let msg = match event {
                Event::Exit { code, .. } => Msg::Ended(code),
                Event::Removed { .. } => Msg::Gone,
                _ => Msg::Output,
            };
            tx.send(msg).is_ok()
        });
        let _ = tx.send(Msg::Gone);
    });
    Ok(())
}

pub fn split_detach(bytes: &[u8]) -> (&[u8], bool) {
    match bytes.iter().position(|b| *b == DETACH_BYTE) {
        Some(at) => (&bytes[..at], true),
        None => (bytes, false),
    }
}

fn keyboard(tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            let n = match stdin.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(Msg::Detach);
                    return;
                }
                Ok(n) => n,
            };
            let (keys, detach) = split_detach(&buf[..n]);
            if !keys.is_empty() && tx.send(Msg::Keys(keys.to_vec())).is_err() {
                return;
            }
            if detach {
                let _ = tx.send(Msg::Detach);
                return;
            }
        }
    });
}

pub fn take_text(pending: &mut Vec<u8>) -> String {
    let good = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => pending.len(),
    };
    let text = String::from_utf8_lossy(&pending[..good]).to_string();
    pending.drain(..good);
    text
}

fn session(ctx: &Ctx, term: &TermInfo, out: &mut dyn Write) -> Result<Option<Option<i32>>> {
    let id = term.id.clone();
    let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
    watch(ctx, &id, tx.clone())?;
    keyboard(tx);
    let mut client = ctx::connect(ctx, Some(CALL_TIMEOUT))?;
    out.write_all(b"\x1b[?1049h\x1b[2J")?;
    let mut size = (0u16, 0u16);
    let mut seq = None;
    let mut pending = Vec::new();
    let mut info = term.clone();
    loop {
        let now = tty_size().unwrap_or((term.rows + 1, term.cols));
        let resized = now != size;
        if resized {
            size = now;
            client.call(&Request::Resize { id: id.clone(), rows: size.0 - 1, cols: size.1 })?;
            seq = None;
            out.write_all(b"\x1b[2J")?;
        }
        if seq.is_none() {
            if let Ok(Reply::Term { term }) = client.call(&Request::Get { id: id.clone() }) {
                info = term;
            }
            seq = draw(&mut client, &info, size, None, out)?.or(seq);
        }
        let msg = match rx.recv_timeout(TICK) {
            Ok(msg) => msg,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
        };
        match msg {
            Msg::Output => seq = draw(&mut client, &info, size, seq, out)?.or(seq),
            Msg::Keys(bytes) => {
                pending.extend_from_slice(&bytes);
                let text = take_text(&mut pending);
                if !text.is_empty() {
                    client.call(&Request::Input { id: id.clone(), text })?;
                }
            }
            Msg::Detach => return Ok(None),
            Msg::Ended(code) if !info.keep => return Ok(Some(code)),
            Msg::Ended(_) => {}
            Msg::Gone => return Ok(Some(None)),
        }
    }
}

fn draw(
    client: &mut nixa_protocol::Client,
    info: &TermInfo,
    size: (u16, u16),
    since: Option<u64>,
    out: &mut dyn Write,
) -> Result<Option<u64>> {
    let snap = match client.call(&Request::Snapshot { id: info.id.clone(), scroll: 0, since_seq: since })? {
        Reply::Snapshot { snapshot } => snapshot,
        Reply::Unchanged { .. } => return Ok(None),
        other => return Err(ctx::unexpected(&other)),
    };
    out.write_all(render(&snap, info, size).as_bytes())?;
    out.flush()?;
    Ok(Some(snap.seq))
}

fn color(c: Color, fg: bool, codes: &mut Vec<String>) {
    let base = if fg { 38 } else { 48 };
    match c {
        Color::Default => {}
        Color::Indexed(n) => codes.push(format!("{base};5;{n}")),
        Color::Rgb(r, g, b) => codes.push(format!("{base};2;{r};{g};{b}")),
    }
}

pub fn sgr(cell: &Cell) -> String {
    let mut codes = vec!["0".to_string()];
    for (flag, code) in [
        (flags::BOLD, "1"),
        (flags::DIM, "2"),
        (flags::ITALIC, "3"),
        (flags::UNDERLINE, "4"),
        (flags::INVERSE, "7"),
        (flags::HIDDEN, "8"),
        (flags::STRIKE, "9"),
    ] {
        if cell.has(flag) {
            codes.push(code.to_string());
        }
    }
    color(cell.fg, true, &mut codes);
    color(cell.bg, false, &mut codes);
    format!("\x1b[{}m", codes.join(";"))
}

pub fn render(snap: &Snapshot, info: &TermInfo, size: (u16, u16)) -> String {
    let mut s = String::from("\x1b[?25l\x1b[H\x1b[0;7m");
    s.push_str(&header(info, size.1));
    s.push_str("\x1b[0m");
    let rows = snap.rows.min(size.0.saturating_sub(1));
    let cols = snap.cols.min(size.1) as usize;
    for r in 0..rows {
        s.push_str(&format!("\x1b[{};1H", r + 2));
        let mut last = String::new();
        let mut used = 0usize;
        for cell in snap.row(r) {
            if cell.has(flags::WIDE_SPACER) {
                continue;
            }
            let width = if cell.has(flags::WIDE) { 2 } else { 1 };
            if used + width > cols {
                break;
            }
            let style = sgr(cell);
            if style != last {
                s.push_str(&style);
                last = style;
            }
            s.push(cell.ch);
            if let Some(z) = &cell.zerowidth {
                s.push_str(z);
            }
            used += width;
        }
        s.push_str("\x1b[0m\x1b[K");
    }
    s.push_str(if snap.modes.app_cursor { "\x1b[?1h" } else { "\x1b[?1l" });
    s.push_str(if snap.modes.bracketed_paste { "\x1b[?2004h" } else { "\x1b[?2004l" });
    s.push_str(&format!("\x1b[{};{}H", snap.cursor.row + 2, snap.cursor.col + 1));
    if snap.cursor.visible {
        s.push_str("\x1b[?25h");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::Cursor;

    fn info() -> TermInfo {
        serde_json::from_value(serde_json::json!({
            "id": "m1", "name": "build", "cwd": "/", "running": true, "rows": 2, "cols": 3, "created_ms": 0
        }))
        .unwrap()
    }

    #[test]
    fn sizes_and_picks() {
        assert_eq!(parse_size("40 120\n"), Some((40, 120)));
        assert_eq!(parse_size("0 0"), None);
        assert_eq!(parse_size("x"), None);
        assert_eq!(pick_index(" 2\n", 3), Some(1));
        assert_eq!(pick_index("0", 3), None);
        assert_eq!(pick_index("4", 3), None);
    }

    #[test]
    fn detach_key_cuts_the_input() {
        assert_eq!(split_detach(b"ab\x1dcd"), (&b"ab"[..], true));
        assert_eq!(split_detach(b"ab"), (&b"ab"[..], false));
    }

    #[test]
    fn split_utf8_waits_for_the_rest() {
        let mut pending = "é".as_bytes()[..1].to_vec();
        assert_eq!(take_text(&mut pending), "");
        pending.push("é".as_bytes()[1]);
        assert_eq!(take_text(&mut pending), "é");
        assert!(pending.is_empty());
    }

    #[test]
    fn header_fits_the_width() {
        let h = header(&info(), 20);
        assert_eq!(h.chars().count(), 20);
        assert!(h.starts_with(" m1  build"));
        assert!(header(&info(), 80).contains(DETACH_NAME));
    }

    #[test]
    fn render_draws_cells_and_cursor() {
        let mut bold = Cell { ch: 'b', ..Cell::default() };
        bold.flags = flags::BOLD;
        bold.fg = Color::Indexed(1);
        let snap = Snapshot {
            id: TermId::from_number(1),
            seq: 1,
            rows: 2,
            cols: 3,
            cells: vec![Cell { ch: 'a', ..Cell::default() }, bold, Cell::default(), Cell::default(), Cell::default(), Cell::default()],
            cursor: Cursor { row: 1, col: 2, visible: true, ..Cursor::default() },
            modes: Default::default(),
            title: None,
            history: 0,
            scroll: 0,
        };
        assert_eq!(sgr(&snap.cells[1]), "\x1b[0;1;38;5;1m");
        let text = render(&snap, &info(), (3, 3));
        assert!(text.contains("\x1b[2;1H\x1b[0ma\x1b[0;1;38;5;1mb"));
        assert!(text.ends_with("\x1b[3;3H\x1b[?25h"));
    }
}
