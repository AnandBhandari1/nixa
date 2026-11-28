mod bundle;
mod check;
mod compile;
mod icon;
mod layout;
mod live;
mod opts;
mod plist;
#[cfg(target_os = "macos")]
mod release;
#[cfg(target_os = "macos")]
mod release_files;
#[cfg(target_os = "macos")]
mod release_runtime;
mod run;
mod sign;
mod soak;
#[cfg(test)]
mod testkit;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match opts::parse(&args).and_then(bundle::dispatch) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("xtask: {why}");
            ExitCode::FAILURE
        }
    }
}
