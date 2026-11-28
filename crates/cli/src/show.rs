use nixa_protocol::{Event, InterruptReport, Status, TermInfo, WaitOutcome, WaitResult};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn ms_text(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1_000.0)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn ago(ms: u64) -> String {
    let secs = now_ms().saturating_sub(ms) / 1_000;
    match secs {
        0..60 => format!("{secs}s ago"),
        60..3_600 => format!("{}m ago", secs / 60),
        3_600..86_400 => format!("{}h ago", secs / 3_600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

fn or_dash(s: &str) -> String {
    if s.is_empty() { "-".to_string() } else { s.to_string() }
}

fn status_word(s: Option<Status>) -> String {
    s.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string())
}

pub fn run_text(t: &TermInfo) -> String {
    if t.running {
        return "running".to_string();
    }
    match t.exit_code {
        Some(c) => format!("exit {c}"),
        None => "ended".to_string(),
    }
}

fn quote(a: &str) -> String {
    let plain = !a.is_empty()
        && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

pub fn cmd_text(cmd: &[String]) -> String {
    if cmd.is_empty() {
        return "(shell)".to_string();
    }
    cmd.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut c: String = s.chars().take(max.saturating_sub(1)).collect();
    c.push('…');
    c
}

pub fn labels_text(labels: &BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ids_text<'a>(ids: impl Iterator<Item = &'a nixa_protocol::TermId>) -> String {
    ids.map(|id| id.to_string()).collect::<Vec<_>>().join(" ")
}

pub fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if i + 1 < row.len() {
                line.push_str(&" ".repeat(widths[i] - cell.chars().count() + 2));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

pub fn term_table(terms: &[TermInfo]) -> String {
    let head = ["ID", "STATUS", "RUN", "NAME", "PARENT", "COMMAND"];
    let mut rows = vec![head.iter().map(|s| s.to_string()).collect::<Vec<_>>()];
    for t in terms {
        let mut status = status_word(t.status);
        if t.draft {
            status.push('*');
        }
        rows.push(vec![
            t.id.to_string(),
            status,
            run_text(t),
            or_dash(&t.name),
            t.parent.as_ref().map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
            clip(&cmd_text(&t.cmd), 60),
        ]);
    }
    table(&rows)
}

pub fn term_detail(t: &TermInfo) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut add = |k: &str, v: String| rows.push(vec![k.to_string(), v]);
    add("id", t.id.to_string());
    add("name", or_dash(&t.name));
    let status = match (t.status, &t.status_note) {
        (Some(s), Some(n)) => format!("{s}: {n}"),
        (s, _) => status_word(s),
    };
    add("status", status);
    let run = match t.pid {
        Some(pid) if t.running => format!("running, pid {pid}"),
        _ => run_text(t),
    };
    add("run", run);
    add("command", cmd_text(&t.cmd));
    add("cwd", t.cwd.clone());
    if let Some(p) = &t.parent {
        add("parent", p.to_string());
    }
    if !t.children.is_empty() {
        add("children", ids_text(t.children.iter()));
    }
    if !t.labels.is_empty() {
        add("labels", labels_text(&t.labels));
    }
    if let Some(title) = &t.title {
        add("title", title.clone());
    }
    add("size", format!("{}x{}", t.cols, t.rows));
    add("keep", if t.keep { "on" } else { "off" }.to_string());
    if t.draft {
        add("draft", "open".to_string());
    }
    add("created", ago(t.created_ms));
    table(&rows)
}

pub fn event_line(e: &Event) -> String {
    match e {
        Event::Created { term } => format!(
            "{} created {} {}",
            term.id,
            or_dash(&term.name),
            cmd_text(&term.cmd)
        ),
        Event::Status { id, status: Some(s), note: Some(n) } => format!("{id} status {s}: {n}"),
        Event::Status { id, status: Some(s), .. } => format!("{id} status {s}"),
        Event::Status { id, .. } => format!("{id} status cleared"),
        Event::Exit { id, code: Some(c) } => format!("{id} exit {c}"),
        Event::Exit { id, .. } => format!("{id} exit"),
        Event::Respawned { id, pid: Some(p) } => format!("{id} respawned, pid {p}"),
        Event::Respawned { id, .. } => format!("{id} respawned"),
        Event::Removed { id } => format!("{id} removed"),
        Event::ParentCleared { id, parent } => format!("{id} detached from {parent}"),
        Event::Label { id, labels } if labels.is_empty() => format!("{id} labels cleared"),
        Event::Label { id, labels } => format!("{id} labels {}", labels_text(labels)),
        Event::Renamed { id, name } => format!("{id} renamed {name}"),
        Event::Title { id, title } => format!("{id} title {}", title.as_deref().unwrap_or("-")),
        Event::Resized { id, rows, cols } => format!("{id} resized {cols}x{rows}"),
        Event::Draft { id, active: true } => format!("{id} draft open"),
        Event::Draft { id, .. } => format!("{id} draft closed"),
        Event::OutputChanged { id, seq } => format!("{id} output {seq}"),
        Event::ChildNote { id, child, note, .. } => format!("{id} child {child}: {note}"),
    }
}

pub fn wait_result(r: &WaitResult) -> String {
    let after = ms_text(r.waited_ms);
    match r.outcome {
        WaitOutcome::Met => {
            let what = match (&r.matched, r.status) {
                (Some(m), _) => format!("matched {m:?}"),
                (None, Some(s)) => format!("status {s}"),
                (None, None) => "done".to_string(),
            };
            format!("{} {what} after {after}", r.id)
        }
        WaitOutcome::Timeout => format!("{} timed out after {after}", r.id),
        WaitOutcome::Exited => format!("{} exited after {after}", r.id),
    }
}

pub fn interrupt_report(r: &InterruptReport) -> String {
    let state = if r.running {
        "still running".to_string()
    } else {
        match r.exit_code {
            Some(c) => format!("exit {c}"),
            None => "ended".to_string(),
        }
    };
    let mut text = format!(
        "{} interrupted: {} -> {}, {state}\n",
        r.id,
        status_word(r.status_before),
        status_word(r.status_after)
    );
    let tail = r.tail.trim_end();
    if !tail.is_empty() {
        text.push_str(tail);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::TermId;

    #[test]
    fn table_pads_columns() {
        let rows = vec![
            vec!["ID".to_string(), "NAME".to_string(), "X".to_string()],
            vec!["m10".to_string(), "a".to_string(), "".to_string()],
        ];
        assert_eq!(table(&rows), "ID   NAME  X\nm10  a\n");
    }

    #[test]
    fn commands_are_quoted() {
        let cmd = vec!["echo".to_string(), "a b".to_string(), "it's".to_string()];
        assert_eq!(cmd_text(&cmd), "echo 'a b' 'it'\\''s'");
        assert_eq!(cmd_text(&[]), "(shell)");
        assert_eq!(clip("abcdef", 4), "abc…");
    }

    #[test]
    fn event_lines() {
        let id = TermId::from_number(1);
        let e = Event::Status { id: id.clone(), status: Some(Status::Blocked), note: Some("needs a yes".into()) };
        assert_eq!(event_line(&e), "m1 status blocked: needs a yes");
        let e = Event::ChildNote {
            id: id.clone(),
            child: TermId::from_number(2),
            status: Some(Status::Done),
            exited: false,
            note: "done".into(),
        };
        assert_eq!(event_line(&e), "m1 child m2: done");
    }

    #[test]
    fn times() {
        assert_eq!(ms_text(250), "250ms");
        assert_eq!(ms_text(1_500), "1.5s");
        assert_eq!(ago(now_ms()), "0s ago");
    }
}
