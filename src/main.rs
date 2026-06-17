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
        Some("taste") => run_taste_command(args),
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

/// Outcome of parsing a command's arguments: run with `T` options, or print
/// help. Parse errors are returned separately as `Err(String)` by the parse
/// functions and surfaced by [`resolve_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedCommand<T> {
    Run(T),
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WebOptions {
    port: u16,
    once: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactCliOptions {
    compact_options: jottrace::CompactOptions,
    details: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DetailOptions {
    details: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleCommand {
    Run,
    Help,
}

fn run_events_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "events",
        parse_events_command(args),
        print_events_help,
        |options| {
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
        },
    )
}

fn parse_events_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<EventsOptions>, String> {
    let mut source_session_id = None;
    let mut source = None;
    let mut limit = None;
    let mut all = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--all" => all = true,
            "--source" => {
                source = Some(next_flag_value(&mut args, "--source")?);
            }
            "--session" => {
                source_session_id = Some(next_flag_value(&mut args, "--session")?);
            }
            "--limit" => {
                let value = next_flag_value(&mut args, "--limit")?;
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

    Ok(ParsedCommand::Run(EventsOptions {
        source,
        source_session_id,
        selection,
    }))
}

fn run_web_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command("web", parse_web_command(args), print_web_help, |options| {
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
    })
}

fn parse_web_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<WebOptions>, String> {
    let mut options = WebOptions {
        port: 0,
        once: false,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--once" => options.once = true,
            "--port" => {
                let value = next_flag_value(&mut args, "--port")?;
                options.port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            _ => return Err(format!("unknown web option: {arg}")),
        }
    }

    Ok(ParsedCommand::Run(options))
}

fn run_ingest_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "ingest",
        parse_detail_command("ingest", args),
        print_ingest_help,
        |options| {
            finish_command("ingest", jottrace::run_ingest(), |report| {
                print_ingest_report(report, options.details);
            })
        },
    )
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
) -> std::result::Result<ParsedCommand<DetailOptions>, String> {
    let mut options = DetailOptions { details: false };

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--details" => options.details = true,
            _ => return Err(format!("unknown {command} option: {arg}")),
        }
    }

    Ok(ParsedCommand::Run(options))
}

fn run_update_command(args: impl Iterator<Item = String>) -> ExitCode {
    match parse_simple_command("update", args) {
        Ok(SimpleCommand::Help) => {
            print_update_help();
            ExitCode::SUCCESS
        }
        Ok(SimpleCommand::Run) => {
            finish_command("update", jottrace::run_update(), print_update_report)
        }
        Err(message) => {
            eprint_command_usage("update", &message);
            ExitCode::from(2)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TasteExtractCliOptions {
    extract_options: jottrace::TasteExtractOptions,
}

fn run_taste_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        None | Some("help") | Some("--help") | Some("-h") => {
            print_taste_help();
            ExitCode::SUCCESS
        }
        Some("extract") => run_taste_extract_command(args),
        Some("status") => run_taste_status_command(args),
        Some("show") => run_taste_show_command(args),
        Some("export") => run_taste_export_command(args),
        Some(subcommand) => {
            eprintln!("unknown taste subcommand: {subcommand}");
            eprintln!("run `jottrace taste --help` for usage");
            ExitCode::from(2)
        }
    }
}

fn run_taste_show_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        None | Some("help") | Some("--help") | Some("-h") => {
            print_taste_show_help();
            ExitCode::SUCCESS
        }
        Some("timeline") => run_taste_show_timeline_command(args),
        Some("example") => run_taste_show_example_command(args),
        Some(subcommand) => {
            eprintln!("unknown taste show subcommand: {subcommand}");
            eprintln!("run `jottrace taste show --help` for usage");
            ExitCode::from(2)
        }
    }
}

fn run_taste_show_example_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "taste show example",
        parse_taste_show_example_command(args),
        print_taste_show_example_help,
        |options| {
            finish_command(
                "taste show example",
                jottrace::run_taste_show_example(options),
                print_taste_example_report,
            )
        },
    )
}

fn run_taste_show_timeline_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "taste show timeline",
        parse_taste_show_timeline_command(args),
        print_taste_show_timeline_help,
        |options| {
            finish_command(
                "taste show timeline",
                jottrace::run_taste_show_timeline(options),
                print_taste_timeline_report,
            )
        },
    )
}

