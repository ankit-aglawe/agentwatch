//! Claude Code adapter.
//!
//! Source: `~/.claude/projects/<project>/<session-id>.jsonl` (and
//! `~/.claude/projects/<project>/<session-id>/subagents/agent-*.jsonl` for
//! sub-agent sessions). One JSON object per line.
//!
//! Schema observed in production logs (Claude Code 2.1.x):
//!
//! Each line is shaped like:
//! ```jsonc
//! {
//!   "type": "user" | "assistant",
//!   "uuid": "...",
//!   "timestamp": "ISO-8601",
//!   "sessionId": "...",
//!   "cwd": "/project/path",
//!   "version": "2.1.69",
//!   "message": {
//!     "role": "user" | "assistant",
//!     "model": "claude-opus-4-7",         // assistant only
//!     "content": string | [content blocks],
//!     "usage": { ... }                    // assistant only
//!   }
//! }
//! ```
//!
//! Content blocks observed: `text`, `thinking`, `tool_use`, `tool_result`.
//!
//! Capability: full — token counts, tool calls, file edits, model name all present.
//!
//! Streaming contract: parser handles one line at a time. The 136MB session
//! in the wild has tens of thousands of lines; `fs::read_to_string` would OOM.

use agentwatch_core::{Agent, AgentEvent, Capability, EventKind, Microcents};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;
use uuid::Uuid;

use crate::{Adapter, AdapterError, ParseResult, SkipReason, SourcePath};

impl From<AgentEvent> for ParseResult {
    fn from(ev: AgentEvent) -> Self {
        ParseResult::Event(ev)
    }
}

/// Extract the basename of a directory path. Used to convert
/// `/home/user/code/my-service` → `my-service`.
fn project_basename(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|s| s.to_string())
}

pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeCodeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Adapter for ClaudeCodeAdapter {
    fn agent(&self) -> Agent {
        Agent::ClaudeCode
    }
    fn capability(&self) -> Capability {
        Capability::Full
    }
    fn session_root(&self) -> Option<PathBuf> {
        agentwatch_core::paths::home_dir().map(|h| h.join(".claude").join("projects"))
    }

    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError> {
        let mut out = Vec::new();
        let root = match self.session_root() {
            Some(p) if p.exists() => p,
            _ => return Ok(out),
        };
        walk_jsonl(&root, &mut out, 0);
        Ok(out)
    }

    fn parse_line(
        &mut self,
        source: &SourcePath,
        line: &str,
        offset: u64,
    ) -> Result<Vec<ParseResult>, AdapterError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(vec![ParseResult::Skip {
                reason: SkipReason::Blank,
            }]);
        }

        let raw: Line = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => {
                return Ok(vec![ParseResult::UnknownLine {
                    raw: trimmed.to_string(),
                    hint: Some("malformed json".to_string()),
                }]);
            }
        };

        let session_id = match raw.session_id.clone() {
            Some(s) => s,
            None => {
                return Ok(vec![ParseResult::UnknownLine {
                    raw: trimmed.to_string(),
                    hint: Some("missing sessionId".to_string()),
                }]);
            }
        };

        let timestamp = parse_timestamp(raw.timestamp.as_deref()).unwrap_or_else(Utc::now);
        let line_uuid = raw.uuid.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        let version_hint = raw.version.clone();
        // Project resolution priority:
        //   1. Known harness/observer tool detected in the source path
        //   2. The cwd field from the JSONL (real project working directory)
        //   3. "observer" fallback for unknown observer-shaped paths
        //   4. None
        let project = detect_observer_tool(&source.path)
            .map(|t| t.to_string())
            .or_else(|| raw.cwd.as_deref().and_then(project_basename))
            .or_else(|| {
                if looks_like_observer_path(&source.path) {
                    Some("observer".to_string())
                } else {
                    None
                }
            });

        let mut results: Vec<ParseResult> = Vec::new();
        let mut block_index = 0u32;

        match raw.kind.as_deref() {
            Some("user") => {
                let char_count = user_content_chars(&raw.message);
                results.push(AgentEvent {
                    id: stable_id(&line_uuid, block_index),
                    agent: Agent::ClaudeCode,
                    session_id,
                    timestamp,
                    kind: EventKind::UserMessage { char_count },
                    project: project.clone(),
                    source_offset: Some(offset),
                    raw: None,
                }
                .into());
            }
            Some("assistant") => {
                let model = raw
                    .message
                    .as_ref()
                    .and_then(|m| m.model.clone())
                    .unwrap_or_else(|| "unknown-model".to_string());

                // ModelCall from `usage`.
                if let Some(usage) = raw.message.as_ref().and_then(|m| m.usage.as_ref()) {
                    let input = usage.input_tokens.unwrap_or(0);
                    let output = usage.output_tokens.unwrap_or(0);
                    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
                    let cache_write = usage.cache_creation_input_tokens.unwrap_or(0);
                    let cost = agentwatch_core::pricing::lookup(&model)
                        .map(|p| p.cost(input, output, cache_read, cache_write))
                        .unwrap_or(Microcents::ZERO);

                    results.push(ParseResult::Event(AgentEvent {
                        id: stable_id(&line_uuid, block_index),
                        agent: Agent::ClaudeCode,
                        session_id: session_id.clone(),
                        timestamp,
                        kind: EventKind::ModelCall {
                            model: model.clone(),
                            input_tokens: input,
                            output_tokens: output,
                            cache_read_tokens: cache_read,
                            cache_write_tokens: cache_write,
                            cost_microcents: cost,
                            duration_ms: 0, // not exposed in the log
                        },
                        project: project.clone(),
                        source_offset: Some(offset),
                        raw: None,
                    }));
                    block_index += 1;
                }

                // AssistantMessage char count (text blocks only).
                let assistant_char_count = assistant_text_chars(&raw.message);
                if assistant_char_count > 0 {
                    results.push(ParseResult::Event(AgentEvent {
                        id: stable_id(&line_uuid, block_index),
                        agent: Agent::ClaudeCode,
                        session_id: session_id.clone(),
                        timestamp,
                        kind: EventKind::AssistantMessage {
                            char_count: assistant_char_count,
                        },
                        project: project.clone(),
                        source_offset: Some(offset),
                        raw: None,
                    }));
                    block_index += 1;
                }

                // ToolCall + FileEdit per content block.
                if let Some(blocks) = content_blocks(&raw.message) {
                    for blk in blocks {
                        if blk.kind.as_deref() != Some("tool_use") {
                            continue;
                        }
                        let tool = blk.name.clone().unwrap_or_else(|| "Tool".to_string());
                        let target = derive_target(&tool, &blk.input);
                        let input_size = blk
                            .input
                            .as_ref()
                            .map(|v| serde_json::to_string(v).map(|s| s.len() as u64).ok())
                            .and_then(|x| x);

                        results.push(ParseResult::Event(AgentEvent {
                            id: stable_id(&line_uuid, block_index),
                            agent: Agent::ClaudeCode,
                            session_id: session_id.clone(),
                            timestamp,
                            kind: EventKind::ToolCall {
                                tool: tool.clone(),
                                target: target.clone(),
                                input_size_bytes: input_size,
                                result_size_bytes: None,
                                duration_ms: None,
                            },
                            project: project.clone(),
                            source_offset: Some(offset),
                            raw: None,
                        }));
                        block_index += 1;

                        // For Edit/Write tools, also emit a FileEdit event so the
                        // "Today: N files touched" counter is accurate.
                        if (tool == "Edit" || tool == "Write" || tool == "MultiEdit")
                            && target.is_some()
                        {
                            results.push(ParseResult::Event(AgentEvent {
                                id: stable_id(&line_uuid, block_index),
                                agent: Agent::ClaudeCode,
                                session_id: session_id.clone(),
                                timestamp,
                                kind: EventKind::FileEdit {
                                    path: target.unwrap_or_default(),
                                    lines_added: 0,  // not directly given; derive in future
                                    lines_removed: 0,
                                },
                                project: project.clone(),
                                source_offset: Some(offset),
                                raw: None,
                            }));
                            block_index += 1;
                        }
                    }
                }

                if results.is_empty() {
                    // Assistant line with no usage and no content blocks we
                    // recognize — emit Unknown so the user knows we saw something
                    // we didn't fully understand.
                    results.push(ParseResult::Event(AgentEvent {
                        id: stable_id(&line_uuid, block_index),
                        agent: Agent::ClaudeCode,
                        session_id,
                        timestamp,
                        kind: EventKind::Unknown {
                            adapter_version_hint: version_hint,
                        },
                        project: project.clone(),
                        source_offset: Some(offset),
                        raw: Some(serde_json::from_str(trimmed).unwrap_or(serde_json::Value::Null)),
                    }));
                }
            }
            // Known housekeeping line types we deliberately skip — they don't
            // map to user-visible events. Counted as Skip not Unknown so the
            // ingest summary stays honest.
            Some("attachment")
            | Some("last-prompt")
            | Some("permission-mode")
            | Some("queue-operation")
            | Some("file-history-snapshot")
            | Some("system")
            | Some("ai-title")
            | Some("tool_use_result")
            | Some("summary") => {
                results.push(ParseResult::Skip {
                    reason: SkipReason::Comment,
                });
            }
            _ => {
                // Truly unknown line type. Record it so we know if Claude Code
                // introduces a new shape that we should learn to handle.
                results.push(ParseResult::UnknownLine {
                    raw: trimmed.to_string(),
                    hint: version_hint,
                });
            }
        }

        Ok(results)
    }
}

