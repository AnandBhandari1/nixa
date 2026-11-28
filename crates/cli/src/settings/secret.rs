use anyhow::{Result, bail};
use std::io::{self, BufRead, IsTerminal};
use std::process::{Command, Stdio};

fn echo(on: bool) {
    let _ = Command::new("stty")
        .arg(if on { "echo" } else { "-echo" })
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .status();
}

pub fn read_hidden(prompt: &str) -> Result<String> {
    let typed = io::stdin().is_terminal();
    if typed {
        eprint!("{prompt}");
        echo(false);
    }
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line);
    if typed {
        echo(true);
        eprintln!();
    }
    read?;
    let key = line.trim().to_string();
    if key.is_empty() {
        bail!("No key was given. Nothing was saved.");
    }
    Ok(key)
}
