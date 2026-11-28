use super::files::{combined, snapshot, tree_files, FILE, TREES_DIR};
use super::rules::{Input, compile, rules_of, seed};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const SAMPLE: &str = "Nixa discuss the login fix, cheap, test first";

fn value_of(args: &[String], flag: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|a| a == flag) else { return Ok(None) };
    match args.get(at + 1) {
        Some(v) if !v.starts_with("--") => Ok(Some(v.clone())),
        _ => Err(format!("{flag} needs a value.")),
    }
}

pub fn report(args: &[String], dir: &Path) -> Result<Value, String> {
    let file = value_of(args, "--rules")?.map(PathBuf::from).unwrap_or_else(|| dir.join(FILE));
    let trees_dir = match value_of(args, "--trees-dir")? {
        Some(d) => PathBuf::from(d),
        None => file.parent().map(|p| p.join(TREES_DIR)).unwrap_or_else(|| PathBuf::from(TREES_DIR)),
    };
    if args.iter().any(|a| a == "--rules") && !file.exists() {
        return Err(format!("No rules file {}.", file.display()));
    }
    let (source, files, rules) = if file.exists() {
        let dir = trees_dir.exists().then_some(trees_dir.as_path());
        let snap = snapshot(&file, dir)?;
        let files: Vec<String> = dir.map(tree_files).unwrap_or_default().iter().map(|p| p.display().to_string()).collect();
        (file.display().to_string(), files, rules_of(&combined(&snap)?)?)
    } else {
        ("built-in rules".to_string(), Vec::new(), seed())
    };
    let engine = compile(&rules)?;
    let sample = value_of(args, "--sample")?.unwrap_or_else(|| SAMPLE.to_string());
    let held = args.iter().any(|a| a == "--held");
    let input = Input { held, kind: value_of(args, "--kind")?, capability: value_of(args, "--capability")?, ..Input::default() };
    let decision = engine.decide(&sample, &input)?;
    let mut out = json!({"source": source, "treeFiles": files});
    if let (Some(map), Value::Object(check)) = (out.as_object_mut(), serde_json::to_value(engine.self_check()).unwrap_or(Value::Null)) {
        map.extend(check);
        map.insert("sample".into(), json!(sample));
        map.insert("decision".into(), serde_json::to_value(&decision).unwrap_or(Value::Null));
    }
    if args.iter().any(|a| a == "--bench") {
        let quick = Input { held, ..Input::default() };
        for _ in 0..1000 {
            let _ = engine.decide(&sample, &quick);
        }
        let start = Instant::now();
        let rounds = 10_000u32;
        for _ in 0..rounds {
            let _ = engine.decide(&sample, &quick);
        }
        let micros = start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(rounds);
        if let Some(map) = out.as_object_mut() {
            map.insert("averageMicroseconds".into(), json!((micros * 100.0).round() / 100.0));
            map.insert("iterations".into(), json!(rounds));
        }
    }
    Ok(out)
}
