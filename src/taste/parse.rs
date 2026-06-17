use serde_json::Value;

/// Where a parsed event originated within a Claude session tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStream {
    Parent,
    Subagent { agent_id: String },
}

/// Resolved or referenced file content from a snapshot or tool proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRef {
    Inline(String),
    Sidecar {
        backup_file_name: String,
        version: Option<u64>,
    },
}

/// Normalized event kinds emitted by the Claude parse layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseKind {
    FileSnapshot,
    ToolProposal,
    ToolResult,
    PermissionDenial,
}

/// One normalized row from walking decoded Claude event payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEvent {
    pub seq: usize,
    pub timestamp: Option<String>,
    pub kind: ParseKind,
    pub file_path: Option<String>,
    pub content_or_ref: Option<ContentRef>,
    pub tool_ref: Option<String>,
    pub tool_name: Option<String>,
    pub source_stream: SourceStream,
}

/// Source-specific parser for taste extraction. Claude is the only implementation today.
pub trait SessionEventParser {
    fn source(&self) -> &'static str;

    fn parse_line(
        &self,
        seq: usize,
        source_stream: &SourceStream,
        line: &[u8],
    ) -> Result<Vec<ParsedEvent>, serde_json::Error>;
}

pub struct ClaudeSessionParser;

impl SessionEventParser for ClaudeSessionParser {
    fn source(&self) -> &'static str {
        "claude_cli"
    }

    fn parse_line(
        &self,
        seq: usize,
        source_stream: &SourceStream,
        line: &[u8],
    ) -> Result<Vec<ParsedEvent>, serde_json::Error> {
        parse_claude_line(seq, source_stream, line)
    }
}

/// Parse one JSONL payload line from a Claude session or subagent stream.
pub fn parse_jsonl(
    source_stream: &SourceStream,
    lines: impl IntoIterator<Item = impl AsRef<[u8]>>,
) -> Result<Vec<ParsedEvent>, serde_json::Error> {
    let parser = ClaudeSessionParser;
    let mut events = Vec::new();
    for (seq, line) in lines.into_iter().enumerate() {
        events.extend(parser.parse_line(seq, source_stream, line.as_ref())?);
    }
    Ok(events)
}

/// Merge parent and subagent parsed streams into one parent-attributed timeline.
pub fn merge_streams(mut streams: Vec<(SourceStream, Vec<ParsedEvent>)>) -> Vec<ParsedEvent> {
    let mut merged = Vec::new();
    for (source_stream, mut events) in streams.drain(..) {
        for event in &mut events {
            event.source_stream = source_stream.clone();
        }
        merged.extend(events);
    }

    merged.sort_by(|left, right| {
        timestamp_key(left)
            .cmp(&timestamp_key(right))
            .then_with(|| left.seq.cmp(&right.seq))
    });
    renumber_seq(merged)
}

pub fn renumber_seq(mut events: Vec<ParsedEvent>) -> Vec<ParsedEvent> {
    for (seq, event) in events.iter_mut().enumerate() {
        event.seq = seq;
    }
    events
}

fn timestamp_key(event: &ParsedEvent) -> String {
    event.timestamp.clone().unwrap_or_default()
}

fn parse_claude_line(
    seq: usize,
    source_stream: &SourceStream,
    line: &[u8],
) -> Result<Vec<ParsedEvent>, serde_json::Error> {
    let value: Value = serde_json::from_slice(line)?;
    let timestamp = event_timestamp(&value);
    let event_type = value.get("type").and_then(Value::as_str);

    match event_type {
        Some("file-history-snapshot") => {
            parse_snapshot_events(seq, source_stream, timestamp, &value)
        }
        Some("assistant") => parse_assistant_events(seq, source_stream, timestamp, &value),
        Some("user") => parse_user_events(seq, source_stream, timestamp, &value),
        _ => Ok(Vec::new()),
    }
}

