use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nixa-cli-space-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

fn nixa(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nixa"))
        .args(args)
        .env("NIXA_DATA_DIR", data)
        .env("NIXA_ENGINE_SOCKET", data.join("none.sock"))
        .env_remove("NIXA_TERM_ID")
        .env_remove("NIXA_BRAIN_ADDR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

#[test]
fn space_add_then_list() {
    let root = temp_dir("run");
    let data = root.join("data");
    let shop = root.join("shop");
    std::fs::create_dir_all(&shop).unwrap();
    let folder = shop.to_string_lossy().to_string();

    let empty = nixa(&data, &["space", "ls"]);
    assert_eq!(empty.status.code(), Some(0));
    assert!(out(&empty).starts_with("No spaces yet."));

    let added = nixa(&data, &["space", "add", &folder]);
    assert_eq!(added.status.code(), Some(0), "{}", String::from_utf8_lossy(&added.stderr));
    assert_eq!(out(&added).trim(), format!("Added space s1  shop  {folder}."));

    let again = nixa(&data, &["--json", "space", "add", &folder, "--name", "other"]);
    let reply: Value = serde_json::from_str(&out(&again)).unwrap();
    assert_eq!(reply["added"], false);
    assert_eq!(reply["space"]["name"], "shop");

    let listed = nixa(&data, &["space", "list"]);
    assert_eq!(out(&listed).trim(), format!("s1  shop  {folder}  (active)"));
    let json: Value = serde_json::from_str(&out(&nixa(&data, &["space", "ls", "--json"]))).unwrap();
    assert_eq!(json["active"], "s1");
    assert_eq!(json["spaces"].as_array().unwrap().len(), 1);

    let missing = nixa(&data, &["space", "add", &root.join("gone").to_string_lossy()]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(nixa(&data, &["space", "add"]).status.code(), Some(2));
    assert_eq!(nixa(&data, &["space", "zap"]).status.code(), Some(2));
    std::fs::remove_dir_all(&root).ok();
}
