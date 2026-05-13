//! The `AgentEvent` spine.
//!
//! This type is the only contract between adapters (which produce events from
//! agent log files) and consumers (TUI, webapp, storage, summary). Once shipped,
//! breaking changes to this schema require a major version bump.
//!
//! Content-redaction policy (Invariant #4): user/assistant message bodies are
//! intentionally absent from this schema. Only `char_count` is stored. There is
//! no field for the actual text. Adding a `content: String` field requires a
//! security review — it would allow code/prompts to land in exports and DB.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::money::Microcents;

/// A single event emitted by an adapter, normalized across all agents.
///
/// One source line in an agent's log file produces zero or one `AgentEvent`.
/// The `id` is stable per source line (deterministic UUIDv5 from source +
/// offset + line content hash) so re-parsing a file produces the same id and
/// SQLite inserts are idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEvent {
    pub id: Uuid,
    pub agent: Agent,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
    /// Working directory basename — feeds the "Hot projects" rollup.
    /// `None` for adapters that don't expose cwd (rare).
    #[serde(default)]
    pub project: Option<String>,
    /// Byte offset in the source file. Enables `agentwatch repair` to resume
    /// re-parsing from the last successful event.
    pub source_offset: Option<u64>,
    /// Original payload, retained only when `kind == EventKind::Unknown`.
    /// Keeping it always would bloat the DB; keeping it for unknowns gives us
    /// debug data when parsers drift.
    pub raw: Option<serde_json::Value>,
}

/// Which agent produced this event.
///
/// Includes both local agents (read from disk) and API providers (read from
/// the `agentwatch proxy` HTTPS_PROXY mode).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    ClaudeCode,
    ClaudeDesktop,
    CodexCli,
    Cursor,
    GeminiCli,
    Windsurf,
    OpenCode,
    ApiAnthropic,
    ApiOpenAi,
    ApiGoogle,
    ApiOther,
}

impl Agent {
    pub fn display_name(&self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::ClaudeDesktop => "Claude Desktop",
            Agent::CodexCli => "Codex CLI",
            Agent::Cursor => "Cursor",
            Agent::GeminiCli => "Gemini CLI",
            Agent::Windsurf => "Windsurf",
            Agent::OpenCode => "OpenCode",
            Agent::ApiAnthropic => "Anthropic API",
            Agent::ApiOpenAi => "OpenAI API",
            Agent::ApiGoogle => "Google API",
            Agent::ApiOther => "API (other)",
        }
    }
}

/// What the event represents.
///
/// Variants are deliberately conservative: anything novel that an adapter
/// cannot map cleanly should become `Unknown` with the original payload kept
/// in `AgentEvent::raw`. Adding new variants is a minor version bump; changing
/// existing variants is a major version bump.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    SessionStart {
        model: String,
        project_dir: Option<String>,
    },
    SessionEnd {
        reason: SessionEndReason,
    },
    ModelCall {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        cost_microcents: Microcents,
        duration_ms: u64,
    },
    ToolCall {
        tool: String,
        target: Option<String>,
        input_size_bytes: Option<u64>,
        result_size_bytes: Option<u64>,
        duration_ms: Option<u64>,
    },
    FileEdit {
        path: String,
        lines_added: u32,
        lines_removed: u32,
    },
    /// Body intentionally absent. See module-level docs.
    UserMessage {
        char_count: u32,
    },
    /// Body intentionally absent. See module-level docs.
    AssistantMessage {
        char_count: u32,
    },
    /// Adapter could not map this line. Original payload is in `AgentEvent::raw`.
    Unknown {
        adapter_version_hint: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    Completed,
    Interrupted,
    Crashed,
    Timeout,
    Unknown,
}

/// What this adapter can extract from its agent.
///
/// Surfaced in `agentwatch doctor`, the TUI footer, the webapp header, and the
/// README badge table (auto-generated via `xtask gen-readme-badges`).
///
/// Capability is produced by adapters (compile-time constants) and consumed by
/// surfaces (TUI/webapp/doctor). It is never deserialized — the only direction
/// is binary → JSON for the webapp. That lets us use `&'static [&'static str]`
/// for the `missing` slice without heap allocation per adapter.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Tokens, tool calls, file edits, model.
    Full,
    /// Partial — see `missing` for what isn't captured.
    Partial {
        missing: &'static [&'static str],
    },
    /// Only the model name; no tokens, no tool calls.
    ModelOnly,
}

impl Capability {
    pub fn label(&self) -> &'static str {
        match self {
            Capability::Full => "full",
            Capability::Partial { .. } => "partial",
            Capability::ModelOnly => "model only",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_event() -> AgentEvent {
        AgentEvent {
            id: Uuid::nil(),
            agent: Agent::ClaudeCode,
            session_id: "sess-123".into(),
            timestamp: Utc.with_ymd_and_hms(2026, 5, 12, 12, 0, 0).unwrap(),
            kind: EventKind::ModelCall {
                model: "claude-opus-4-7".into(),
                input_tokens: 1000,
                output_tokens: 200,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_microcents: Microcents::from_cents(150),
                duration_ms: 3400,
            },
            project: None,
            source_offset: Some(2048),
            raw: None,
        }
    }

    #[test]
    fn agent_event_serde_roundtrip() {
        let event = sample_event();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn unknown_event_retains_raw() {
        let payload = serde_json::json!({ "novel_field": "future" });
        let event = AgentEvent {
            id: Uuid::nil(),
            agent: Agent::ClaudeCode,
            session_id: "s".into(),
            timestamp: Utc::now(),
            kind: EventKind::Unknown {
                adapter_version_hint: Some("claude-code v2.0".into()),
            },
            project: None,
            source_offset: None,
            raw: Some(payload.clone()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.raw, Some(payload));
    }

    #[test]
    fn agent_display_names_stable() {
        assert_eq!(Agent::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(Agent::CodexCli.display_name(), "Codex CLI");
        assert_eq!(Agent::Cursor.display_name(), "Cursor");
    }

    #[test]
    fn capability_labels_stable() {
        assert_eq!(Capability::Full.label(), "full");
        assert_eq!(
            Capability::Partial {
                missing: &["tool_calls"]
            }
            .label(),
            "partial"
        );
        assert_eq!(Capability::ModelOnly.label(), "model only");
    }

    #[test]
    fn user_message_has_no_content_field() {
        // Compile-time enforcement of Invariant #4: there is no `content` field
        // on UserMessage. If this test ever fails to compile because someone
        // added one, that change requires a security review.
        let json = serde_json::json!({
            "type": "user_message",
            "char_count": 42
        });
        let kind: EventKind = serde_json::from_value(json).unwrap();
        match kind {
            EventKind::UserMessage { char_count } => assert_eq!(char_count, 42),
            _ => panic!("expected UserMessage"),
        }
    }
}
