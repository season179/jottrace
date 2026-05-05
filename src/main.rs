use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        None | Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("--version") | Some("-V") => {
            println!("jottrace {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("doctor") => run_doctor_command(),
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `jottrace --help` for usage");
            ExitCode::from(2)
        }
    }
}

fn run_doctor_command() -> ExitCode {
    match jottrace::run_doctor() {
        Ok(report) => {
            println!("jottrace doctor");
            println!("data_dir: {} (ok)", report.data_dir.display());
            println!("permissions: private (ok)");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jottrace doctor failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("jottrace {}", env!("CARGO_PKG_VERSION"));
    println!("Preserve AI coding-session transcripts into a local journal.");
    println!();
    println!("Usage:");
    println!("  jottrace doctor");
    println!("  jottrace --version");
}
