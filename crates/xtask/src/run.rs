use std::process::{Command, Output};

pub fn show(cmd: &Command) -> String {
    let mut text = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}

pub fn status(cmd: &mut Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("cannot start {}: {e}", show(cmd)))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({status})", show(cmd)))
    }
}

pub fn output(cmd: &mut Command) -> Result<Output, String> {
    cmd.output().map_err(|e| format!("cannot start {}: {e}", show(cmd)))
}

pub fn text(cmd: &mut Command) -> Result<String, String> {
    let out = output(cmd)?;
    let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
    all.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(all)
    } else {
        Err(format!("{} failed ({}): {}", show(cmd), out.status, all.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_joins_both_streams_and_reports_failure() {
        let both = text(Command::new("/bin/sh").args(["-c", "echo out; echo err >&2"])).unwrap();
        assert_eq!(both, "out\nerr\n");
        let bad = text(Command::new("/bin/sh").args(["-c", "echo nope; exit 3"])).unwrap_err();
        assert!(bad.contains("/bin/sh -c echo nope; exit 3 failed"), "{bad}");
        assert!(bad.ends_with("nope"), "{bad}");
        assert!(status(&mut Command::new("/nonexistent/xtask-tool")).unwrap_err().starts_with("cannot start"));
    }
}