fn event_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("snapshot")
                .and_then(|snapshot| snapshot.get("timestamp"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn parse_snapshot_events(
    seq: usize,
    source_stream: &SourceStream,
    timestamp: Option<String>,
    value: &Value,
) -> Result<Vec<ParsedEvent>, serde_json::Error> {
    let Some(backups) = value
        .get("snapshot")
        .and_then(|snapshot| snapshot.get("trackedFileBackups"))
    else {
        return Ok(Vec::new());
    };

    let mut events = Vec::new();
    match backups {
        Value::Array(entries) => {
            for entry in entries {
                let Some(file_path) = entry.get("filePath").and_then(Value::as_str) else {
                    continue;
                };
                let content = entry
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let entry_timestamp = entry
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| timestamp.clone());
                events.push(ParsedEvent {
                    seq,
                    timestamp: entry_timestamp,
                    kind: ParseKind::FileSnapshot,
                    file_path: Some(file_path.to_string()),
                    content_or_ref: content.map(ContentRef::Inline),
                    tool_ref: None,
                    tool_name: None,
                    source_stream: source_stream.clone(),
                });
            }
        }
        Value::Object(entries) => {
            for (file_path, entry) in entries {
                let backup_file_name = entry
                    .get("backupFileName")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let version = entry.get("version").and_then(Value::as_u64);
                let entry_timestamp = entry
                    .get("backupTime")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| timestamp.clone());
                events.push(ParsedEvent {
                    seq,
                    timestamp: entry_timestamp,
                    kind: ParseKind::FileSnapshot,
                    file_path: Some(file_path.clone()),
                    content_or_ref: backup_file_name.map(|backup_file_name| ContentRef::Sidecar {
                        backup_file_name,
                        version,
                    }),
                    tool_ref: None,
                    tool_name: None,
                    source_stream: source_stream.clone(),
                });
            }
        }
        _ => {}
    }

    Ok(events)
}

fn parse_assistant_events(
    seq: usize,
    source_stream: &SourceStream,
    timestamp: Option<String>,
    value: &Value,
) -> Result<Vec<ParsedEvent>, serde_json::Error> {
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut events = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let tool_ref = block.get("id").and_then(Value::as_str).map(str::to_string);
        let tool_name = block
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let input = block.get("input");
        let file_path = input
            .and_then(|input| input.get("file_path"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let proposal_content = tool_proposal_content(tool_name.as_deref(), input);
        events.push(ParsedEvent {
            seq,
            timestamp: timestamp.clone(),
            kind: ParseKind::ToolProposal,
            file_path,
            content_or_ref: proposal_content,
            tool_ref,
            tool_name,
            source_stream: source_stream.clone(),
        });
    }

    Ok(events)
}

fn tool_proposal_content(tool_name: Option<&str>, input: Option<&Value>) -> Option<ContentRef> {
    let input = input?;
    match tool_name {
        Some("Edit") => input
            .get("new_string")
            .and_then(Value::as_str)
            .map(|content| ContentRef::Inline(content.to_string())),
        Some("Write") | Some("NotebookEdit") => input
            .get("content")
            .or_else(|| input.get("new_source"))
            .and_then(Value::as_str)
            .map(|content| ContentRef::Inline(content.to_string())),
        Some("Bash") => input
            .get("command")
            .and_then(Value::as_str)
            .map(|content| ContentRef::Inline(content.to_string())),
        _ => None,
    }
}

fn parse_user_events(
    seq: usize,
    source_stream: &SourceStream,
    timestamp: Option<String>,
    value: &Value,
) -> Result<Vec<ParsedEvent>, serde_json::Error> {
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut events = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let tool_ref = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let result_text = tool_result_text(block);
        let denied = result_text
            .as_deref()
            .is_some_and(|text| text.contains("new_string was NOT written"));
        events.push(ParsedEvent {
            seq,
            timestamp: timestamp.clone(),
            kind: if denied {
                ParseKind::PermissionDenial
            } else {
                ParseKind::ToolResult
            },
            file_path: None,
            content_or_ref: result_text.map(ContentRef::Inline),
            tool_ref,
            tool_name: None,
            source_stream: source_stream.clone(),
        });
    }

    Ok(events)
}

