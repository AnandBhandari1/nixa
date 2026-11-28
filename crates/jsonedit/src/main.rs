fn main() -> std::process::ExitCode {
    nixa_jsonedit::run_main(std::env::args().skip(1).collect())
}
