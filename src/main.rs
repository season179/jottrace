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
        Some("compact") => run_compact_command(args),
        Some("doctor") => run_doctor_command(),
        Some("events") => run_events_command(args),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventsOptions {
    source: String,
    source_session_id: String,
    selection: EventsSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EventsSelection {
    All,
    Limit(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EventsCommand {
    Run(EventsOptions),
    Help,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactCommand {
    Run(jottrace::CompactOptions),
    Help,
}

fn run_events_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_events_command(args) {
        Ok(EventsCommand::Run(options)) => options,
        Ok(EventsCommand::Help) => {
            print_events_help();
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}");
            eprintln!("run `jottrace events --help` for usage");
            return ExitCode::from(2);
        }
    };

    let db_path = match jottrace::storage::db_path_from_env() {
        Ok(db_path) => db_path,
        Err(error) => {
            eprintln!("jottrace events failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let limit = match options.selection {
        EventsSelection::All => None,
        EventsSelection::Limit(limit) => Some(limit),
    };
    let stdout = io::stdout();
    let mut stdout = io::BufWriter::new(stdout.lock());
    let result = jottrace::storage::for_each_decoded_event_payload_for_session(
        &db_path,
        &options.source,
        &options.source_session_id,
        limit,
        |payload| {
            stdout
                .write_all(payload)
                .and_then(|()| stdout.write_all(b"\n"))
                .map_err(|source| jottrace::JottraceError::Output { source })
        },
    );

    let result = result.and_then(|()| {
        stdout
            .flush()
            .map_err(|source| jottrace::JottraceError::Output { source })
    });
    if let Err(error) = result {
        if matches!(
            &error,
            jottrace::JottraceError::Output { source }
                if source.kind() == io::ErrorKind::BrokenPipe
        ) {
            return ExitCode::SUCCESS;
        }
        eprintln!("jottrace events failed: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn parse_events_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<EventsCommand, String> {
    let mut source_session_id = None;
    let mut source = None;
    let mut limit = None;
    let mut all = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(EventsCommand::Help),
            "--all" => all = true,
            "--source" => {
                source = Some(
                    args.next()
                        .ok_or_else(|| "--source requires a value".to_string())?,
                );
            }
            "--session" => {
                source_session_id = Some(
                    args.next()
                        .ok_or_else(|| "--session requires a value".to_string())?,
                );
            }
            "--limit" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--limit requires a value".to_string())?;
                let parsed = value
                    .parse::<i64>()
                    .map_err(|_| format!("invalid limit: {value}"))?;
                if parsed < 1 {
                    return Err(format!("invalid limit: {value}; expected at least 1"));
                }
                limit = Some(parsed);
            }
            _ => return Err(format!("unknown events option: {arg}")),
        }
    }

    let source = source.ok_or_else(|| "events requires --source <source>".to_string())?;
    let source_session_id = source_session_id
        .ok_or_else(|| "events requires --session <source_session_id>".to_string())?;
    let selection = match (all, limit) {
        (true, Some(_)) => {
            return Err("events accepts either --limit <n> or --all, not both".to_string());
        }
        (true, None) => EventsSelection::All,
        (false, Some(limit)) => EventsSelection::Limit(limit),
        (false, None) => return Err("events requires --limit <n> or --all".to_string()),
    };

    Ok(EventsCommand::Run(EventsOptions {
        source,
        source_session_id,
        selection,
    }))
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

fn run_compact_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_compact_command(args) {
        Ok(CompactCommand::Run(options)) => options,
        Ok(CompactCommand::Help) => {
            print_compact_help();
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}");
            eprintln!("run `jottrace compact --help` for usage");
            return ExitCode::from(2);
        }
    };

    match jottrace::run_compact(options) {
        Ok(report) => {
            println!("jottrace compact");
            println!("db: {}", report.db_path.display());
            println!("mode: {}", compact_mode_name(report.mode));
            if report.mode != jottrace::CompactMode::Vacuum {
                println!("batch_size: {}", report.batch_size);
            }
            println!("raw_events_before: {}", report.raw_events_before);
            println!("zstd_events_before: {}", report.zstd_events_before);
            println!("raw_events_after: {}", report.raw_events_after);
            println!("zstd_events_after: {}", report.zstd_events_after);
            println!(
                "unsupported_codec_events: {}",
                report.unsupported_codec_events
            );
            println!("eligible_raw_events: {}", report.eligible_raw_events);
            println!("converted_events: {}", report.converted_events);
            println!("skipped_raw_events: {}", report.skipped_raw_events);
            println!("skipped_small_events: {}", report.skipped_small_events);
            println!(
                "skipped_not_smaller_events: {}",
                report.skipped_not_smaller_events
            );
            println!(
                "skipped_round_trip_failed_events: {}",
                report.skipped_round_trip_failed_events
            );
            println!("stored_bytes_before: {}", report.stored_bytes_before);
            println!("stored_bytes_after: {}", report.stored_bytes_after);
            println!("estimated_bytes_saved: {}", report.estimated_bytes_saved);
            println!(
                "sqlite_reclaimable_bytes_before: {}",
                report.sqlite_reclaimable_bytes_before
            );
            println!(
                "sqlite_reclaimable_bytes: {}",
                report.sqlite_reclaimable_bytes
            );
            println!(
                "unresolved_ingest_errors: {}",
                report.unresolved_ingest_errors
            );
            if report.mode == jottrace::CompactMode::DryRun {
                println!(
                    "next: rerun with `jottrace compact --apply` to rewrite eligible payloads"
                );
            }
            println!(
                "disk_reclaim: after applying, run `jottrace compact --vacuum` to reclaim free SQLite pages"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("jottrace compact failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_compact_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<CompactCommand, String> {
    let mut options = jottrace::CompactOptions::default();
    let mut explicit_mode = false;
    let mut batch_size_provided = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(CompactCommand::Help),
            "--apply" => {
                if explicit_mode {
                    return Err("compact accepts only one of --apply or --vacuum".to_string());
                }
                explicit_mode = true;
                options.mode = jottrace::CompactMode::Apply;
            }
            "--vacuum" => {
                if explicit_mode {
                    return Err("compact accepts only one of --apply or --vacuum".to_string());
                }
                if batch_size_provided {
                    return Err("compact --vacuum does not accept --batch-size".to_string());
                }
                explicit_mode = true;
                options.mode = jottrace::CompactMode::Vacuum;
            }
            "--batch-size" => {
                if options.mode == jottrace::CompactMode::Vacuum {
                    return Err("compact --vacuum does not accept --batch-size".to_string());
                }
                let value = args
                    .next()
                    .ok_or_else(|| "--batch-size requires a value".to_string())?;
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid batch size: {value}"))?;
                if parsed < 1 {
                    return Err(format!("invalid batch size: {value}; expected at least 1"));
                }
                if parsed > jottrace::compact::MAX_COMPACT_BATCH_SIZE {
                    return Err(format!(
                        "invalid batch size: {value}; expected at most {}",
                        jottrace::compact::MAX_COMPACT_BATCH_SIZE
                    ));
                }
                options.batch_size = parsed;
                batch_size_provided = true;
            }
            _ => return Err(format!("unknown compact option: {arg}")),
        }
    }

    Ok(CompactCommand::Run(options))
}

fn compact_mode_name(mode: jottrace::CompactMode) -> &'static str {
    match mode {
        jottrace::CompactMode::DryRun => "dry-run",
        jottrace::CompactMode::Apply => "apply",
        jottrace::CompactMode::Vacuum => "vacuum",
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
    println!("  jottrace compact [--apply|--vacuum] [--batch-size <n>]");
    println!("  jottrace doctor");
    println!(
        "  jottrace events --source <source> --session <source_session_id> (--limit <n>|--all)"
    );
    println!("  jottrace ingest");
    println!("  jottrace status");
    println!("  jottrace web [--port <port>] [--once]");
    println!("  jottrace --version");
}

fn print_compact_help() {
    println!("jottrace compact");
    println!("Report or rewrite eligible raw event payloads as zstd.");
    println!();
    println!("Usage:");
    println!("  jottrace compact [--apply|--vacuum] [--batch-size <n>]");
    println!();
    println!("Options:");
    println!("  --apply           Rewrite eligible raw payload rows in bounded batches");
    println!("  --vacuum          Reclaim free SQLite pages after compaction");
    println!("  --batch-size <n>  Raw rows to inspect per batch (default: 1000, max: 10000)");
}

fn print_events_help() {
    println!("jottrace events");
    println!("Print decoded event JSONL for one stored session.");
    println!();
    println!("Usage:");
    println!(
        "  jottrace events --source <source> --session <source_session_id> (--limit <n>|--all)"
    );
    println!();
    println!("Options:");
    println!("  --all                         Print every event in the selected session");
    println!("  --source <source>             Stored source, for example claude_cli");
    println!("  --session <source_session_id>  Stored source session id to inspect");
    println!("  --limit <n>                   Maximum number of events to print");
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
