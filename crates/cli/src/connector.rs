use std::os::unix::process::CommandExt;

const MISSING_CODE: i32 = 127;

pub fn run(argv: &[String]) -> i32 {
    let Some(bin) = crate::gh::brain_bin() else {
        eprintln!("nixa connector: nixa-brain was not found next to nixa.");
        return MISSING_CODE;
    };
    let err = std::process::Command::new(&bin).arg("connector").args(argv).exec();
    eprintln!("nixa connector: {} did not start: {err}", bin.display());
    MISSING_CODE
}
