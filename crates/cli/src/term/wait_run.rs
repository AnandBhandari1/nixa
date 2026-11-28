use crate::ctx::{self, Ctx};
use anyhow::Result;
use nixa_protocol::{EngineError, ErrorCode, ReadSource, Reply, Request, TermId, TermInfo, WaitFor, WaitOutcome};
use std::io::Write;

const ROUND_MS: u64 = 6 * 60 * 60 * 1000;

pub fn clean_output(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let end = lines.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1);
    let mut out = lines[..end].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

pub fn code_from_note(note: &str) -> Option<i32> {
    let (_, tail) = note.rsplit_once("ended with code ")?;
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

fn is_gone(e: &anyhow::Error) -> bool {
    e.downcast_ref::<EngineError>().is_some_and(|e| e.code == ErrorCode::NotFound)
}

fn wait_exit(ctx: &Ctx, id: &TermId, timeout_ms: Option<u64>) -> Result<Option<WaitOutcome>> {
    loop {
        let round = timeout_ms.unwrap_or(ROUND_MS);
        let req = Request::Wait { id: id.clone(), until: WaitFor::Exit, timeout_ms: Some(round) };
        let result = match ctx::call_with(ctx, &req, None) {
            Ok(Reply::Waited { result }) => result,
            Ok(other) => return Err(ctx::unexpected(&other)),
            Err(e) if is_gone(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        if result.outcome != WaitOutcome::Timeout || timeout_ms.is_some() {
            return Ok(Some(result.outcome));
        }
    }
}

fn history(ctx: &Ctx, id: &TermId) -> Result<String> {
    match ctx::call(ctx, &Request::Read { id: id.clone(), source: ReadSource::History, lines: None })? {
        Reply::Text { text, .. } => Ok(text),
        other => Err(ctx::unexpected(&other)),
    }
}

fn from_parent(ctx: &Ctx, parent: &TermId, id: &TermId) -> (String, Option<i32>) {
    let text = match ctx::call(ctx, &Request::ChildOutput { id: parent.clone(), child: id.clone() }) {
        Ok(Reply::Text { text, .. }) => text,
        _ => String::new(),
    };
    let code = match ctx::call(ctx, &Request::ChildReports { id: parent.clone(), reader: None }) {
        Ok(Reply::ChildReports { reports }) => reports
            .iter()
            .rev()
            .find(|r| &r.child == id && r.exited)
            .and_then(|r| code_from_note(&r.note)),
        _ => None,
    };
    (text, code)
}

pub fn run(ctx: &Ctx, term: &TermInfo, timeout_ms: Option<u64>, out: &mut dyn Write) -> Result<i32> {
    let id = term.id.clone();
    let outcome = wait_exit(ctx, &id, timeout_ms)?;
    if outcome == Some(WaitOutcome::Timeout) {
        let text = clean_output(&history(ctx, &id).unwrap_or_default());
        if ctx.json {
            let v = serde_json::json!({"id": id, "timed_out": true, "exit_code": null, "output": text});
            writeln!(out, "{v}")?;
        } else {
            out.write_all(text.as_bytes())?;
        }
        eprintln!("nixa: {id} did not end in time. It still runs. See it with: nixa term attach {id}");
        return Ok(super::EXIT_TIMEOUT);
    }
    let (text, code) = match ctx::call(ctx, &Request::Get { id: id.clone() }) {
        Ok(Reply::Term { term: now }) => {
            let text = history(ctx, &id)?;
            let _ = ctx::call(ctx, &Request::Remove { id: id.clone() });
            (text, now.exit_code)
        }
        Ok(other) => return Err(ctx::unexpected(&other)),
        Err(e) if is_gone(&e) => match &term.parent {
            Some(parent) => from_parent(ctx, parent, &id),
            None => (String::new(), None),
        },
        Err(e) => return Err(e),
    };
    let text = clean_output(&text);
    let code = code.unwrap_or(1);
    if ctx.json {
        let v = serde_json::json!({"id": id, "timed_out": false, "exit_code": code, "output": text});
        writeln!(out, "{v}")?;
    } else {
        out.write_all(text.as_bytes())?;
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_loses_blank_tail() {
        assert_eq!(clean_output("hi  \n\n   \n"), "hi\n");
        assert_eq!(clean_output("\n\n"), "");
        assert_eq!(clean_output("a\n\nb\n"), "a\n\nb\n");
    }

    #[test]
    fn code_is_read_from_the_end_note() {
        assert_eq!(code_from_note("m2 (sh) ended with code 3"), Some(3));
        assert_eq!(code_from_note("m2 (sh) ended with code -1"), Some(-1));
        assert_eq!(code_from_note("m2 is done"), None);
    }
}
