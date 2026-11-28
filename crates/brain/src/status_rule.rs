//! The one rule every prompt Nixa gives an agent carries: report your status, so Nixa chat knows.

/// The nixa command inside a Nixa terminal (the engine sets NIXA_BIN and NIXA_TERM_ID there).
pub const NIXA: &str = "$NIXA_BIN";

/// The rule line for an agent prompt. The note is plain words; `nixa term status` takes it as the words after the status.
pub fn rule() -> String {
    format!(
        "- Tell Nixa your status as you go; Nixa chat shows it to the user. Use {NIXA} term status working \"what you do now\" when you start, {NIXA} term status blocked \"what you wait for\" when you wait for anything, {NIXA} term status done \"Changed: what you changed. Checks: each command you ran and its real result. Not checked: what you did not test.\" when you finish (keep these three parts, short and true; Nixa reviews your work from them), and {NIXA} term status error \"what went wrong\" when something fails. With the nixa-mcp server, its nixa_term_status tool does the same.\n- Nixa tools: run {NIXA} --skill once for the full guide to the nixa command (terminals, helpers, tasks, files, the deep brain).\n- Usage limit: when your usage, weekly or token limit is near or hit, run {NIXA} agent limit --note \"what is done, what is left\" (add --until TIME when the reset time is shown). Nixa moves your task to another agent."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_names_every_status_and_the_own_terminal_command() {
        let line = rule();
        assert!(line.starts_with("- Tell Nixa your status"));
        for word in ["term status working", "term status blocked", "term status done", "Changed:", "Checks:", "Not checked:", "term status error", "$NIXA_BIN", "nixa_term_status", "$NIXA_BIN --skill"] {
            assert!(line.contains(word), "missing {word}");
        }
    }
}
