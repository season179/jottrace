use std::env;
use std::fmt::Display;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    // The first env arg is the executable path; commands start after it.
    let mut args = env::args().skip(1);
    let command = args.next();

    match command.as_deref() {
        None | Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("--version") | Some("-V") => {
            println!("jottrace {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("compact") => run_compact_command(args),
        Some("doctor") => run_doctor_command(args),
        Some("events") => run_events_command(args),
        Some("ingest") => run_ingest_command(args),
        Some("pack") => run_pack_command(args),
        Some("settle") => run_settle_command(args),
        Some("status") => run_status_command(args),
        Some("update") | Some("upgrade") => run_update_command(args),
        Some("web") => run_web_command(args),
        Some(command) if jottrace::update::is_auto_update_command(command) => {
            run_auto_update_background_command(args)
        }
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
struct CompactCliOptions {
    compact_options: jottrace::CompactOptions,
    details: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactCommand {
    Run(CompactCliOptions),
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DetailOptions {
    details: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailCommand {
    Run(DetailOptions),
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleCommand {
    Run,
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
            eprint_command_usage("events", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    let db_path = match jottrace::storage::db_path_from_env() {
        Ok(db_path) => db_path,
        Err(error) => {
            eprint_command_failure("events", error);
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
        eprint_command_failure("events", error);
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
            eprint_command_usage("web", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    let db_path = match jottrace::storage::db_path_from_env() {
        Ok(db_path) => db_path,
        Err(error) => {
            eprint_command_failure("web", error);
            return ExitCode::FAILURE;
        }
    };

    let server = match jottrace::web::WebServer::bind(db_path.clone(), options.port) {
        Ok(server) => server,
        Err(error) => {
            eprint_command_failure("web", error);
            return ExitCode::FAILURE;
        }
    };

    print_web_startup(server.local_url(), &db_path);

    let result = if options.once {
        server.serve_once()
    } else {
        server.serve_forever()
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprint_command_failure("web", error);
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

fn run_ingest_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_detail_command("ingest", args) {
        Ok(DetailCommand::Help) => {
            print_ingest_help();
            return ExitCode::SUCCESS;
        }
        Ok(DetailCommand::Run(options)) => options,
        Err(message) => {
            eprint_command_usage("ingest", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    match jottrace::run_ingest() {
        Ok(report) => {
            print_ingest_report(&report, options.details);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure("ingest", error);
            ExitCode::FAILURE
        }
    }
}

fn parse_simple_command(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<SimpleCommand, String> {
    match args.next() {
        None => Ok(SimpleCommand::Run),
        Some(arg) if arg == "--help" || arg == "-h" => Ok(SimpleCommand::Help),
        Some(arg) => Err(format!("unknown {command} option: {arg}")),
    }
}

fn parse_detail_command(
    command: &str,
    args: impl Iterator<Item = String>,
) -> std::result::Result<DetailCommand, String> {
    let mut options = DetailOptions { details: false };

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(DetailCommand::Help),
            "--details" => options.details = true,
            _ => return Err(format!("unknown {command} option: {arg}")),
        }
    }

    Ok(DetailCommand::Run(options))
}

fn run_update_command(args: impl Iterator<Item = String>) -> ExitCode {
    match parse_simple_command("update", args) {
        Ok(SimpleCommand::Help) => {
            print_update_help();
            ExitCode::SUCCESS
        }
        Ok(SimpleCommand::Run) => match jottrace::run_update() {
            Ok(report) => {
                print_update_report(&report);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprint_command_failure("update", error);
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            eprint_command_usage("update", &message);
            ExitCode::from(2)
        }
    }
}

fn run_auto_update_background_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    if args.next().is_some() {
        return ExitCode::from(2);
    }
    jottrace::update::run_auto_update_silent();
    ExitCode::SUCCESS
}

fn run_compact_command(args: impl Iterator<Item = String>) -> ExitCode {
    let cli_options = match parse_compact_command(args) {
        Ok(CompactCommand::Run(cli_options)) => cli_options,
        Ok(CompactCommand::Help) => {
            print_compact_help();
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprint_command_usage("compact", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    match jottrace::compact::run_compact_with_diagnostics(
        cli_options.compact_options,
        cli_options.details,
    ) {
        Ok(report) => {
            print_compact_report(&report, cli_options.details);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure("compact", error);
            ExitCode::FAILURE
        }
    }
}

fn eprint_command_usage(command: &str, message: &str) {
    eprintln!("{message}");
    eprintln!("run `jottrace {command} --help` for usage");
}

fn eprint_command_failure(command: &str, error: impl Display) {
    eprintln!("jottrace {command} failed: {error}");
}

fn print_web_startup(local_url: impl AsRef<str>, db_path: &Path) {
    println!("jottrace web");
    println!("url: {}", local_url.as_ref());
    println!("db: {}", db_path.display());
    let _ = io::stdout().flush();
}

fn print_update_report(report: &jottrace::UpdateReport) {
    println!("jottrace update");
    println!("current_version: {}", report.current_version);
    println!("target_version: {}", report.target_version);
    println!("install_path: {}", report.install_path.display());
    println!("result: {}", report.result.as_str());
}

fn print_ingest_report(report: &jottrace::IngestReport, details: bool) {
    println!("jottrace ingest");
    if details {
        println!("db: {}", report.db_path.display());
    }
    println!("files: {}", report.file_count);
    println!("sessions: {}", report.session_count);
    println!("events: {}", report.event_count);
    println!("inserted_events: {}", report.inserted_event_count);
    println!("skipped_files: {}", report.skipped_file_count);
    println!(
        "unresolved_ingest_errors: {}",
        report.unresolved_ingest_error_count
    );
}

fn print_status_report(report: &jottrace::StatusReport, details: bool) {
    println!("jottrace status");
    if details {
        println!("db: {}", report.db_path.display());
        println!("schema_version: {}", report.schema_version);
    }
    println!("sessions: {}", report.session_count);
    println!("events: {}", report.event_count);
    println!(
        "unresolved_ingest_errors: {}",
        report.unresolved_ingest_error_count
    );
}

fn print_doctor_report(report: &jottrace::DoctorReport, details: bool) {
    println!("jottrace doctor");
    if details {
        println!("data_dir: {} (ok)", report.data_dir.display());
    }
    println!("permissions: private (ok)");
    println!(
        "unresolved_ingest_errors: {}",
        report.unresolved_ingest_error_count
    );
    if !details && report.unresolved_ingest_error_count > 0 {
        println!("next: run `jottrace doctor --details` to inspect recent ingest errors");
    }
    if details && !report.recent_ingest_errors.is_empty() {
        println!(
            "recent_ingest_errors: {}",
            report.recent_ingest_errors.len()
        );
    }
    if details {
        for ingest_error in &report.recent_ingest_errors {
            print_recent_ingest_error(ingest_error);
        }
    }
}

fn print_recent_ingest_error(ingest_error: &jottrace::IngestErrorSummary) {
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

fn print_pack_report(report: &jottrace::PackReport) {
    println!("jottrace pack");
    println!("archive: {}", report.archive.display());
    println!("bytes: {}", report.archive_bytes);
    println!("schema_version: {}", report.schema_version);
    println!("sessions: {}", report.session_count);
    println!("events: {}", report.event_count);
    println!("next: copy to the destination, then run `jottrace settle <archive>`");
}

fn print_settle_report(report: &jottrace::SettleReport) {
    println!("jottrace settle");
    println!("data_dir: {}", report.data_dir.display());
    println!("schema_version: {}", report.schema_version);
    println!("sessions: {}", report.session_count);
    println!("events: {}", report.event_count);
}

fn print_compact_report(report: &jottrace::CompactReport, details: bool) {
    println!("jottrace compact");
    println!("mode: {}", compact_mode_name(report.mode));
    if details {
        println!("db: {}", report.db_path.display());
    }
    if details && report.mode != jottrace::CompactMode::Vacuum {
        println!("batch_size: {}", report.batch_size);
    }
    println!("eligible_raw_events: {}", report.eligible_raw_events);
    if details || report.mode == jottrace::CompactMode::Apply {
        println!("converted_events: {}", report.converted_events);
    }
    println!("estimated_bytes_saved: {}", report.estimated_bytes_saved);
    if details {
        println!("raw_events_before: {}", report.raw_events_before);
        println!("zstd_events_before: {}", report.zstd_events_before);
        println!("raw_events_after: {}", report.raw_events_after);
        println!("zstd_events_after: {}", report.zstd_events_after);
        println!(
            "unsupported_codec_events: {}",
            report.unsupported_codec_events
        );
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
        println!(
            "sqlite_reclaimable_bytes_before: {}",
            report.sqlite_reclaimable_bytes_before
        );
    }
    if details || report.mode != jottrace::CompactMode::DryRun {
        println!(
            "sqlite_reclaimable_bytes: {}",
            report.sqlite_reclaimable_bytes
        );
    }
    println!(
        "unresolved_ingest_errors: {}",
        report.unresolved_ingest_errors
    );
    if report.mode == jottrace::CompactMode::DryRun {
        println!("next: rerun with `jottrace compact --apply` to rewrite eligible payloads");
    }
    if details || report.mode == jottrace::CompactMode::Apply {
        println!(
            "disk_reclaim: after applying, run `jottrace compact --vacuum` to reclaim free SQLite pages"
        );
    }
}

fn parse_compact_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<CompactCommand, String> {
    let mut options = jottrace::CompactOptions::default();
    let mut details = false;
    let mut explicit_mode = false;
    let mut batch_size_provided = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(CompactCommand::Help),
            "--details" => details = true,
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

    Ok(CompactCommand::Run(CompactCliOptions {
        compact_options: options,
        details,
    }))
}

fn compact_mode_name(mode: jottrace::CompactMode) -> &'static str {
    match mode {
        jottrace::CompactMode::DryRun => "dry-run",
        jottrace::CompactMode::Apply => "apply",
        jottrace::CompactMode::Vacuum => "vacuum",
    }
}

fn run_doctor_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_detail_command("doctor", args) {
        Ok(DetailCommand::Help) => {
            print_doctor_help();
            return ExitCode::SUCCESS;
        }
        Ok(DetailCommand::Run(options)) => options,
        Err(message) => {
            eprint_command_usage("doctor", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    // Keep the CLI responsible for presentation while the library owns the
    // filesystem checks. That makes future commands easier to test directly.
    match jottrace::run_doctor_with_options(jottrace::DoctorOptions {
        include_recent_errors: options.details,
    }) {
        Ok(report) => {
            print_doctor_report(&report, options.details);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure("doctor", error);
            ExitCode::FAILURE
        }
    }
}

fn run_status_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_detail_command("status", args) {
        Ok(DetailCommand::Help) => {
            print_status_help();
            return ExitCode::SUCCESS;
        }
        Ok(DetailCommand::Run(options)) => options,
        Err(message) => {
            eprint_command_usage("status", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    match jottrace::run_status() {
        Ok(report) => {
            print_status_report(&report, options.details);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure("status", error);
            ExitCode::FAILURE
        }
    }
}

enum PackCommand {
    Run(PackCliOptions),
    Help,
}

struct PackCliOptions {
    output: Option<PathBuf>,
}

enum SettleCommand {
    Run(SettleCliOptions),
    Help,
}

struct SettleCliOptions {
    archive: PathBuf,
    force: bool,
}

fn parse_pack_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<PackCommand, String> {
    let mut options = PackCliOptions { output: None };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(PackCommand::Help),
            "--output" | "-o" => {
                let value = args
                    .next()
                    .ok_or_else(|| String::from("missing value for --output"))?;
                options.output = Some(PathBuf::from(value));
            }
            _ => return Err(format!("unknown pack option: {arg}")),
        }
    }
    Ok(PackCommand::Run(options))
}

fn parse_settle_command(
    args: impl Iterator<Item = String>,
) -> std::result::Result<SettleCommand, String> {
    let mut archive: Option<PathBuf> = None;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(SettleCommand::Help),
            "--force" => force = true,
            arg if arg.starts_with("--") => {
                return Err(format!("unknown settle option: {arg}"));
            }
            _ => {
                if archive.is_some() {
                    return Err(format!("unexpected positional argument: {arg}"));
                }
                archive = Some(PathBuf::from(arg));
            }
        }
    }
    let archive = archive.ok_or_else(|| String::from("missing archive path"))?;
    Ok(SettleCommand::Run(SettleCliOptions { archive, force }))
}

fn run_pack_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_pack_command(args) {
        Ok(PackCommand::Help) => {
            print_pack_help();
            return ExitCode::SUCCESS;
        }
        Ok(PackCommand::Run(options)) => options,
        Err(message) => {
            eprint_command_usage("pack", &message);
            return ExitCode::from(2);
        }
    };
    jottrace::update::maybe_spawn_auto_update();

    match jottrace::run_pack(jottrace::PackOptions {
        output: options.output,
    }) {
        Ok(report) => {
            print_pack_report(&report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure("pack", error);
            ExitCode::FAILURE
        }
    }
}

fn run_settle_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match parse_settle_command(args) {
        Ok(SettleCommand::Help) => {
            print_settle_help();
            return ExitCode::SUCCESS;
        }
        Ok(SettleCommand::Run(options)) => options,
        Err(message) => {
            eprint_command_usage("settle", &message);
            return ExitCode::from(2);
        }
    };

    match jottrace::run_settle(jottrace::SettleOptions {
        archive: options.archive,
        force: options.force,
    }) {
        Ok(report) => {
            print_settle_report(&report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure("settle", error);
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
    println!("  jottrace compact [--apply|--vacuum] [--batch-size <n>] [--details]");
    println!("  jottrace doctor [--details]");
    println!(
        "  jottrace events --source <source> --session <source_session_id> (--limit <n>|--all)"
    );
    println!("  jottrace ingest [--details]");
    println!("  jottrace pack [--output <path>]");
    println!("  jottrace settle <archive> [--force]");
    println!("  jottrace status [--details]");
    println!("  jottrace update");
    println!("  jottrace upgrade");
    println!("  jottrace web [--port <port>] [--once]");
    println!("  jottrace --version");
    println!("  jottrace <command> --help");
}

fn print_pack_help() {
    println!("jottrace pack");
    println!("Bundle the journal directory into a single tar.gz for transport.");
    println!();
    println!("Usage:");
    println!("  jottrace pack [--output <path>]");
    println!();
    println!("Options:");
    println!(
        "  --output <path>  Write the archive to <path> (default: jottrace-pack-<utc>.tar.gz)"
    );
    println!();
    println!("The archive is created with mode 0600. Move it to the destination,");
    println!("then run `jottrace settle <archive>` there.");
}

fn print_settle_help() {
    println!("jottrace settle");
    println!("Unpack a `jottrace pack` archive into the local journal directory.");
    println!();
    println!("Usage:");
    println!("  jottrace settle <archive> [--force]");
    println!();
    println!("Options:");
    println!("  --force  Overwrite an existing non-empty journal at JOTTRACE_HOME");
    println!();
    println!("Permissions are restored to 0700/0600 and schema migrations run on open.");
}

fn print_compact_help() {
    println!("jottrace compact");
    println!("Report or rewrite eligible raw event payloads as zstd.");
    println!();
    println!("Usage:");
    println!("  jottrace compact [--apply|--vacuum] [--batch-size <n>] [--details]");
    println!();
    println!("Options:");
    println!("  --apply           Rewrite eligible raw payload rows in bounded batches");
    println!("  --vacuum          Reclaim free SQLite pages after compaction");
    println!("  --batch-size <n>  Raw rows to inspect per batch (default: 1000, max: 10000)");
    println!("  --details         Include database path and full compaction counters");
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

fn print_doctor_help() {
    println!("jottrace doctor");
    println!("Check the journal directory, database, permissions, and ingest errors.");
    println!();
    println!("Usage:");
    println!("  jottrace doctor [--details]");
    println!();
    println!("Options:");
    println!("  --details  Include data directory and recent ingest-error fields");
}

fn print_ingest_help() {
    println!("jottrace ingest");
    println!("Preserve Claude and Codex JSONL sessions into the local journal.");
    println!();
    println!("Usage:");
    println!("  jottrace ingest [--details]");
    println!();
    println!("Options:");
    println!("  --details  Include the database path");
}

fn print_status_help() {
    println!("jottrace status");
    println!("Print journal schema, session, event, and ingest-error counts.");
    println!();
    println!("Usage:");
    println!("  jottrace status [--details]");
    println!();
    println!("Options:");
    println!("  --details  Include the database path and schema version");
}

fn print_update_help() {
    println!("jottrace update");
    println!("Replace the installed binary with the matching GitHub Release artifact.");
    println!();
    println!("Usage:");
    println!("  jottrace update");
    println!("  jottrace upgrade");
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
