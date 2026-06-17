pub mod compiler;
pub mod extract;
pub mod parse;
pub mod sidecar;
pub mod timeline;

pub use compiler::{
    EvidenceKind, PreferenceCompiler, PreferenceExample, PreferenceOutcome, EXTRACTOR_VERSION,
    replace_session_preference_examples,
};
pub use extract::{TasteExtractOptions, TasteExtractReport, run_taste_extract};
pub use parse::{
    ClaudeSessionParser, ContentRef, ParseKind, ParsedEvent, SessionEventParser, SourceStream,
    merge_streams, parse_jsonl, renumber_seq,
};
pub use sidecar::{ResolvedContent, SnapshotSidecarResolver};
pub use timeline::{
    FileTimelineMaterializer, FileTimelineRow, TimelineSourceKind, replace_session_file_timelines,
};
