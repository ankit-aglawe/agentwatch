//! Background watcher that keeps the SQLite store fresh while the TUI runs.
//!
//! v0.1 design: poll-based. Every `interval` seconds, the watcher walks all
//! active adapters, runs incremental ingest (source-state skips unchanged
//! files), and writes new events. The TUI's own 2-second SQLite poll picks up
//! the changes naturally — no IPC between watcher and TUI beyond the DB.
//!
//! v0.2 will swap this for `notify`-based filesystem events with debouncing,
//! but the polling layer stays as a fallback for FUSE/Docker bind mounts.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use agentwatch_adapters::{
    claude_code::ClaudeCodeAdapter, claude_desktop::ClaudeDesktopAdapter,
    codex_cli::CodexCliAdapter, cursor::CursorAdapter, gemini_cli::GeminiCliAdapter,
    opencode::OpenCodeAdapter, windsurf::WindsurfAdapter, Adapter, ParseResult,
};
use agentwatch_core::AgentEvent;
use agentwatch_store::{StoreError, Writer};

const BATCH_SIZE: usize = 1000;

/// Per-call ingest summary. Useful for surface UIs that want to show "N new
/// events in the last cycle".
#[derive(Debug, Default, Clone, Copy)]
pub struct IngestStats {
    pub files_total: usize,
    pub files_unchanged: usize,
    pub lines: usize,
    pub events: usize,
    pub skipped: usize,
    pub unknown: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("adapter: {0}")]
    Adapter(#[from] agentwatch_adapters::AdapterError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

fn default_adapters() -> Vec<(String, Box<dyn Adapter>)> {
    vec![
        ("claude-code".into(), Box::new(ClaudeCodeAdapter::new())),
        ("claude-desktop".into(), Box::new(ClaudeDesktopAdapter::new())),
        ("codex-cli".into(), Box::new(CodexCliAdapter::new())),
        ("cursor".into(), Box::new(CursorAdapter::new())),
        ("gemini-cli".into(), Box::new(GeminiCliAdapter::new())),
        ("windsurf".into(), Box::new(WindsurfAdapter::new())),
        ("opencode".into(), Box::new(OpenCodeAdapter::new())),
    ]
}

/// Run one incremental-ingest pass across all adapters (or one filtered).
/// Honors source_state — files unchanged since last call are skipped.
pub fn run_once(
    writer: &mut Writer,
    agent_filter: Option<&str>,
) -> Result<IngestStats, WatchError> {
    let mut adapters = default_adapters();
    if let Some(filter) = agent_filter {
        adapters.retain(|(name, _)| name == filter);
    }

    let mut stats = IngestStats::default();
    let mut batch: Vec<AgentEvent> = Vec::with_capacity(BATCH_SIZE);

    for (name, adapter) in adapters.iter_mut() {
        let sources = adapter.discover_sources()?;
        stats.files_total += sources.len();
        for source in &sources {
            let metadata = match std::fs::metadata(&source.path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let size = metadata.len();
            let path_str = source.path.display().to_string();

            if !writer.source_needs_ingest(name, &path_str, mtime_ms, size)? {
                stats.files_unchanged += 1;
                continue;
            }

            ingest_file(adapter.as_mut(), source, &source.path, &mut batch, writer, &mut stats)?;
            writer.mark_source_ingested(name, &path_str, mtime_ms, size)?;
        }
    }

    if !batch.is_empty() {
        stats.events += writer.insert_batch(&batch)?;
        batch.clear();
    }

    Ok(stats)
}

fn ingest_file(
    adapter: &mut dyn Adapter,
    source: &agentwatch_adapters::SourcePath,
    path: &Path,
    batch: &mut Vec<AgentEvent>,
    writer: &mut Writer,
    stats: &mut IngestStats,
) -> Result<(), WatchError> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let reader = BufReader::new(file);
    let mut offset: u64 = 0;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line_len = line.len() as u64 + 1;
        let parsed = adapter.parse_line(source, &line, offset)?;
        offset += line_len;
        stats.lines += 1;
        for result in parsed {
            match result {
                ParseResult::Event(ev) => {
                    batch.push(ev);
                    if batch.len() >= BATCH_SIZE {
                        stats.events += writer.insert_batch(batch)?;
                        batch.clear();
                    }
                }
                ParseResult::UnknownLine { .. } => stats.unknown += 1,
                ParseResult::Skip { .. } => stats.skipped += 1,
            }
        }
    }
    Ok(())
}

/// Owns the watcher thread. Drop = stop signal + join.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Watcher {
    /// Spawn the watcher thread. It will call `run_once` every `interval`
    /// against the agentwatch SQLite at the default path, ignoring errors so
    /// transient SQLite locks don't kill the watcher.
    pub fn spawn(interval: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let handle = thread::spawn(move || {
            let db = agentwatch_core::paths::db_path();
            // Open the writer once and reuse. If it fails, sleep and retry.
            let mut writer: Option<Writer> = None;
            while !stop_clone.load(Ordering::Relaxed) {
                if writer.is_none() {
                    writer = Writer::open(&db).ok();
                }
                if let Some(w) = writer.as_mut() {
                    let _ = run_once(w, None);
                }
                let start = Instant::now();
                while start.elapsed() < interval {
                    if stop_clone.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