/// Consume the value following a flag, erroring `"{flag} requires a value"`
/// when the flag is the final argument.
fn next_flag_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> std::result::Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// Consume the value following a single-value flag, rejecting a repeated flag.
///
/// The value is always consumed first, so a missing value is reported before a
/// duplicate — matching the inline order these call sites previously used.
fn take_single_flag_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
    command: &str,
    already_set: bool,
) -> std::result::Result<String, String> {
    let value = next_flag_value(args, flag)?;
    if already_set {
        return Err(format!("{command} accepts only one {flag} value"));
    }
    Ok(value)
}

fn parse_taste_show_example_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<jottrace::TasteShowExampleOptions>, String> {
    let mut source_session_id = None;
    let mut tool_use_id = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--session" => {
                source_session_id = Some(take_single_flag_value(
                    &mut args,
                    "--session",
                    "taste show example",
                    source_session_id.is_some(),
                )?);
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown taste show example option: {value}"));
            }
            value => {
                if tool_use_id.is_some() {
                    return Err(
                        "taste show example accepts only one tool_use_id argument".to_string()
                    );
                }
                tool_use_id = Some(value.to_string());
            }
        }
    }

    let tool_use_id =
        tool_use_id.ok_or_else(|| "taste show example requires <tool_use_id>".to_string())?;

    Ok(ParsedCommand::Run(jottrace::TasteShowExampleOptions {
        tool_use_id,
        source_session_id,
    }))
}

fn parse_taste_show_timeline_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<jottrace::TasteShowTimelineOptions>, String> {
    let mut source_session_id = None;
    let mut file_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--session" => {
                source_session_id = Some(take_single_flag_value(
                    &mut args,
                    "--session",
                    "taste show timeline",
                    source_session_id.is_some(),
                )?);
            }
            "--file" => {
                file_path = Some(take_single_flag_value(
                    &mut args,
                    "--file",
                    "taste show timeline",
                    file_path.is_some(),
                )?);
            }
            _ => return Err(format!("unknown taste show timeline option: {arg}")),
        }
    }

    let source_session_id = source_session_id
        .ok_or_else(|| "taste show timeline requires --session <source_session_id>".to_string())?;
    let file_path =
        file_path.ok_or_else(|| "taste show timeline requires --file <path>".to_string())?;

    Ok(ParsedCommand::Run(jottrace::TasteShowTimelineOptions {
        source_session_id,
        file_path,
    }))
}

fn print_taste_example_report(report: &jottrace::TasteExampleShowReport) {
    let example = &report.example;
    println!("jottrace taste show example");
    println!("tool_use_id: {}", example.tool_use_id);
    println!("session: {}", example.source_session_id);
    println!("generation: {}", example.generation);
    println!("proposal_event_seq: {}", example.proposal_event_seq);
    println!("file: {}", example.file_path.as_deref().unwrap_or("-"));
    println!("tool: {}", example.tool_name);
    println!("outcome: {}", example.outcome.as_str());
    println!("confidence: {}", example.confidence);
    println!("evidence_kind: {}", example.evidence_kind.as_str());
    println!("extractor_version: {}", example.extractor_version);
    println!("proposal:");
    match &example.proposal_content {
        Some(content) => println!("{content}"),
        None => println!("<missing proposal>"),
    }
    println!("context:");
    match &example.context {
        Some(content) => println!("{content}"),
        None => println!("<missing context>"),
    }
}

fn print_taste_timeline_report(report: &jottrace::TasteTimelineShowReport) {
    println!("jottrace taste show timeline");
    println!("session: {}", report.source_session_id);
    println!("file: {}", report.file_path);
    println!("rows: {}", report.rows.len());
    for row in &report.rows {
        let trigger = row.trigger_event_ref.as_deref().unwrap_or("-");
        println!(
            "seq {} (event_seq {}) [{}] trigger={}",
            row.seq,
            row.event_seq,
            row.source_kind.as_str(),
            trigger
        );
        match &row.content {
            Some(content) => println!("{content}"),
            None => println!("<missing content>"),
        }
        if row.seq + 1 < report.rows.len() {
            println!();
        }
    }
}

fn run_taste_status_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "taste status",
        parse_detail_command("taste status", args),
        print_taste_status_help,
        |options| {
            finish_command("taste status", jottrace::run_taste_status(), |report| {
                print_taste_status_report(report, options.details);
            })
        },
    )
}

