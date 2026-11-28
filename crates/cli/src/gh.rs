use std::os::unix::process::CommandExt;
use std::path::PathBuf;

const BRAIN_BIN: &str = "nixa-brain";
const MISSING_CODE: i32 = 127;

pub fn brain_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.join(BRAIN_BIN);
    bin.is_file().then_some(bin)
}

pub fn run(argv: &[String]) -> i32 {
    let Some(bin) = brain_bin() else {
        eprintln!("nixa gh gate: {BRAIN_BIN} was not found next to nixa. Nothing was sent to GitHub.");
        return MISSING_CODE;
    };
    let err = std::process::Command::new(&bin).arg("gh").args(argv).exec();
    eprintln!("nixa gh gate: {} did not start: {err}", bin.display());
    MISSING_CODE
}
