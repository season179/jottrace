pub mod compiler;
pub mod export;
pub mod extract;
pub mod parse;
pub mod show;
pub mod sidecar;
pub mod status;
pub mod timeline;

/// Defines a database-backed enum whose variants serialize to and from fixed
/// strings, generating the inverse `as_str` / `from_db_str` pair from a single
/// variant-to-string table so the two directions cannot drift apart.
macro_rules! db_string_enum {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            pub fn from_db_str(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}
pub(crate) use db_string_enum;

pub use compiler::{
    EXTRACTOR_VERSION, EvidenceKind, HIGH_CONFIDENCE_THRESHOLD, PreferenceCompiler,
    PreferenceExample, PreferenceOutcome, replace_session_preference_examples,
};
pub use export::{
    TasteExportFormat, TasteExportOptions, TasteExportReport, run_taste_export,
    taste_export_for_data_dir,
};
pub use extract::{
    TasteExtractOptions, TasteExtractReport, run_taste_extract, taste_extract_for_data_dir,
};
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
    TasteEvidenceCounts, TasteOutcomeCounts, TasteStatusReport, run_taste_status,
    taste_status_for_data_dir,
};
pub use timeline::{
    FileTimelineMaterializer, FileTimelineRow, TimelineSourceKind, replace_session_file_timelines,
};