fn print_taste_status_report(report: &jottrace::TasteStatusReport, details: bool) {
    println!("jottrace taste status");
    if details {
        println!("db: {}", report.db_path.display());
        println!("extractor_version: {}", report.extractor_version);
        println!("evidence:");
        println!("  direct_edit: {}", report.evidence.direct_edit);
        println!("  direct_write: {}", report.evidence.direct_write);
        println!("  bash_correlation: {}", report.evidence.bash_correlation);
        println!("  mcp_correlation: {}", report.evidence.mcp_correlation);
        println!("  permission_denial: {}", report.evidence.permission_denial);
        println!(
            "  missing_final_state: {}",
            report.evidence.missing_final_state
        );
        let low_confidence = report
            .proposals
            .saturating_sub(report.high_confidence_proposals);
        println!("low_confidence_proposals: {low_confidence}");
    }
    println!("claude_parent_sessions: {}", report.claude_parent_sessions);
    println!("sessions_processed: {}", report.sessions_processed);
    println!("sessions_pending: {}", report.sessions_pending);
    println!("proposals: {}", report.proposals);
    println!("outcomes:");
    println!("  accepted: {}", report.outcomes.accepted);
    println!("  rejected: {}", report.outcomes.rejected);
    println!("  edited: {}", report.outcomes.edited);
    println!("high_confidence_coverage: {:.1}%", report.coverage_percent);
    if !details {
        println!("extractor_version: {}", report.extractor_version);
    }
}

fn run_taste_export_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "taste export",
        parse_taste_export_command(args),
        print_taste_export_help,
        |options| {
            finish_command(
                "taste export",
                jottrace::run_taste_export(options),
                print_taste_export_report,
            )
        },
    )
}

fn parse_taste_export_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<jottrace::TasteExportOptions>, String> {
    let mut format = None;
    let mut output_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--format" => {
                let value = take_single_flag_value(
                    &mut args,
                    "--format",
                    "taste export",
                    format.is_some(),
                )?;
                format = Some(
                    jottrace::TasteExportFormat::from_cli(&value)
                        .ok_or_else(|| format!("unsupported export format: {value}"))?,
                );
            }
            "--out" => {
                let value = take_single_flag_value(
                    &mut args,
                    "--out",
                    "taste export",
                    output_path.is_some(),
                )?;
                output_path = Some(PathBuf::from(value));
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown taste export option: {value}"));
            }
            value => return Err(format!("unexpected taste export argument: {value}")),
        }
    }

    let format = format.ok_or_else(|| "taste export requires --format <format>".to_string())?;

    Ok(ParsedCommand::Run(jottrace::TasteExportOptions {
        format,
        output_path,
    }))
}

fn print_taste_export_report(report: &jottrace::TasteExportReport) {
    eprintln!("jottrace taste export");
    eprintln!("format: {}", report.format.as_str());
    match &report.output_path {
        Some(path) => eprintln!("out: {}", path.display()),
        None => eprintln!("out: <stdout>"),
    }
    eprintln!("rows_exported: {}", report.rows_exported);
}

fn run_taste_extract_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "taste extract",
        parse_taste_extract_command(args),
        print_taste_extract_help,
        |options| {
            finish_command(
                "taste extract",
                jottrace::run_taste_extract(options.extract_options),
                print_taste_extract_report,
            )
        },
    )
}

fn parse_taste_extract_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<TasteExtractCliOptions>, String> {
    let mut options = jottrace::TasteExtractOptions::default();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--force" => options.force = true,
            "--session" => {
                options.source_session_id = Some(take_single_flag_value(
                    &mut args,
                    "--session",
                    "taste extract",
                    options.source_session_id.is_some(),
                )?);
            }
            _ => return Err(format!("unknown taste extract option: {arg}")),
        }
    }

    Ok(ParsedCommand::Run(TasteExtractCliOptions {
        extract_options: options,
    }))
}

fn print_taste_extract_report(report: &jottrace::TasteExtractReport) {
    println!("sessions_processed: {}", report.sessions_processed);
    println!("sessions_skipped: {}", report.sessions_skipped);
    println!("timeline_rows: {}", report.timeline_rows_written);
    println!(
        "preference_examples: {}",
        report.preference_examples_written
    );
}

fn run_auto_update_background_command(mut args: impl Iterator<Item = String>) -> ExitCode {
    if args.next().is_some() {
        return ExitCode::from(2);
    }
    jottrace::update::run_auto_update_silent();
    ExitCode::SUCCESS
}

fn run_compact_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "compact",
        parse_compact_command(args),
        print_compact_help,
        |cli_options| {
            finish_command(
                "compact",
                jottrace::compact::run_compact_with_diagnostics(
                    cli_options.compact_options,
                    cli_options.details,
                ),
                |report| print_compact_report(report, cli_options.details),
            )
        },
    )
}

