use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    // The first env arg is the executable path; commands start after it.
    let mut args = env::args().skip(1);

    // Match on &str values so command dispatch stays simple and avoids cloning
    // the user-provided argument just to inspect it.
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
            // A distinct usage-error code lets scripts tell "bad command" apart
            // from runtime failures such as permission or filesystem errors.
            ExitCode::from(2)
        }
    }
}

fn run_doctor_command() -> ExitCode {
    // Keep the CLI responsible for presentation while the library owns the
    // filesystem checks. That makes future commands easier to test directly.
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
    // Cargo exposes package metadata at compile time, so --version and help
    // cannot drift away from Cargo.toml.
    println!("jottrace {}", env!("CARGO_PKG_VERSION"));
    println!("Preserve AI coding-session transcripts into a local journal.");
    println!();
    println!("Usage:");
    println!("  jottrace doctor");
    println!("  jottrace --version");
}