/// Walk a directory recursively, max-depth 5, collecting `.jsonl` files.
/// Includes observer/memory tools (claude-mem etc.) — they use real tokens
/// too, just label them by tool name in the parser.
fn walk_jsonl(dir: &std::path::Path, out: &mut Vec<SourcePath>, depth: u8) {
    if depth > 5 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_jsonl(&path, out, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            out.push(SourcePath { path, session_id });
        }
    }
}

/// Known observer/memory/harness tool names. When a session file lives under
/// a directory matching one of these, we use the tool name as the project
/// label so the user can see which tool is consuming tokens.
const KNOWN_OBSERVER_TOOLS: &[&str] = &[
    "claude-mem",
    "mempalace",
    "agent-shadow-brain",
    "hermes-agent",
    "octopoda",
    "agentrecall",
    "agentmemory",
    "agent-memory",
    "openclaw",
    "agentic-stack",
    "agenthandover",
];

/// Try to identify a known harness/observer tool from the source file path.
/// Returns the canonical tool name if found.
fn detect_observer_tool(source_path: &std::path::Path) -> Option<&'static str> {
    let s = source_path.to_string_lossy().to_ascii_lowercase();
    KNOWN_OBSERVER_TOOLS
        .iter()
        .copied()
        .find(|tool| s.contains(tool))
}

/// Generic fallback for observer-like paths we can't identify by name.
/// Looks for telltale substrings ("observer", "memory", "harness").
fn looks_like_observer_path(source_path: &std::path::Path) -> bool {
    let s = source_path.to_string_lossy().to_ascii_lowercase();
    s.contains("observer-session")
        || s.contains("memory-")
        || s.contains("-harness-")
}

