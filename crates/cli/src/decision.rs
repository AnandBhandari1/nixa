use crate::args::usage;
use crate::ctx::Ctx;
use crate::{help, http};
use anyhow::Result;
use serde_json::json;
use std::io::Write;

pub const ALLOWED: [&str; 5] = ["--sample", "--kind", "--capability", "--held", "--bench"];

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::DECISION.as_bytes())?;
        return Ok(2);
    };
    match sub.as_str() {
        "help" | "-h" | "--help" => {
            out.write_all(help::DECISION.as_bytes())?;
            Ok(0)
        }
        "check" => check(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa decision {other}"))),
    }
}

pub fn check_args(rest: &[String]) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "--json" {
            continue;
        }
        if !ALLOWED.contains(&arg.as_str()) {
            return Err(usage(format!("nixa decision check does not know {arg}")));
        }
        args.push(arg.clone());
        if matches!(arg.as_str(), "--sample" | "--kind" | "--capability") {
            let value = iter.next().ok_or_else(|| usage(format!("{arg} needs a value")))?;
            args.push(value.clone());
        }
    }
    Ok(args)
}

fn check(ctx: &Ctx, rest: &[String], out: &mut dyn Write) -> Result<i32> {
    let args = check_args(rest)?;
    let reply = http::post("/decision/check", &json!({"args": args}))?;
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        writeln!(out, "{}", serde_json::to_string_pretty(&reply["report"])?)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_takes_only_the_safe_flags() {
        let words = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(check_args(&words("--sample hello --held")).unwrap(), words("--sample hello --held"));
        assert!(check_args(&words("--rules /tmp/x.json")).is_err());
        assert!(check_args(&words("--kind")).is_err());
        assert!(check_args(&[]).unwrap().is_empty());
    }
}