fn eprint_command_usage(command: &str, message: &str) {
    eprintln!("{message}");
    eprintln!("run `jottrace {command} --help` for usage");
}

fn eprint_command_failure(command: &str, error: impl Display) {
    eprintln!("jottrace {command} failed: {error}");
}

/// Resolve a parsed command into its run options, or an [`ExitCode`] the caller
/// should return immediately: `Help` prints usage and exits 0, a parse error
/// prints the usage hint and exits 2.
fn resolve_command<T>(
    command: &str,
    parsed: std::result::Result<ParsedCommand<T>, String>,
    print_help: fn(),
) -> std::result::Result<T, ExitCode> {
    match parsed {
        Ok(ParsedCommand::Run(options)) => Ok(options),
        Ok(ParsedCommand::Help) => {
            print_help();
            Err(ExitCode::SUCCESS)
        }
        Err(message) => {
            eprint_command_usage(command, &message);
            Err(ExitCode::from(2))
        }
    }
}

/// Resolve a parsed command, spawn the background auto-update check, then run
/// `body` with the resolved options. Centralizes the resolve-then-spawn prologue
/// shared by every option-carrying command runner: `Help` and parse errors
/// short-circuit to their exit code (via [`resolve_command`]) without spawning
/// the updater or invoking `body`.
fn run_resolved_command<T>(
    command: &str,
    parsed: std::result::Result<ParsedCommand<T>, String>,
    print_help: fn(),
    body: impl FnOnce(T) -> ExitCode,
) -> ExitCode {
    match resolve_command(command, parsed, print_help) {
        Ok(options) => {
            jottrace::update::maybe_spawn_auto_update();
            body(options)
        }
        Err(code) => code,
    }
}

