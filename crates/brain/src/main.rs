fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "gh") {
        std::process::exit(nixa_brain::git_tools::gh_gate::gate_main(&args[1..]));
    }
    if args.first().is_some_and(|a| a == "connector") {
        std::process::exit(nixa_brain::connectors::cli::main(&args[1..]));
    }
    nixa_brain::log_file::watch(&nixa_protocol::paths::data_dir());
    tokio::runtime::Runtime::new()?.block_on(nixa_brain::run())
}
