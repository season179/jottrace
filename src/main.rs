use std::env;
use std::io::{self, Write};
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
        Some("ingest") => run_ingest_command(),
        Some("status") => run_status_command(),
        Some("web") => run_web_command(args),
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `jottrace --help` for usage");
            // A distinct usage-error code lets scripts tell "bad command" apart
            // from runtime failures such as permission or filesystem errors.
            ExitCode::from(2)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WebOptions {
    port: u16,
    once: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebCommand {
    Run(WebOptions),
    Help,
}

fn run_web_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_web_command(args) {
        Ok(WebCommand::Run(options)) => options,
        Ok(WebCommand::Help) => {
            print_web_help();
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}");
            eprintln!("run `jottrace web --help` for usage");
            return ExitCode::from(2);
        }
    };

    let db_path = match jottrace::storage::db_path_from_env() {
        Ok(db_path) => db_path,
        Err(error) => {
            eprintln!("jottrace web failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    let server = match jottrace::web::WebServer::bind(db_path.clone(), options.port) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("jottrace web failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("jottrace web");
    println!("url: {}", server.local_url());
    println!("db: {}", db_path.display());
    let _ = io::stdout().flush();

    let result = if options.once {
        server.serve_once()
    } else {
        server.serve_forever()
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jottrace web failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_web_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<WebCommand, String> {
    let mut options = WebOptions {
        port: 0,
        once: false,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(WebCommand::Help),
            "--once" => options.once = true,
            "--port" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--port requires a value".to_string())?;
                options.port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            _ => return Err(format!("unknown web option: {arg}")),
        }
    }

    Ok(WebCommand::Run(options))
}

fn run_ingest_command() -> ExitCode {
    match jottrace::run_ingest() {
        Ok(report) => {
            println!("jottrace ingest");
            println!("db: {}", report.db_path.display());
            println!("files: {}", report.file_count);
            println!("sessions: {}", report.session_count);
            println!("events: {}", report.event_count);
            println!("inserted_events: {}", report.inserted_event_count);
            println!(
                "unresolved_ingest_errors: {}",
                report.unresolved_ingest_error_count
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jottrace ingest failed: {error}");
            ExitCode::FAILURE
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
            println!(
                "unresolved_ingest_errors: {}",
                report.unresolved_ingest_error_count
            );
            if !report.recent_ingest_errors.is_empty() {
                println!(
                    "recent_ingest_errors: {}",
                    report.recent_ingest_errors.len()
                );
            }
            for ingest_error in &report.recent_ingest_errors {
                println!("recent_ingest_error:");
                println!("  source: {}", ingest_error.source);
                if let Some(source_session_id) = &ingest_error.source_session_id {
                    println!("  source_session_id: {source_session_id}");
                }
                println!("  file: {}", ingest_error.file_path.display());
                if let Some(line_number) = ingest_error.line_number {
                    println!("  line: {line_number}");
                }
                if let Some(byte_offset) = ingest_error.byte_offset {
                    println!("  byte_offset: {byte_offset}");
                }
                if let Some(generation) = ingest_error.generation {
                    println!("  generation: {generation}");
                }
                println!("  kind: {}", ingest_error.error_kind);
                println!("  first_seen_at: {}", ingest_error.first_seen_at);
                println!("  last_seen_at: {}", ingest_error.last_seen_at);
                println!("  occurrences: {}", ingest_error.occurrence_count);
                println!("  message: {}", ingest_error.message);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jottrace doctor failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_status_command() -> ExitCode {
    match jottrace::run_status() {
        Ok(report) => {
            println!("jottrace status");
            println!("db: {}", report.db_path.display());
            println!("schema_version: {}", report.schema_version);
            println!("sessions: {}", report.session_count);
            println!("events: {}", report.event_count);
            println!(
                "unresolved_ingest_errors: {}",
                report.unresolved_ingest_error_count
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jottrace status failed: {error}");
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
    println!("  jottrace ingest");
    println!("  jottrace status");
    println!("  jottrace web [--port <port>] [--once]");
    println!("  jottrace --version");
}

fn print_web_help() {
    println!("jottrace web");
    println!("Start a read-only local web UI for the preserved journal.");
    println!();
    println!("Usage:");
    println!("  jottrace web [--port <port>] [--once]");
    println!();
    println!("Options:");
    println!("  --port <port>  Bind to a fixed localhost port instead of an available port");
    println!("  --once         Serve one request and exit, useful for smoke tests");
}
