mod agents;
mod bridge;
mod gaps;
mod gateway;
mod models;
mod rpc;
mod secrets;
mod state;
mod tasks;
mod term;
mod tools;
mod trees;
mod window;

pub fn run_main(args: Vec<String>) {
    if args.get(1).map(String::as_str) == Some("bridge") {
        bridge::run();
        return;
    }
    if args.get(1).map(String::as_str) == Some("guard") {
        bridge::guard(args.get(2).map(String::as_str), args.get(3).map(String::as_str));
        return;
    }
    let state = state::State::from_env();
    eprintln!(
        "nixa-mcp listening on stdio ({} mode, engine {})",
        state.mode.as_str(),
        state.socket.display()
    );
    rpc::serve(std::io::stdin().lock(), std::io::stdout(), state);
}