/// Stable per-event ID. Re-parsing the same line + block_index produces the
/// same UUID, so SQLite `INSERT OR IGNORE` deduplicates re-ingest.
fn stable_id(line_uuid: &str, block_index: u32) -> Uuid {
    let key = format!("claude_code:{line_uuid}:{block_index}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes())
}

fn parse_timestamp(s: Option<&str>) -> Option<DateTime<Utc>> {
    let s = s?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn user_content_chars(message: &Option<Message>) -> u32 {
    let msg = match message {
        Some(m) => m,
        None => return 0,
    };
    match &msg.content {
        Some(Content::Text(s)) => s.chars().count() as u32,
        Some(Content::Blocks(blocks)) => blocks
            .iter()
            .filter_map(|b| match b.kind.as_deref() {
                Some("text") => b.text.as_ref().map(|t| t.chars().count() as u32),
                _ => None,
            })
            .sum(),
        None => 0,
    }
}

fn assistant_text_chars(message: &Option<Message>) -> u32 {
    // Identical logic for now; kept separate to make future divergence cheap.
    user_content_chars(message)
}

fn content_blocks(message: &Option<Message>) -> Option<&Vec<ContentBlock>> {
    let msg = message.as_ref()?;
    match &msg.content {
        Some(Content::Blocks(blocks)) => Some(blocks),
        _ => None,
    }
}

fn derive_target(tool: &str, input: &Option<serde_json::Value>) -> Option<String> {
    let input = input.as_ref()?;
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(String::from);
    match tool {
        "Read" | "Edit" | "Write" | "MultiEdit" => s("file_path").or_else(|| s("path")),
        "Bash" => s("command").map(|c| shorten(&c, 80)),
        "Grep" => s("pattern").map(|p| shorten(&p, 80)),
        "Glob" => s("pattern"),
        "WebSearch" | "WebFetch" => s("query").or_else(|| s("url")),
        _ => None,
    }
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

// ---------- JSON schema (subset of what Claude Code writes) ----------

#[derive(Debug, Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    uuid: Option<String>,
    timestamp: Option<String>,
    version: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    content: Option<Content>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_LINE: &str = r#"{"parentUuid":null,"isSidechain":true,"userType":"external","cwd":"/x","sessionId":"sess-1","version":"2.1.62","gitBranch":"master","type":"user","message":{"role":"user","content":"Hello"},"uuid":"u-1","timestamp":"2026-03-02T10:30:16.473Z"}"#;

    const ASSISTANT_WITH_USAGE: &str = r#"{"parentUuid":"u-1","sessionId":"sess-1","version":"2.1.69","message":{"model":"claude-opus-4-7","id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"OK"}],"stop_reason":"end_turn","usage":{"input_tokens":100,"cache_creation_input_tokens":50,"cache_read_input_tokens":1000,"output_tokens":20}},"type":"assistant","uuid":"u-2","timestamp":"2026-03-02T10:30:30.000Z"}"#;

    const ASSISTANT_WITH_TOOL: &str = r#"{"sessionId":"sess-1","version":"2.1.69","message":{"model":"claude-opus-4-7","role":"assistant","content":[{"type":"text","text":"Let me check."},{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/auth.ts","old_string":"x","new_string":"y"}}],"usage":{"input_tokens":50,"output_tokens":5}},"type":"assistant","uuid":"u-3","timestamp":"2026-03-02T10:30:45.000Z"}"#;

    #[test]
    fn user_line_produces_user_message_event() {
        let mut adapter = ClaudeCodeAdapter::new();
        let source = SourcePath {
            path: PathBuf::from("/tmp/test.jsonl"),
            session_id: "sess-1".to_string(),
        };
        let results = adapter.parse_line(&source, USER_LINE, 0).unwrap();
        assert_eq!(results.len(), 1);
        match &results[0] {
            ParseResult::Event(ev) => {
                assert_eq!(ev.agent, Agent::ClaudeCode);
                assert_eq!(ev.session_id, "sess-1");
                match &ev.kind {
                    EventKind::UserMessage { char_count } => assert_eq!(*char_count, 5),
                    other => panic!("expected UserMessage, got {other:?}"),
                }
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn assistant_with_usage_produces_modelcall() {
        let mut adapter = ClaudeCodeAdapter::new();
        let source = SourcePath {
            path: PathBuf::from("/tmp/test.jsonl"),
            session_id: "sess-1".to_string(),
        };
        let results = adapter.parse_line(&source, ASSISTANT_WITH_USAGE, 0).unwrap();
        // Expect ModelCall + AssistantMessage.
        assert!(results.len() >= 1);
        let has_model_call = results.iter().any(|r| {
            matches!(r, ParseResult::Event(ev) if matches!(ev.kind, EventKind::ModelCall { .. }))
        });
        assert!(has_model_call, "expected ModelCall event");

        for r in &results {
            if let ParseResult::Event(ev) = r {
                if let EventKind::ModelCall {
                    model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    cost_microcents,
                    ..
                } = &ev.kind
                {
                    assert_eq!(model, "claude-opus-4-7");
                    assert_eq!(*input_tokens, 100);
                    assert_eq!(*output_tokens, 20);
                    assert_eq!(*cache_read_tokens, 1000);
                    assert_eq!(*cache_write_tokens, 50);
                    // Opus 4.7 rates (microcents/M):
                    //   input:        $15.00/M  =  150_000 µc/M
                    //   output:       $75.00/M  =  750_000 µc/M
                    //   cache_read:   $ 1.50/M  =   15_000 µc/M
                    //   cache_write:  $18.75/M  =  187_500 µc/M
                    // Cost = rate × tokens / 1M:
                    //   input:        150_000 × 100  / 1M = 15 µc
                    //   output:       750_000 × 20   / 1M = 15 µc
                    //   cache_read:    15_000 × 1000 / 1M = 15 µc
                    //   cache_write:  187_500 × 50   / 1M =  9 µc (integer floor)
                    // Total ≈ 54 µc = $0.0054
                    assert!(
                        cost_microcents.as_u64() >= 50 && cost_microcents.as_u64() <= 60,
                        "expected ~54 µc, got {}",
                        cost_microcents.as_u64()
                    );
                }
            }
        }
    }

    #[test]
    fn assistant_with_tool_use_produces_toolcall_and_fileedit() {
        let mut adapter = ClaudeCodeAdapter::new();
        let source = SourcePath {
            path: PathBuf::from("/tmp/test.jsonl"),
            session_id: "sess-1".to_string(),
        };
        let results = adapter.parse_line(&source, ASSISTANT_WITH_TOOL, 0).unwrap();
        let mut saw_tool_call = false;
        let mut saw_file_edit = false;
        for r in &results {
            if let ParseResult::Event(ev) = r {
                match &ev.kind {
                    EventKind::ToolCall { tool, target, .. } => {
                        assert_eq!(tool, "Edit");
                        assert_eq!(target.as_deref(), Some("src/auth.ts"));
                        saw_tool_call = true;
                    }
                    EventKind::FileEdit { path, .. } => {
                        assert_eq!(path, "src/auth.ts");
                        saw_file_edit = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_tool_call, "expected ToolCall");
        assert!(saw_file_edit, "expected FileEdit");
    }

    #[test]
    fn malformed_line_returns_unknown() {
        let mut adapter = ClaudeCodeAdapter::new();
        let source = SourcePath {
            path: PathBuf::from("/tmp/test.jsonl"),
            session_id: "sess-1".to_string(),
        };
        let results = adapter.parse_line(&source, "{not json", 0).unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], ParseResult::UnknownLine { .. }));
    }

    #[test]
    fn blank_line_is_skipped() {
        let mut adapter = ClaudeCodeAdapter::new();
        let source = SourcePath {
            path: PathBuf::from("/tmp/test.jsonl"),
            session_id: "sess-1".to_string(),
        };
        let results = adapter.parse_line(&source, "   \n", 0).unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0],
            ParseResult::Skip {
                reason: SkipReason::Blank
            }
        ));
    }

    #[test]
    fn stable_id_is_deterministic() {
        let a = stable_id("u-1", 0);
        let b = stable_id("u-1", 0);
        let c = stable_id("u-1", 1);
        let d = stable_id("u-2", 0);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }
}
