pub mod compiler;
pub mod extract;
pub mod parse;
pub mod show;
pub mod sidecar;
pub mod status;
pub mod timeline;

pub use compiler::{
    EXTRACTOR_VERSION, EvidenceKind, HIGH_CONFIDENCE_THRESHOLD, PreferenceCompiler,
    PreferenceExample, PreferenceOutcome, replace_session_preference_examples,
};
pub use extract::{TasteExtractOptions, TasteExtractReport, run_taste_extract};
pub use parse::{
    ClaudeSessionParser, ContentRef, ParseKind, ParsedEvent, SessionEventParser, SourceStream,
    merge_streams, parse_jsonl, renumber_seq,
};
pub use show::{
    TasteExampleShowReport, TasteShowExampleOptions, TasteShowTimelineOptions,
    TasteTimelineShowReport, run_taste_show_example, run_taste_show_timeline,
    show_example_for_data_dir, show_timeline_for_data_dir,
};
pub use sidecar::{ResolvedContent, SnapshotSidecarResolver};
pub use status::{
    TasteOutcomeCounts, TasteStatusReport, run_taste_status, taste_status_for_data_dir,
};
pub use timeline::{
    FileTimelineMaterializer, FileTimelineRow, TimelineSourceKind, replace_session_file_timelines,
};
