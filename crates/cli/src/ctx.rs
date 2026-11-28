use crate::args::{self, Parsed};
use anyhow::{Result, anyhow};
use nixa_protocol::{Client, Reply, Request, paths};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

pub const CALL_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct Ctx {
    pub socket: PathBuf,
    pub json: bool,
}

pub fn globals(argv: &[String]) -> Result<(Ctx, &[String])> {
    let mut json = false;
    let mut socket = None;
    let mut i = 0;
    while let Some(a) = argv.get(i) {
        if a == "--json" {
            json = true;
            i += 1;
        } else if a == "--socket" {
            let v = argv
                .get(i + 1)
                .ok_or_else(|| args::usage("--socket needs a value"))?;
            socket = Some(PathBuf::from(v));
            i += 2;
        } else if let Some(v) = a.strip_prefix("--socket=") {
            socket = Some(PathBuf::from(v));
            i += 1;
        } else {
            break;
        }
    }
    let socket = socket.unwrap_or_else(paths::socket_path);
    Ok((Ctx { socket, json }, &argv[i..]))
}

pub fn command(
    ctx: &Ctx,
    argv: &[String],
    with_value: &[&str],
    stop_at_pos: bool,
) -> Result<(Parsed, Ctx)> {
    let mut names = with_value.to_vec();
    names.push("--socket");
    let mut p = args::parse(argv, &names, stop_at_pos)?;
    let json = p.flag(&["--json"]) || ctx.json;
    let socket = p
        .value(&["--socket"])
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.socket.clone());
    Ok((p, Ctx { socket, json }))
}

pub fn connect(ctx: &Ctx, timeout: Option<Duration>) -> Result<Client> {
    let client = nixa_protocol::connect(&ctx.socket)
        .map_err(|e| anyhow!("{e:#}\nStart it with: nixa engine start"))?;
    client.set_timeout(timeout)?;
    Ok(client)
}

pub fn call(ctx: &Ctx, req: &Request) -> Result<Reply> {
    call_with(ctx, req, Some(CALL_TIMEOUT))
}

pub fn call_with(ctx: &Ctx, req: &Request, timeout: Option<Duration>) -> Result<Reply> {
    connect(ctx, timeout)?.call(req)
}

pub fn print_json(out: &mut dyn Write, reply: &Reply) -> Result<i32> {
    writeln!(out, "{}", serde_json::to_string(reply)?)?;
    Ok(0)
}

pub fn unexpected(reply: &Reply) -> anyhow::Error {
    anyhow!(
        "unexpected reply from nixa-engine: {}",
        serde_json::to_string(reply).unwrap_or_default()
    )
}
