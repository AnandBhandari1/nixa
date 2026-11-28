mod agent_hooks;
mod counter;
mod cwd;
mod draft;
mod engine;
mod handlers;
mod output;
mod procs;
mod recover;
mod screen;
mod server;
mod terminal;
mod terminal_ids;
mod wait;
mod writer;

pub use server::{Running, run, start, stop};

const USAGE: &str = "usage: nixa-engine [--socket <path>]";

fn parse_args(args: Vec<String>) -> std::path::PathBuf {
    let mut args = args.into_iter();
    let mut socket = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => match args.next() {
                Some(p) if !p.is_empty() => socket = Some(std::path::PathBuf::from(p)),
                _ => {
                    eprintln!("{USAGE}");
                    std::process::exit(2);
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("nixa-engine {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument {other}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let socket = socket.unwrap_or_else(nixa_protocol::paths::socket_path);
    std::path::absolute(&socket).unwrap_or(socket)
}

pub fn run_main(args: Vec<String>) {
    let socket = parse_args(args);
    if let Err(e) = run(socket) {
        eprintln!("nixa-engine: {e:#}");
        std::process::exit(1);
    }
}
