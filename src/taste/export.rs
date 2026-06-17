use rusqlite::{Connection, params};
use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::storage::query_collect;
use crate::{JottraceError, io_error};
use crate::{Result, data_dir_from_env, open_locked_database, private_open_options};

use super::compiler::{EvidenceKind, PreferenceExample, PreferenceOutcome};

const CLAUDE_SOURCE: &str = "claude_cli";

/// Supported export serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TasteExportFormat {
    Jsonl,
}

impl TasteExportFormat {
    pub fn from_cli(value: &str) -> Option<Self> {
        match value {
            "jsonl" => Some(Self::Jsonl),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
        }
    }
}

/// Options for `jottrace taste export`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteExportOptions {
    pub format: TasteExportFormat,
    pub output_path: Option<PathBuf>,
}

/// Summary of an export run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasteExportReport {
    pub db_path: PathBuf,
    pub format: TasteExportFormat,
    pub output_path: Option<PathBuf>,
    pub rows_exported: u64,
}

/// One JSONL export row: the trainer-facing `(context, chosen, rejected)` triple
/// plus provenance fields for filtering low-confidence subclasses.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct TasteExportRecord<'a> {
    context: Option<&'a str>,
    chosen: Option<&'a str>,
    rejected: Option<&'a str>,
    outcome: &'static str,
    confidence: f64,
    tool_use_id: &'a str,
    source_session_id: &'a str,
    file_path: Option<&'a str>,
    tool_name: &'a str,
    evidence_kind: &'static str,
}

/// Export labeled preference examples from the local journal.
pub fn run_taste_export(options: TasteExportOptions) -> Result<TasteExportReport> {
    let data_dir = data_dir_from_env()?;
    taste_export_for_data_dir(&data_dir, options)
}

/// Export labeled preference examples from a specific journal directory (tests).
pub fn taste_export_for_data_dir(
    data_dir: &Path,
    options: TasteExportOptions,
) -> Result<TasteExportReport> {
    let (db_path, _lock, conn) = open_locked_database(data_dir)?;
    let examples = load_preference_examples(&db_path, &conn)?;
    let rows_exported = u64::try_from(examples.len()).expect("row count fits in u64");
    write_export(&examples, &options)?;
    Ok(TasteExportReport {
        db_path,
        format: options.format,
        output_path: options.output_path,
        rows_exported,
    })
}

fn load_preference_examples(db_path: &Path, conn: &Connection) -> Result<Vec<PreferenceExample>> {
    query_collect(
        db_path,
        conn,
        "SELECT tool_use_id, source_session_id, generation, proposal_event_seq, file_path,
                tool_name, proposal_content, context, outcome, confidence, evidence_kind,
                extractor_version
         FROM preference_examples
         WHERE source = ?1
         ORDER BY source_session_id ASC, generation ASC, proposal_event_seq ASC",
        params![CLAUDE_SOURCE],
        |row| {
            let tool_use_id: String = row.get(0)?;
            let source_session_id: String = row.get(1)?;
            let generation: i64 = row.get(2)?;
            let proposal_event_seq: i64 = row.get(3)?;
            let outcome: String = row.get(8)?;
            let evidence_kind: String = row.get(10)?;
            Ok(PreferenceExample {
                source: CLAUDE_SOURCE.to_string(),
                source_session_id,
                generation: usize::try_from(generation).expect("generation fits in usize"),
                proposal_event_seq: usize::try_from(proposal_event_seq)
                    .expect("proposal_event_seq fits in usize"),
                tool_use_id,
                file_path: row.get(4)?,
                tool_name: row.get(5)?,
                proposal_content: row.get(6)?,
                context: row.get(7)?,
                outcome: PreferenceOutcome::from_db_str(&outcome).expect("valid outcome"),
                confidence: row.get(9)?,
                evidence_kind: EvidenceKind::from_db_str(&evidence_kind)
                    .expect("valid evidence_kind"),
                extractor_version: row.get(11)?,
            })
        },
    )
}

fn write_export(examples: &[PreferenceExample], options: &TasteExportOptions) -> Result<()> {
    match options.format {
        TasteExportFormat::Jsonl => write_jsonl_export(examples, options.output_path.as_deref()),
    }
}

fn write_jsonl_export(examples: &[PreferenceExample], output_path: Option<&Path>) -> Result<()> {
    let mut buffer = Vec::new();
    for example in examples {
        let record = export_record(example);
        serde_json::to_writer(&mut buffer, &record).map_err(|source| JottraceError::Output {
            source: io::Error::other(source),
        })?;
        buffer.push(b'\n');
    }

    match output_path {
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
            }
            let mut file = private_open_options()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|source| io_error(path, source))?;
            file.write_all(&buffer)
                .map_err(|source| JottraceError::Output { source })?;
        }
        None => {
            io::stdout()
                .write_all(&buffer)
                .map_err(|source| JottraceError::Output { source })?;
        }
    }

    Ok(())
}

fn export_record(example: &PreferenceExample) -> TasteExportRecord<'_> {
    let proposal = example.proposal_content.as_deref();
    let (chosen, rejected) = match example.outcome {
        PreferenceOutcome::Accepted | PreferenceOutcome::Edited => (proposal, None),
        PreferenceOutcome::Rejected => (None, proposal),
    };

    TasteExportRecord {
        context: example.context.as_deref(),
        chosen,
        rejected,
        outcome: example.outcome.as_str(),
        confidence: example.confidence,
        tool_use_id: &example.tool_use_id,
        source_session_id: &example.source_session_id,
        file_path: example.file_path.as_deref(),
        tool_name: &example.tool_name,
        evidence_kind: example.evidence_kind.as_str(),
    }
}
