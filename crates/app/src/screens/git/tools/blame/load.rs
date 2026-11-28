use super::model::{self, Kind, MAX_LINES, Parsed, Problem, problem};
use crate::screens::git::tools::run::{self, Cancel};
use std::path::Path;

pub fn blame_file(
    root: &Path,
    file: &str,
    rev: &str,
    background: bool,
    cancel: Option<&Cancel>,
) -> Result<Parsed, Problem> {
    let path =
        model::clean_path(file).map_err(|e| problem(Kind::Bad, "Cannot show this file", &e))?;
    let rev = model::clean_rev(rev).map_err(|e| problem(Kind::Bad, "Cannot show this file", &e))?;
    let args = model::blame_args(&path, &rev, MAX_LINES);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let wait = if background {
        run::LONG_WAIT
    } else {
        run::READ_WAIT
    };
    let out = run::git_with(root, &args, &[], wait, cancel);
    if out.cancelled {
        return Err(problem(
            Kind::Cancelled,
            "Stopped.",
            "Press Reload to try again.",
        ));
    }
    if out.timed_out {
        let said = crate::screens::git::tools::errors::describe(&out.said());
        return Err(Problem {
            kind: Kind::Timeout,
            title: said.title,
            hint: said.hint,
            detail: said.detail,
        });
    }
    if !out.ok() {
        let text = out.said();
        let text = if text.is_empty() {
            format!("git blame failed ({})", out.code.unwrap_or(-1))
        } else {
            text
        };
        return match model::blame_failure(&text, &rev) {
            Some(p) => Err(p),
            None => Ok(Parsed::default()),
        };
    }
    let parsed = model::parse_blame(&out.stdout, MAX_LINES);
    if model::is_binary(&parsed) {
        return Err(problem(
            Kind::Binary,
            "This is a binary file",
            "Only text files show who changed each line.",
        ));
    }
    Ok(parsed)
}
