#[cfg(unix)]
fn main() {
    std::process::exit(maya_cli::run(std::env::args().skip(1).collect()))
}

#[cfg(not(unix))]
fn main() {
    eprintln!("maya-cli drives sessions through tmux and runs on Linux and macOS; on Windows, run it inside WSL.");
    std::process::exit(2)
}
