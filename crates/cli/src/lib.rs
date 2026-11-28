mod agent;
mod app;
mod args;
mod brain;
mod chat;
mod connector;
mod ctx;
mod decide;
mod decision;
mod deep;
mod do_cmd;
mod drones;
mod tool;
mod engine;
mod exec;
mod file;
mod gh;
mod heavy;
mod help;
mod hooks;
mod http;
mod models;
mod office;
mod phone;
mod privacy;
mod rules;
mod settings;
mod show;
mod skill;
mod space;
mod task;
mod term;
mod voice;

use args::UsageError;
use ctx::Ctx;
use nixa_protocol::{EngineError, Reply};
use std::io::{self, Write};

pub fn run_main(argv: Vec<String>) -> ! {
    if argv.first().is_some_and(|a| a == "gh") {
        std::process::exit(gh::run(&argv[1..]));
    }
    if argv.first().is_some_and(|a| a == "heavy") {
        std::process::exit(heavy::run(&argv[1..]));
    }
    if argv.first().is_some_and(|a| a == "connector") {
        std::process::exit(connector::run(&argv[1..]));
    }
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let code = run(&argv, &mut out);
    let _ = out.flush();
    std::process::exit(code);
}

fn run(argv: &[String], out: &mut dyn Write) -> i32 {
    let json = argv.iter().take_while(|a| *a != "--").any(|a| a == "--json");
    let result = ctx::globals(argv).and_then(|(ctx, rest)| dispatch(&ctx, rest, out));
    match result {
        Ok(code) => code,
        Err(err) => report(json, err, out),
    }
}

fn dispatch(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> anyhow::Result<i32> {
    let Some((cmd, rest)) = argv.split_first() else {
        out.write_all(help::MAIN.as_bytes())?;
        return Ok(2);
    };
    match cmd.as_str() {
        "term" | "t" => term::run(ctx, rest, out),
        "app" => app::run(ctx, rest, out),
        "do" => do_cmd::run(ctx, rest, out),
        "engine" => engine::run(ctx, rest, out),
        "task" => task::run(ctx, rest, out),
        "space" => space::run(ctx, rest, out),
        "deep" => deep::run(ctx, rest, out),
        "drones" => drones::run(ctx, rest, out),
        "agent" | "agents" => agent::run(ctx, rest, out),
        "tool" | "tools" => tool::run(ctx, rest, out),
        "screenshot" | "shot" => tool::screenshot(ctx, rest, out),
        "flow" | "flows" | "flowman" => tool::flow(ctx, rest, out),
        "decide" => decide::run(ctx, rest, out),
        "decision" => decision::run(ctx, rest, out),
        "brain" => brain::run(ctx, rest, out),
        "hooks" => hooks::run(ctx, rest, out),
        "voice" => voice::run(ctx, rest, out),
        "phone" => phone::run(ctx, rest, out),
        "exec" => exec::run(ctx, rest, out),
        "file" => file::run(ctx, rest, out),
        "privacy" => privacy::run(ctx, rest, out),
        "office" => office::run(ctx, rest, out),
        "settings" => settings::run(ctx, rest, out),
        "rules" | "rule" => rules::run(ctx, rest, out),
        "models" => models::run(ctx, rest, out),
        "chat" => chat::run(ctx, rest, out),
        "help" | "-h" | "--help" => {
            help::show(rest.first().map(String::as_str), out)?;
            Ok(0)
        }
        "skill" | "--skill" => {
            out.write_all(skill::GUIDE.as_bytes())?;
            Ok(0)
        }
        "version" | "-V" | "--version" => {
            writeln!(
                out,
                "nixa {} (protocol {})",
                env!("CARGO_PKG_VERSION"),
                nixa_protocol::PROTOCOL_VERSION
            )?;
            Ok(0)
        }
        other => Err(args::usage(format!("unknown command {other:?}"))),
    }
}

fn report(json: bool, err: anyhow::Error, out: &mut dyn Write) -> i32 {
    if is_broken_pipe(&err) {
        return 0;
    }
    if let Some(u) = err.downcast_ref::<UsageError>() {
        eprintln!("nixa: {u}");
        eprintln!("Run: nixa help");
        return 2;
    }
    if let Some(e) = err.downcast_ref::<EngineError>() {
        if json {
            let reply = Reply::error(e.code, e.message.clone());
            let _ = writeln!(out, "{}", serde_json::to_string(&reply).unwrap_or_default());
        } else {
            eprintln!("nixa: {}", e.message);
        }
        return 1;
    }
    eprintln!("nixa: {err:#}");
    1
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|c| {
        c.downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    })
}