/// Print a command's report on success, or report the failure, mapping each to
/// the matching exit code. Centralizes the Ok->print->SUCCESS /
/// Err->report-failure->FAILURE shape shared by every report-producing command.
fn finish_command<R, E: Display>(
    command: &str,
    result: std::result::Result<R, E>,
    print: impl FnOnce(&R),
) -> ExitCode {
    match result {
        Ok(report) => {
            print(&report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint_command_failure(command, error);
            ExitCode::FAILURE
        }
    }
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
) -> std::result::Result<ParsedCommand<CompactCliOptions>, String> {
    let mut options = jottrace::CompactOptions::default();
    let mut details = false;
    let mut explicit_mode = false;
    let mut batch_size_provided = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
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
                let value = next_flag_value(&mut args, "--batch-size")?;
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

    Ok(ParsedCommand::Run(CompactCliOptions {
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
    run_resolved_command(
        "doctor",
        parse_detail_command("doctor", args),
        print_doctor_help,
        |options| {
            // Keep the CLI responsible for presentation while the library owns
            // the filesystem checks. That makes future commands easier to test
            // directly.
            finish_command(
                "doctor",
                jottrace::run_doctor_with_options(jottrace::DoctorOptions {
                    include_recent_errors: options.details,
                }),
                |report| print_doctor_report(report, options.details),
            )
        },
    )
}

fn run_status_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "status",
        parse_detail_command("status", args),
        print_status_help,
        |options| {
            finish_command("status", jottrace::run_status(), |report| {
                print_status_report(report, options.details);
            })
        },
    )
}

struct PackCliOptions {
    output: Option<PathBuf>,
}

struct SettleCliOptions {
    archive: PathBuf,
    force: bool,
}

fn parse_pack_command(
    mut args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<PackCliOptions>, String> {
    let mut options = PackCliOptions { output: None };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
            "--output" | "-o" => {
                let value = next_flag_value(&mut args, "--output")?;
                options.output = Some(PathBuf::from(value));
            }
            _ => return Err(format!("unknown pack option: {arg}")),
        }
    }
    Ok(ParsedCommand::Run(options))
}

fn parse_settle_command(
    args: impl Iterator<Item = String>,
) -> std::result::Result<ParsedCommand<SettleCliOptions>, String> {
    let mut archive: Option<PathBuf> = None;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(ParsedCommand::Help),
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
    Ok(ParsedCommand::Run(SettleCliOptions { archive, force }))
}

fn run_pack_command(args: impl Iterator<Item = String>) -> ExitCode {
    run_resolved_command(
        "pack",
        parse_pack_command(args),
        print_pack_help,
        |options| {
            finish_command(
                "pack",
                jottrace::run_pack(jottrace::PackOptions {
                    output: options.output,
                }),
                print_pack_report,
            )
        },
    )
}

fn run_settle_command(args: impl Iterator<Item = String>) -> ExitCode {
    let options = match resolve_command("settle", parse_settle_command(args), print_settle_help) {
        Ok(options) => options,
        Err(code) => return code,
    };

    finish_command(
        "settle",
        jottrace::run_settle(jottrace::SettleOptions {
            archive: options.archive,
            force: options.force,
        }),
        print_settle_report,
    )
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
    println!("  jottrace taste extract [--session <source_session_id>] [--force]");
    println!("  jottrace taste status [--details]");
    println!("  jottrace taste show timeline --session <id> --file <path>");
    println!("  jottrace taste show example [--session <id>] <tool_use_id>");
    println!("  jottrace taste export --format jsonl [--out <path>]");
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

fn print_taste_help() {
    println!("jottrace taste");
    println!("Extract labeled coding preference examples from preserved Claude sessions.");
    println!();
    println!("Usage:");
    println!("  jottrace taste extract [--session <source_session_id>] [--force]");
    println!("  jottrace taste status [--details]");
    println!("  jottrace taste show timeline --session <id> --file <path>");
    println!("  jottrace taste show example [--session <id>] <tool_use_id>");
    println!("  jottrace taste export --format jsonl [--out <path>]");
    println!();
    println!("Run `jottrace taste extract --help` for extraction options.");
    println!("Run `jottrace taste status --help` for status options.");
    println!("Run `jottrace taste show timeline --help` for timeline inspection options.");
    println!("Run `jottrace taste show example --help` for example inspection options.");
    println!("Run `jottrace taste export --help` for export options.");
}

fn print_taste_show_help() {
    println!("jottrace taste show");
    println!("Inspect materialized taste extraction artifacts.");
    println!();
    println!("Usage:");
    println!("  jottrace taste show timeline --session <source_session_id> --file <path>");
    println!("  jottrace taste show example [--session <source_session_id>] <tool_use_id>");
    println!();
    println!("Run `jottrace taste show timeline --help` for timeline options.");
    println!("Run `jottrace taste show example --help` for example options.");
}

fn print_taste_show_example_help() {
    println!("jottrace taste show example");
    println!("Inspect one labeled preference example with full context.");
    println!();
    println!("Usage:");
    println!("  jottrace taste show example [--session <source_session_id>] <tool_use_id>");
    println!();
    println!("Options:");
    println!(
        "  --session <id>  Disambiguate when the same tool_use_id appears in multiple sessions"
    );
}

fn print_taste_show_timeline_help() {
    println!("jottrace taste show timeline");
    println!("Inspect the reconstructed per-file content timeline for one session.");
    println!();
    println!("Usage:");
    println!("  jottrace taste show timeline --session <source_session_id> --file <path>");
    println!();
    println!("Options:");
    println!("  --session <id>  Claude parent source_session_id");
    println!("  --file <path>   File path as stored in file_timelines (relative to session cwd)");
}

fn print_taste_status_help() {
    println!("jottrace taste status");
    println!("Report taste extraction counts and high-confidence coverage.");
    println!();
    println!("Usage:");
    println!("  jottrace taste status [--details]");
    println!();
    println!("Options:");
    println!("  --details  Include database path, extractor version, and evidence-kind breakdown");
}

fn print_taste_extract_help() {
    println!("jottrace taste extract");
    println!("Materialize file timelines and preference examples for Claude sessions.");
    println!();
    println!("Usage:");
    println!("  jottrace taste extract [--session <source_session_id>] [--force]");
    println!("  jottrace taste status [--details]");
    println!("  jottrace taste show timeline --session <id> --file <path>");
    println!("  jottrace taste show example [--session <id>] <tool_use_id>");
    println!("  jottrace taste export --format jsonl [--out <path>]");
    println!();
    println!("Options:");
    println!("  --session <id>  Extract only the given Claude parent source_session_id");
    println!(
        "  --force         Re-extract even when rows already use the current extractor version"
    );
}

fn print_taste_export_help() {
    println!("jottrace taste export");
    println!("Emit labeled preference examples as JSONL for external trainer consumption.");
    println!();
    println!("Usage:");
    println!("  jottrace taste export --format jsonl [--out <path>]");
    println!();
    println!("Options:");
    println!("  --format <format>  Export format (only jsonl is supported today)");
    println!("  --out <path>       Write JSONL to this path instead of stdout");
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
