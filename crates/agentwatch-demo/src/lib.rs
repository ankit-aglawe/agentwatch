//! Synthetic-data generator for `agentwatch demo` (cherry-pick E1).
//!
//! First-run UX matters most for tweet-driven landings. Users who run
//! `agentwatch` on day-zero with no agent history need to see the product
//! working in <1s. `demo` populates the SQLite DB with realistic synthetic
//! activity so any reader can `agentwatch demo && agentwatch status` and see
//! the live product immediately.
//!
//! Determinism: `--seed N` produces identical event streams across runs. CI
//! snapshot tests pin a seed; the user-facing default uses a time-based seed.

use std::path::Path;

use agentwatch_core::{Agent, AgentEvent, EventKind, Microcents, SessionEndReason};
use agentwatch_store::Writer;
use chrono::{Duration, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DemoError {
    #[error("store: {0}")]
    Store(#[from] agentwatch_store::StoreError),
}

pub struct DemoConfig {
    /// Seed for the RNG. None → time-based seed (different every run).
    pub seed: Option<u64>,
    /// How far back to span the synthetic events (default: last 8 hours).
    pub span: Duration,
    /// Approximate events per minute (default: 4).
    pub events_per_minute: u32,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            seed: None,
            span: Duration::hours(8),
            events_per_minute: 4,
        }
    }
}

/// Generate synthetic events and write them to the SQLite DB at `db_path`.
/// Returns the number of events written.
pub fn populate(db_path: &Path, config: DemoConfig) -> Result<usize, DemoError> {
    let writer = Writer::open(db_path)?;
    let events = generate_events(&config);
    let count = events.len();
    for event in events {
        writer.insert(&event)?;
    }
    Ok(count)
}

/// Generate the synthetic event stream without writing anything.
/// Pure function - testable in isolation, deterministic with a seed.
pub fn generate_events(config: &DemoConfig) -> Vec<AgentEvent> {
    let mut rng = match config.seed {
        Some(s) => fastrand::Rng::with_seed(s),
        None => fastrand::Rng::new(),
    };

    let mut out = Vec::new();
    let start = Utc::now() - config.span;
    let total_minutes = config.span.num_minutes().max(1) as u64;
    let target_events =
        (total_minutes * config.events_per_minute as u64).max(20) as usize;

    // ~4 sessions over the span, mixed across agents.
    let agent_mix = [
        Agent::ClaudeCode,
        Agent::ClaudeCode,
        Agent::ClaudeCode,
        Agent::CodexCli,
        Agent::Cursor,
    ];

    let session_count = 4;
    let session_len = target_events / session_count;
    for session_idx in 0..session_count {
        let agent = agent_mix[rng.usize(..agent_mix.len())];
        let session_id = format!("sess-{}-{}", session_idx, rng.u64(..));
        let session_start =
            start + Duration::minutes((session_idx as i64) * (total_minutes as i64) / 4);
        let model = pick_model(&mut rng, agent);
        let project_dir = pick_project_dir(&mut rng);
        let project_basename = project_dir
            .rsplit('/')
            .next()
            .map(|s| s.to_string());

        // SessionStart
        out.push(AgentEvent {
            id: Uuid::new_v4(),
            agent,
            session_id: session_id.clone(),
            timestamp: session_start,
            kind: EventKind::SessionStart {
                model: model.to_string(),
                project_dir: Some(project_dir.to_string()),
            },
            project: project_basename.clone(),
            source_offset: None,
            raw: None,
        });

        // Events within the session
        for i in 0..session_len {
            let t = session_start + Duration::seconds((i * 30) as i64);
            out.push(synth_event(&mut rng, agent, &session_id, t, model, project_basename.clone()));
        }

        // SessionEnd
        out.push(AgentEvent {
            id: Uuid::new_v4(),
            agent,
            session_id: session_id.clone(),
            timestamp: session_start + Duration::seconds((session_len * 30) as i64),
            kind: EventKind::SessionEnd {
                reason: SessionEndReason::Completed,
            },
            project: project_basename.clone(),
            source_offset: None,
            raw: None,
        });
    }

    out
}

