use rusqlite::{Connection, params};
use std::path::PathBuf;

use crate::storage::{
    DB_FILE_NAME, RAW_CODEC, ZSTD_CODEC, ZSTD_MIN_PAYLOAD_BYTES, decode_event_payload,
    encode_event_payload, open_database, query_one, sqlite_error,
    unresolved_ingest_error_count_from_connection,
};
use crate::{JottraceError, Result, data_dir_from_env};

pub const DEFAULT_COMPACT_BATCH_SIZE: usize = 1_000;
pub const MAX_COMPACT_BATCH_SIZE: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactMode {
    DryRun,
    Apply,
    Vacuum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactOptions {
    pub mode: CompactMode,
    pub batch_size: usize,
}

impl Default for CompactOptions {
    fn default() -> Self {
        Self {
            mode: CompactMode::DryRun,
            batch_size: DEFAULT_COMPACT_BATCH_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactReport {
    pub db_path: PathBuf,
    pub mode: CompactMode,
    pub batch_size: usize,
    pub raw_events_before: u64,
    pub zstd_events_before: u64,
    pub raw_events_after: u64,
    pub zstd_events_after: u64,
    pub unsupported_codec_events: u64,
    pub eligible_raw_events: u64,
    pub converted_events: u64,
    pub skipped_raw_events: u64,
    pub skipped_small_events: u64,
    pub skipped_not_smaller_events: u64,
    pub skipped_round_trip_failed_events: u64,
    pub stored_bytes_before: u64,
    pub stored_bytes_after: u64,
    pub estimated_bytes_saved: u64,
    pub sqlite_reclaimable_bytes_before: u64,
    pub sqlite_reclaimable_bytes: u64,
    pub unresolved_ingest_errors: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct EventStorageStats {
    raw_events: u64,
    zstd_events: u64,
    unsupported_codec_events: u64,
    stored_bytes: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RawPayloadAnalysis {
    eligible_events: u64,
    skipped_small_events: u64,
    skipped_not_smaller_events: u64,
    skipped_round_trip_failed_events: u64,
    estimated_bytes_saved: u64,
    converted_events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventKey {
    session_id: i64,
    generation: i64,
    seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawEvent {
    key: EventKey,
    payload: Vec<u8>,
}

type ApplyBatchFn =
    fn(&std::path::Path, &mut Connection, Vec<CompactionUpdate>) -> Result<AppliedBatch>;

pub fn run_compact(options: CompactOptions) -> Result<CompactReport> {
    run_compact_with_diagnostics(options, true)
}

pub fn run_compact_with_diagnostics(
    options: CompactOptions,
    include_diagnostics: bool,
) -> Result<CompactReport> {
    validate_compact_options(options)?;

    let data_dir = data_dir_from_env()?;
    let _lock = match options.mode {
        CompactMode::DryRun => None,
        CompactMode::Apply | CompactMode::Vacuum => Some(crate::acquire_data_lock(&data_dir)?),
    };
    let db_path = data_dir.join(DB_FILE_NAME);
    let mut conn = open_database(&db_path)?;
    let before = if include_diagnostics {
        event_storage_stats(&db_path, &conn)?
    } else {
        EventStorageStats::default()
    };
    let sqlite_reclaimable_bytes_before = if include_diagnostics {
        sqlite_reclaimable_bytes(&db_path, &conn)?
    } else {
        0
    };
    let (raw_analysis, after) = match options.mode {
        CompactMode::DryRun => {
            let analysis =
                scan_raw_payload_candidates(&db_path, &mut conn, options.batch_size, None)?;
            let after = if include_diagnostics {
                compacted_storage_estimate(&before, analysis.eligible_events, &analysis)
            } else {
                EventStorageStats::default()
            };
            (analysis, after)
        }
        CompactMode::Apply => {
            let analysis = scan_raw_payload_candidates(
                &db_path,
                &mut conn,
                options.batch_size,
                Some(apply_update_batch),
            )?;
            let after = if include_diagnostics {
                compacted_storage_estimate(&before, analysis.converted_events, &analysis)
            } else {
                EventStorageStats::default()
            };
            (analysis, after)
        }
        CompactMode::Vacuum => {
            conn.execute_batch("VACUUM;")
                .map_err(|source| sqlite_error(&db_path, source))?;
            (RawPayloadAnalysis::default(), before)
        }
    };
    let sqlite_reclaimable_bytes = if include_diagnostics || options.mode != CompactMode::DryRun {
        sqlite_reclaimable_bytes(&db_path, &conn)?
    } else {
        0
    };
    let unresolved_ingest_errors = unresolved_ingest_error_count_from_connection(&db_path, &conn)?;

    Ok(CompactReport {
        db_path,
        mode: options.mode,
        batch_size: options.batch_size,
        raw_events_before: before.raw_events,
        zstd_events_before: before.zstd_events,
        raw_events_after: after.raw_events,
        zstd_events_after: after.zstd_events,
        unsupported_codec_events: before.unsupported_codec_events,
        eligible_raw_events: raw_analysis.eligible_events,
        converted_events: raw_analysis.converted_events,
        skipped_raw_events: raw_analysis.skipped_events(),
        skipped_small_events: raw_analysis.skipped_small_events,
        skipped_not_smaller_events: raw_analysis.skipped_not_smaller_events,
        skipped_round_trip_failed_events: raw_analysis.skipped_round_trip_failed_events,
        stored_bytes_before: before.stored_bytes,
        stored_bytes_after: after.stored_bytes,
        estimated_bytes_saved: raw_analysis.estimated_bytes_saved,
        sqlite_reclaimable_bytes_before,
        sqlite_reclaimable_bytes,
        unresolved_ingest_errors,
    })
}

fn compacted_storage_estimate(
    before: &EventStorageStats,
    converted_events: u64,
    analysis: &RawPayloadAnalysis,
) -> EventStorageStats {
    EventStorageStats {
        raw_events: before.raw_events.saturating_sub(converted_events),
        zstd_events: before.zstd_events + converted_events,
        unsupported_codec_events: before.unsupported_codec_events,
        stored_bytes: before
            .stored_bytes
            .saturating_sub(analysis.estimated_bytes_saved),
    }
}

fn validate_compact_options(options: CompactOptions) -> Result<()> {
    if (1..=MAX_COMPACT_BATCH_SIZE).contains(&options.batch_size) {
        return Ok(());
    }

    Err(JottraceError::InvalidCompactBatchSize {
        batch_size: options.batch_size,
        max: MAX_COMPACT_BATCH_SIZE,
    })
}

impl RawPayloadAnalysis {
    fn skipped_events(&self) -> u64 {
        self.skipped_small_events
            + self.skipped_not_smaller_events
            + self.skipped_round_trip_failed_events
    }
}

fn scan_raw_payload_candidates(
    path: &std::path::Path,
    conn: &mut Connection,
    batch_size: usize,
    apply_batch: Option<ApplyBatchFn>,
) -> Result<RawPayloadAnalysis> {
    let mut cursor = None;
    let mut analysis = RawPayloadAnalysis {
        skipped_small_events: count_small_raw_events(path, conn)?,
        ..RawPayloadAnalysis::default()
    };

    loop {
        let events = raw_event_batch(path, conn, cursor, batch_size)?;
        let Some(last) = events.last() else {
            break;
        };
        cursor = Some(last.key);

        let mut updates = Vec::new();
        for event in events {
            match compact_raw_payload(&event.payload)? {
                RawPayloadPlan::Compact {
                    payload,
                    payload_size,
                    saved_bytes,
                } => {
                    analysis.eligible_events += 1;
                    updates.push(CompactionUpdate {
                        key: event.key,
                        payload,
                        payload_size,
                        saved_bytes,
                    });
                }
                RawPayloadPlan::SkipNotSmaller => analysis.skipped_not_smaller_events += 1,
                RawPayloadPlan::SkipRoundTripFailed => {
                    analysis.skipped_round_trip_failed_events += 1;
                }
            }
        }

        match apply_batch {
            Some(apply_batch) => {
                let applied = apply_batch(path, conn, updates)?;
                analysis.converted_events += applied.converted_events;
                analysis.estimated_bytes_saved += applied.saved_bytes;
            }
            None => {
                analysis.estimated_bytes_saved +=
                    updates.iter().map(|update| update.saved_bytes).sum::<u64>();
            }
        }
    }

    Ok(analysis)
}

fn apply_update_batch(
    path: &std::path::Path,
    conn: &mut Connection,
    updates: Vec<CompactionUpdate>,
) -> Result<AppliedBatch> {
    if updates.is_empty() {
        return Ok(AppliedBatch::default());
    }

    let tx = conn
        .transaction()
        .map_err(|source| sqlite_error(path, source))?;
    let mut converted_events = 0;
    let mut saved_bytes = 0;
    {
        let mut statement = tx
            .prepare(
                "UPDATE events
                 SET payload = ?1,
                     codec = ?2,
                     payload_size = ?3
                 WHERE session_id = ?4
                   AND generation = ?5
                   AND seq = ?6
                   AND codec = ?7",
            )
            .map_err(|source| sqlite_error(path, source))?;
        for update in updates {
            let updated = statement
                .execute(params![
                    update.payload,
                    ZSTD_CODEC,
                    update.payload_size as i64,
                    update.key.session_id,
                    update.key.generation,
                    update.key.seq,
                    RAW_CODEC,
                ])
                .map_err(|source| sqlite_error(path, source))?;
            if updated > 0 {
                converted_events += updated as u64;
                saved_bytes += update.saved_bytes;
            }
        }
    }
    tx.commit().map_err(|source| sqlite_error(path, source))?;
    Ok(AppliedBatch {
        converted_events,
        saved_bytes,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct AppliedBatch {
    converted_events: u64,
    saved_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RawPayloadPlan {
    Compact {
        payload: Vec<u8>,
        payload_size: usize,
        saved_bytes: u64,
    },
    SkipNotSmaller,
    SkipRoundTripFailed,
}

fn compact_raw_payload(payload: &[u8]) -> Result<RawPayloadPlan> {
    let encoded = encode_event_payload(payload)?;
    if encoded.codec == RAW_CODEC {
        return Ok(RawPayloadPlan::SkipNotSmaller);
    }

    if decode_event_payload(&encoded.payload, encoded.codec)? != payload {
        return Ok(RawPayloadPlan::SkipRoundTripFailed);
    }

    let saved_bytes = (payload.len() - encoded.payload.len()) as u64;
    Ok(RawPayloadPlan::Compact {
        payload: encoded.payload,
        payload_size: encoded.payload_size,
        saved_bytes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactionUpdate {
    key: EventKey,
    payload: Vec<u8>,
    payload_size: usize,
    saved_bytes: u64,
}

fn raw_event_batch(
    path: &std::path::Path,
    conn: &Connection,
    cursor: Option<EventKey>,
    batch_size: usize,
) -> Result<Vec<RawEvent>> {
    let batch_size = batch_size as i64;
    let sql = match cursor {
        Some(_) => {
            "SELECT session_id, generation, seq, payload
             FROM events
             WHERE codec = ?1
               AND payload_size >= ?2
               AND (
                   session_id > ?3
                   OR (session_id = ?3 AND generation > ?4)
                   OR (session_id = ?3 AND generation = ?4 AND seq > ?5)
               )
             ORDER BY session_id, generation, seq
             LIMIT ?6"
        }
        None => {
            "SELECT session_id, generation, seq, payload
             FROM events
             WHERE codec = ?1
               AND payload_size >= ?2
             ORDER BY session_id, generation, seq
             LIMIT ?3"
        }
    };
    let mut statement = conn
        .prepare(sql)
        .map_err(|source| sqlite_error(path, source))?;
    let mut rows = match cursor {
        Some(cursor) => statement
            .query(params![
                RAW_CODEC,
                ZSTD_MIN_PAYLOAD_BYTES as i64,
                cursor.session_id,
                cursor.generation,
                cursor.seq,
                batch_size
            ])
            .map_err(|source| sqlite_error(path, source))?,
        None => statement
            .query(params![
                RAW_CODEC,
                ZSTD_MIN_PAYLOAD_BYTES as i64,
                batch_size
            ])
            .map_err(|source| sqlite_error(path, source))?,
    };
    let mut events = Vec::new();
    while let Some(row) = rows.next().map_err(|source| sqlite_error(path, source))? {
        events.push(RawEvent {
            key: EventKey {
                session_id: row.get(0).map_err(|source| sqlite_error(path, source))?,
                generation: row.get(1).map_err(|source| sqlite_error(path, source))?,
                seq: row.get(2).map_err(|source| sqlite_error(path, source))?,
            },
            payload: row.get(3).map_err(|source| sqlite_error(path, source))?,
        });
    }
    Ok(events)
}

fn count_small_raw_events(path: &std::path::Path, conn: &Connection) -> Result<u64> {
    let count: i64 = query_one(
        path,
        conn,
        "SELECT COUNT(*)
             FROM events
             WHERE codec = ?1
               AND payload_size < ?2",
        params![RAW_CODEC, ZSTD_MIN_PAYLOAD_BYTES as i64],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}

fn event_storage_stats(path: &std::path::Path, conn: &Connection) -> Result<EventStorageStats> {
    let (raw_events, zstd_events, unsupported_codec_events, stored_bytes): (i64, i64, i64, i64) =
        query_one(
            path,
            conn,
            "SELECT
                COALESCE(SUM(CASE WHEN codec = ?1 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN codec = ?2 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN codec NOT IN (?1, ?2) THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(length(payload)), 0)
             FROM events",
            params![RAW_CODEC, ZSTD_CODEC],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

    Ok(EventStorageStats {
        raw_events: raw_events as u64,
        zstd_events: zstd_events as u64,
        unsupported_codec_events: unsupported_codec_events as u64,
        stored_bytes: stored_bytes as u64,
    })
}

fn sqlite_reclaimable_bytes(path: &std::path::Path, conn: &Connection) -> Result<u64> {
    let page_size: i64 = query_one(path, conn, "PRAGMA page_size", [], |row| row.get(0))?;
    let freelist_count: i64 = query_one(path, conn, "PRAGMA freelist_count", [], |row| row.get(0))?;
    Ok((page_size as u64).saturating_mul(freelist_count as u64))
}
