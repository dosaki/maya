pub mod args;
pub mod commands;
mod executor;
mod notify;
pub mod run_cmd;
pub mod status_file;
pub mod tmux;

pub fn run(args: Vec<String>) -> i32 {
    let claude_dir = maya_core::claude_dir();
    match args::parse(&args) {
        Err(usage) => {
            eprintln!("{usage}");
            2
        }
        Ok(args::Cmd::Help) => {
            println!("{}", args::USAGE);
            0
        }
        Ok(args::Cmd::Version) => {
            println!("maya {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(args::Cmd::Pair { host, port, code, name }) => report(commands::pair(&claude_dir, &host, port, name.as_deref(), &code).map(|m| format!("Paired with {m}"))),
        Ok(args::Cmd::Status) => report(commands::status(&claude_dir)),
        Ok(args::Cmd::Hooks(op)) => {
            if matches!(op, args::HooksOp::Install | args::HooksOp::Status) && !commands::jq_found() {
                eprintln!("warning: {}", commands::NO_JQ);
            }
            report(commands::hooks(&claude_dir, op))
        }
        Ok(args::Cmd::Config(args::ConfigOp::ProjectsDir(dir))) => report(commands::set_projects_dir(&claude_dir, &dir)),
        Ok(args::Cmd::Run) => run_cmd::run(&claude_dir),
        Ok(args::Cmd::Start { dir, prompt, options }) => report(commands::start(&claude_dir, dir, prompt, options)),
    }
}

fn report(r: Result<String, String>) -> i32 {
    match r {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
