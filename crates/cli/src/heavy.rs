use nixa_protocol::heavy::{self, Job};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const HELP: &str = "nixa heavy: take turns for builds and tests on this Mac

Usage:
  nixa heavy                     show the slots, what runs in them and who waits
  nixa heavy status [--json]     the same
  nixa heavy on | off            turn slots on or off
  nixa heavy slots N             how many heavy runs at the same time (1 to 16, default 2)
  nixa heavy [--] COMMAND ...    run any command in a slot

In every Nixa terminal, cargo, xcodebuild, gradle, swift, go, npm, pnpm, yarn and bun
go through Nixa first. Builds and tests wait for a free slot. Other commands run at once.
A command run inside a slot never waits again, so nested builds cannot block.
";

const WAIT_STEP: Duration = Duration::from_millis(500);
const MISSING_CODE: i32 = 127;
const START_FAILED_CODE: i32 = 126;

pub fn run(argv: &[String]) -> i32 {
    let data = nixa_protocol::paths::data_dir();
    match argv.first().map(String::as_str) {
        None | Some("status") => show(&data, argv.iter().any(|a| a == "--json")),
        Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            0
        }
        Some("on") => set(&data, |s| s.on = true),
        Some("off") => set(&data, |s| s.on = false),
        Some("slots") => match argv.get(1).and_then(|n| n.parse::<usize>().ok()) {
            Some(n) if (1..=heavy::MAX_SLOTS).contains(&n) => set(&data, |s| s.slots = n),
            _ => {
                eprintln!("nixa heavy slots: give a number from 1 to {}.", heavy::MAX_SLOTS);
                2
            }
        },
        Some("--tool") => {
            let Some(tool) = argv.get(1) else {
                eprintln!("nixa heavy --tool: give the tool name.");
                return 2;
            };
            if !heavy::TOOLS.contains(&tool.as_str()) {
                eprintln!("nixa heavy --tool: {tool} is not a known tool. Known: {}.", heavy::TOOLS.join(", "));
                return 2;
            }
            let rest = after_dashes(&argv[2..]);
            let heavy_now = std::env::var_os(heavy::HELD_ENV).is_none() && heavy::is_heavy(tool, rest);
            let Some(real) = real_tool(&data, tool) else {
                eprintln!("{}: {tool} was not found on PATH.", heavy::MARK);
                return MISSING_CODE;
            };
            if heavy_now { in_slot(&data, &real, tool, rest) } else { exec(&real, rest) }
        }
        Some(_) => {
            let rest = after_dashes(argv);
            let Some((cmd, args)) = rest.split_first() else {
                eprintln!("nixa heavy: give a command to run.");
                return 2;
            };
            let real = if cmd.contains('/') { Some(PathBuf::from(cmd)) } else { real_tool(&data, cmd) };
            let Some(real) = real else {
                eprintln!("nixa heavy: {cmd} was not found on PATH.");
                return MISSING_CODE;
            };
            if std::env::var_os(heavy::HELD_ENV).is_some() { exec(&real, args) } else { in_slot(&data, &real, cmd, args) }
        }
    }
}

fn after_dashes(argv: &[String]) -> &[String] {
    match argv.first() {
        Some(first) if first == "--" => &argv[1..],
        _ => argv,
    }
}

fn real_tool(data: &Path, tool: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    let skip: Vec<PathBuf> = [std::env::current_exe().ok(), Some(heavy::bin_dir(data).join(tool))].into_iter().flatten().collect();
    let found = heavy::find_real(tool, &path, &skip)?;
    let helpers = heavy::bin_dir(data);
    (found.parent() != Some(helpers.as_path())).then_some(found)
}

fn set(data: &Path, change: impl FnOnce(&mut heavy::Settings)) -> i32 {
    let mut s = heavy::load(data);
    change(&mut s);
    match heavy::save(data, &s) {
        Ok(s) => {
            println!("Heavy slots: {}, {} at once.", if s.on { "on" } else { "off" }, s.slots);
            0
        }
        Err(e) => {
            eprintln!("nixa heavy: {e}");
            1
        }
    }
}

fn line(job: &Job) -> String {
    let secs = heavy::now_ms().saturating_sub(job.since) / 1000;
    let place = if job.terminal.is_empty() { String::new() } else { format!(" in {}", job.terminal) };
    format!("{}{place}, {} s, {}", job.command, secs, job.folder)
}

fn show(data: &Path, json: bool) -> i32 {
    if json {
        println!("{}", heavy::status(data));
        return 0;
    }
    let s = heavy::load(data);
    println!("Heavy slots: {}, {} at once. Helpers {}.", if s.on { "on" } else { "off" }, s.slots, if heavy::ready(data) { "ready" } else { "not written yet" });
    let busy = heavy::busy(data);
    if busy.is_empty() {
        println!("No heavy run now.");
    }
    for job in &busy {
        println!("  slot {}: {}", job.slot, line(job));
    }
    for (n, job) in heavy::waiting(data).iter().enumerate() {
        println!("  waiting {}: {}", n + 1, line(job));
    }
    0
}

fn exec(real: &Path, args: &[String]) -> i32 {
    let err = std::process::Command::new(real).args(args).exec();
    eprintln!("nixa heavy: {} did not start: {err}", real.display());
    START_FAILED_CODE
}

fn job_of(name: &str, args: &[String]) -> Job {
    let mut command = nixa_store::text::hide_secrets(&std::iter::once(name.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" "), &[]);
    if command.chars().count() > 160 {
        command = command.chars().take(160).collect::<String>() + "…";
    }
    Job {
        pid: std::process::id(),
        command,
        folder: std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default(),
        terminal: std::env::var(nixa_protocol::paths::ENV_TERM_ID).unwrap_or_default(),
        since: heavy::now_ms(),
        slot: 0,
    }
}

fn wait_for_slot(data: &Path, job: &Job) -> Option<PathBuf> {
    let queue = heavy::join_queue(data, job);
    let mut told = false;
    let slot = loop {
        let s = heavy::load(data);
        if !s.on {
            break None;
        }
        if heavy::my_turn(data, s.slots, job.pid)
            && let Some(slot) = heavy::take(data, s.slots, job)
        {
            break Some(slot);
        }
        if !told {
            let busy: Vec<String> = heavy::busy(data).iter().map(|j| j.command.clone()).filter(|c| !c.is_empty()).collect();
            let now = if busy.is_empty() { String::new() } else { format!(" ({})", busy.join("; ")) };
            eprintln!("{}: all {} slots are busy{now}. Waiting for a free slot. Ctrl-C stops waiting.", heavy::MARK, s.slots);
            told = true;
        }
        std::thread::sleep(WAIT_STEP);
    };
    if let Some(file) = queue {
        let _ = std::fs::remove_file(file);
    }
    if told && slot.is_some() {
        eprintln!("{}: got a slot, starting {}.", heavy::MARK, job.command);
    }
    slot
}

fn in_slot(data: &Path, real: &Path, name: &str, args: &[String]) -> i32 {
    let job = job_of(name, args);
    let Some(slot) = wait_for_slot(data, &job) else { return exec(real, args) };
    heavy::ignore_stop_keys();
    let status = std::process::Command::new(real).args(args).env(heavy::HELD_ENV, "1").status();
    heavy::release(&slot);
    match status {
        Ok(s) => s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(1)),
        Err(e) => {
            eprintln!("nixa heavy: {} did not start: {e}", real.display());
            START_FAILED_CODE
        }
    }
}