fn tool_result_text(block: &Value) -> Option<String> {
    match block.get("content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(items)) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text);
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn merge_streams_orders_by_timestamp_then_seq() {
        let parent = vec![ParsedEvent {
            seq: 1,
            timestamp: Some("2026-06-17T10:00:02.000Z".to_string()),
            kind: ParseKind::ToolProposal,
            file_path: None,
            content_or_ref: None,
            tool_ref: Some("parent".to_string()),
            tool_name: None,
            source_stream: SourceStream::Parent,
        }];
        let subagent = vec![ParsedEvent {
            seq: 0,
            timestamp: Some("2026-06-17T10:00:02.500Z".to_string()),
            kind: ParseKind::ToolProposal,
            file_path: None,
            content_or_ref: None,
            tool_ref: Some("sub".to_string()),
            tool_name: None,
            source_stream: SourceStream::Subagent {
                agent_id: "agent-1".to_string(),
            },
        }];

        let merged = merge_streams(vec![
            (SourceStream::Parent, parent),
            (
                SourceStream::Subagent {
                    agent_id: "agent-1".to_string(),
                },
                subagent,
            ),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].tool_ref.as_deref(), Some("parent"));
        assert_eq!(merged[1].tool_ref.as_deref(), Some("sub"));
        assert_eq!(merged[0].seq, 0);
        assert_eq!(merged[1].seq, 1);
    }

    #[test]
    fn parse_snapshot_list_and_dict_shapes() {
        let list_line = br#"{"type":"file-history-snapshot","snapshot":{"trackedFileBackups":[{"filePath":"/tmp/a.rs","content":"inline"}]}}"#;
        let dict_line = br#"{"type":"file-history-snapshot","snapshot":{"trackedFileBackups":{"src/a.rs":{"backupFileName":"blob@v1","version":1}}}}"#;

        let list = parse_claude_line(0, &SourceStream::Parent, list_line).expect("list snapshot");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_path.as_deref(), Some("/tmp/a.rs"));
        assert_eq!(
            list[0].content_or_ref,
            Some(ContentRef::Inline("inline".to_string()))
        );

        let dict = parse_claude_line(1, &SourceStream::Parent, dict_line).expect("dict snapshot");
        assert_eq!(dict.len(), 1);
        assert_eq!(dict[0].file_path.as_deref(), Some("src/a.rs"));
        assert_eq!(
            dict[0].content_or_ref,
            Some(ContentRef::Sidecar {
                backup_file_name: "blob@v1".to_string(),
                version: Some(1),
            })
        );
    }

    #[test]
    fn permission_denial_is_detected_from_tool_result_text() {
        let line = br#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-deny","content":"new_string was NOT written"}]}}"#;
        let events = parse_claude_line(0, &SourceStream::Parent, line).expect("denial");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ParseKind::PermissionDenial);
        assert_eq!(events[0].tool_ref.as_deref(), Some("tool-deny"));
    }

    #[test]
    fn merge_stream_timestamp_ordering_is_stable() {
        let earlier = ParsedEvent {
            seq: 5,
            timestamp: Some("a".to_string()),
            kind: ParseKind::ToolResult,
            file_path: None,
            content_or_ref: None,
            tool_ref: None,
            tool_name: None,
            source_stream: SourceStream::Parent,
        };
        let later = ParsedEvent {
            seq: 1,
            timestamp: Some("b".to_string()),
            kind: ParseKind::ToolResult,
            file_path: None,
            content_or_ref: None,
            tool_ref: None,
            tool_name: None,
            source_stream: SourceStream::Parent,
        };
        let merged = merge_streams(vec![(SourceStream::Parent, vec![later, earlier])]);
        assert_eq!(merged[0].timestamp.as_deref(), Some("a"));
        assert_eq!(merged[1].timestamp.as_deref(), Some("b"));
    }

    #[test]
    fn timestamp_key_handles_missing_timestamp() {
        let event = ParsedEvent {
            seq: 0,
            timestamp: None,
            kind: ParseKind::ToolResult,
            file_path: None,
            content_or_ref: None,
            tool_ref: None,
            tool_name: None,
            source_stream: SourceStream::Parent,
        };
        assert_eq!(timestamp_key(&event), "");
    }

    #[test]
    fn ordering_uses_seq_when_timestamps_match() {
        let left = ParsedEvent {
            seq: 1,
            timestamp: Some("same".to_string()),
            kind: ParseKind::ToolResult,
            file_path: None,
            content_or_ref: None,
            tool_ref: None,
            tool_name: None,
            source_stream: SourceStream::Parent,
        };
        let right = ParsedEvent {
            seq: 2,
            timestamp: Some("same".to_string()),
            kind: ParseKind::ToolResult,
            file_path: None,
            content_or_ref: None,
            tool_ref: None,
            tool_name: None,
            source_stream: SourceStream::Parent,
        };
        assert_eq!(
            timestamp_key(&left).cmp(&timestamp_key(&right)),
            Ordering::Equal
        );
        assert_eq!(left.seq.cmp(&right.seq), Ordering::Less);
    }
}
