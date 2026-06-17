pub mod parse;
pub mod sidecar;

pub use parse::{
    ClaudeSessionParser, ContentRef, ParseKind, ParsedEvent, SessionEventParser, SourceStream,
    merge_streams, parse_jsonl, renumber_seq,
};
pub use sidecar::{ResolvedContent, SnapshotSidecarResolver};
