use crate::opts::{Cmd, HELP, Opts};
use crate::{check, compile, icon, layout, live, sign};
use std::fs;
use std::path::Path;

pub fn dispatch(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Help => {
            print!("{HELP}");
            Ok(())
        }
        Cmd::Icon { root } => {
            let icns = icon::make(&root)?;
            println!("Made {}", icns.display());
            Ok(())
        }
        Cmd::Check { app, root, live, licenses } => checks(&app, &root, live, licenses),
        Cmd::Bundle(opts) => bundle(&opts),
        Cmd::Soak(args) => crate::soak::run(&args),
        #[cfg(target_os = "macos")]
        Cmd::Release(args) => crate::release::run(&args),
        #[cfg(not(target_os = "macos"))]
        Cmd::Release(_) => Err("release activation is available only on macOS".into()),
    }
}

pub fn checks(app: &Path, root: &Path, live_check: bool, licenses: bool) -> Result<(), String> {
    println!("Checking {}", app.display());
    if !licenses {
        println!("  license file check skipped (--allow-missing-licenses)");
    }
    let mut problems = check::static_checks(app, root, licenses)?;
    match sign::verify(app) {
        Ok(text) => println!("  signature: {text}"),
        Err(why) => problems.push(format!("the signature does not verify: {why}")),
    }
    if live_check {
        let found = live::check(app)?;
        if found.is_empty() {
            println!("  phone door and brain: no page route answers");
        }
        problems.extend(found);
    } else {
        println!("  live page route check skipped (--no-live-check)");
    }
    if problems.is_empty() {
        println!("  programs, Info.plist, icon, hooks{}: ok", if licenses { ", license files" } else { "" });
        println!("  no phone page files, no phone page text, no build folder paths, only macOS libraries");
        return Ok(());
    }
    for problem in &problems {
        println!("  problem: {problem}");
    }
    Err(format!("{} problem(s) in {}", problems.len(), app.display()))
}

fn bundle(opts: &Opts) -> Result<(), String> {
    let identity = opts.sign.clone().unwrap_or_else(sign::default_identity);
    if identity.is_empty() {
        return Err("--sign needs an identity, or - for ad hoc".to_string());
    }
    let missing = layout::license_gate(&opts.root, opts.licenses)?;
    if !missing.is_empty() {
        println!("Warning: this test bundle has no {}", missing.join(", "));
    }
    let release = if opts.build { compile::build(opts)? } else { compile::release_dir(&opts.target_dir) };
    fs::create_dir_all(&opts.out).map_err(|e| format!("cannot make {}: {e}", opts.out.display()))?;
    let stage = layout::stage_path(&opts.out);
    let licenses = layout::assemble(&opts.root, &release, &stage, env!("CARGO_PKG_VERSION"))?;
    if !licenses.is_empty() {
        println!("Copied {} into Contents/Resources", licenses.join(", "));
    }
    println!("Signing {} with identity {}", stage.display(), identity);
    sign::sign(&stage, &identity)?;
    checks(&stage, &opts.root, opts.live, opts.licenses).map_err(|why| format!("{why}. The unfinished bundle is left in {}", stage.display()))?;
    let app = layout::replace(&stage, &opts.out)?;
    println!("Made {}", app.display());
    Ok(())
}