fn synth_event(
    rng: &mut fastrand::Rng,
    agent: Agent,
    session_id: &str,
    t: chrono::DateTime<Utc>,
    model: &'static str,
    project: Option<String>,
) -> AgentEvent {
    // Mix of model calls, tool calls, file edits. Skew toward tool calls because
    // that's what users actually see most.
    let roll = rng.u32(0..100);
    let kind = if roll < 40 {
        EventKind::ToolCall {
            tool: pick_tool(rng).to_string(),
            target: Some(pick_target(rng).to_string()),
            input_size_bytes: Some(rng.u64(500..50_000)),
            result_size_bytes: Some(rng.u64(100..5_000)),
            duration_ms: Some(rng.u64(50..2_000)),
        }
    } else if roll < 70 {
        let path = pick_file(rng).to_string();
        EventKind::FileEdit {
            path,
            lines_added: rng.u32(0..50),
            lines_removed: rng.u32(0..30),
        }
    } else {
        let input_tokens = rng.u64(500..8_000);
        let output_tokens = rng.u64(50..1_000);
        let cache_read = rng.u64(0..2_000);
        let cost = compute_cost(model, input_tokens, output_tokens, cache_read);
        EventKind::ModelCall {
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cache_read_tokens: cache_read,
            cache_write_tokens: 0,
            cost_microcents: cost,
            duration_ms: rng.u64(800..6_000),
        }
    };

    AgentEvent {
        id: Uuid::new_v4(),
        agent,
        session_id: session_id.to_string(),
        timestamp: t,
        kind,
        project,
        source_offset: None,
        raw: None,
    }
}

fn compute_cost(model: &str, input: u64, output: u64, cache_read: u64) -> Microcents {
    agentwatch_core::pricing::lookup(model)
        .map(|p| p.cost(input, output, cache_read, 0))
        .unwrap_or(Microcents::ZERO)
}

fn pick_model(rng: &mut fastrand::Rng, agent: Agent) -> &'static str {
    match agent {
        Agent::ClaudeCode => {
            let opts = ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"];
            opts[rng.usize(..opts.len())]
        }
        Agent::CodexCli => "gpt-5-codex",
        Agent::Cursor => "gpt-4-turbo",
        Agent::Windsurf => "windsurf-large",
        Agent::OpenCode => "opencode-mini",
        Agent::GeminiCli => "gemini-2.5-pro",
        Agent::ClaudeDesktop => "claude-opus-4-7",
        _ => "unknown-model",
    }
}

fn pick_project_dir(rng: &mut fastrand::Rng) -> &'static str {
    let opts = [
        "~/work/web-app",
        "~/work/api-server",
        "~/projects/cli-tool",
        "~/code/ml-pipeline",
    ];
    opts[rng.usize(..opts.len())]
}

fn pick_tool(rng: &mut fastrand::Rng) -> &'static str {
    let opts = ["Read", "Edit", "Bash", "Grep", "Glob", "WebSearch"];
    opts[rng.usize(..opts.len())]
}

fn pick_target(rng: &mut fastrand::Rng) -> &'static str {
    let opts = [
        "src/auth.ts",
        "src/api/users.ts",
        "tests/auth.test.ts",
        "package.json",
        "README.md",
        "npm test",
        "rg useState",
        "src/lib/db.ts",
    ];
    opts[rng.usize(..opts.len())]
}

fn pick_file(rng: &mut fastrand::Rng) -> &'static str {
    let opts = [
        "src/auth.ts",
        "src/api/users.ts",
        "src/lib/db.ts",
        "tests/auth.test.ts",
        "tests/users.test.ts",
        "src/components/Login.tsx",
        "styles/main.css",
        "README.md",
    ];
    opts[rng.usize(..opts.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generate_is_deterministic_with_seed() {
        let cfg1 = DemoConfig {
            seed: Some(42),
            ..Default::default()
        };
        let cfg2 = DemoConfig {
            seed: Some(42),
            ..Default::default()
        };
        let a = generate_events(&cfg1);
        let b = generate_events(&cfg2);

        // Same length, same kinds in order.
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.agent, y.agent);
            assert_eq!(x.session_id, y.session_id);
            assert_eq!(std::mem::discriminant(&x.kind), std::mem::discriminant(&y.kind));
        }
    }

    #[test]
    fn generate_produces_enough_events() {
        let cfg = DemoConfig {
            seed: Some(7),
            span: Duration::hours(8),
            events_per_minute: 4,
        };
        let events = generate_events(&cfg);
        // 8h * 4/min * 60 min = ~1920, divided across 4 sessions plus session boundaries.
        // Allow a generous range.
        assert!(events.len() >= 50, "got {} events", events.len());
    }

    #[test]
    fn populate_writes_to_sqlite() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("demo.sqlite");
        let cfg = DemoConfig {
            seed: Some(123),
            span: Duration::hours(2),
            events_per_minute: 2,
        };
        let count = populate(&db, cfg).unwrap();
        assert!(count > 0);
        assert!(db.exists());
    }
}
