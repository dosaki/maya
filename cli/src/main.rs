fn main() {
    std::process::exit(maya_cli::run(std::env::args().skip(1).collect()))
}
