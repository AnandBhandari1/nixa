use std::process::Command;

fn nixa(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_nixa")).args(args).env_remove("NIXA_BRAIN_ADDR").output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn help_names_the_connector_command() {
    let (code, text, _) = nixa(&["help", "connector"]);
    assert_eq!(code, 0);
    assert!(text.starts_with("nixa connector: outside tools"));
    assert!(text.contains("secret ENTRY --remove"));
    let (_, main, _) = nixa(&["help"]);
    assert!(main.contains("connector outside tools for the brain"));
}
