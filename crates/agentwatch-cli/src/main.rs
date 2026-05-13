//! agentwatch CLI entry point.
//!
//! Dispatches subcommands into the relevant crate. Owns:
//!   - tracing initialization
//!   - config file loading
//!   - top-level error reporting

use anyhow::Result;
use chrono::{Duration, Utc};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "agentwatch",
    version,
    about = "htop for your AI coding agents",
    long_about = "Live, local observability for Claude Code, Cursor, Codex, \
                  Windsurf, OpenCode, Gemini CLI, and Claude Desktop. Reads \
                  session logs your agents already write to disk. Local-first, \
                  no signup, no cloud."
)]
struct Cli {
    /// Use polling instead of filesystem events (for FUSE / Docker bind mounts).
    #[arg(long, global = true)]
    poll: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Boot the local webapp on 127.0.0.1:7878.
    Serve {
        /// Preferred port; auto-increments if taken.
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    /// Populate the SQLite DB with realistic synthetic activity, then launch the TUI.
    Demo {
        /// Deterministic seed. Omit for time-based seed.
        #[arg(long)]
        seed: Option<u64>,
        /// Just generate, don't launch TUI.
        #[arg(long)]
        no_tui: bool,
    },
    /// Plain-text spend report.
    Report {
        #[arg(long, value_enum, default_value_t = Span::Today)]
        since: Span,
    },
    /// Markdown narrative summary.
    Summary {
        #[arg(long, value_enum, default_value_t = Span::Week)]
        span: Span,
        /// Add a 3-sentence narrative via your own SLM API key (env-based).
        #[arg(long)]
        vibe: bool,
    },
    /// Self-contained HTML or JSON dump for any timeframe.
    Export {
        #[arg(long, value_enum, default_value_t = ExportFormat::Html)]
        format: ExportFormat,
        /// Include code content (paths only by default - Invariant #4).
        #[arg(long)]
        with_snippets: bool,
    },
    /// Single snapshot of the current state. No live mode.
    #[command(name = "--once", visible_alias = "once")]
    Once,
    /// Detect installed agents and report capability badges.
    Doctor,
    /// Rebuild SQLite DB from raw log files. Idempotent.
    Repair,
    /// Run the opt-in HTTP proxy mode for direct-API calls.
    Proxy {
        #[arg(long, default_value_t = 7777)]
        port: u16,
        /// Log request/response bodies for 24h. OPT-IN, OFF BY DEFAULT.
        #[arg(long)]
        log_bodies: bool,
    },
    /// One-line status output for tmux / vim / zsh / polybar status bars.
    Status {
        #[arg(long, value_enum, default_value_t = StatusFormat::Compact)]
        format: StatusFormat,
    },
    /// Show or edit configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Show or refresh model pricing data.
    Pricing {
        #[command(subcommand)]
        action: PricingAction,
    },
    /// Scan installed agents' session logs and ingest events into SQLite.
    /// Idempotent - safe to re-run; only new events are added.
    Ingest {
        /// Only ingest from this specific agent (e.g. claude-code).
        #[arg(long)]
        agent: Option<String>,
        /// Print per-file progress.
        #[arg(long)]
        verbose: bool,
    },
    /// Print the agentwatch banner. `--mini` for one-line form.
    Banner {
        #[arg(long)]
        mini: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    Show,
    Plan { spec: String },
    Display {
        #[arg(value_enum)]
        mode: DisplayMode,
    },
    Enable { agent: String },
    Disable { agent: String },
    /// Set spend budgets for API users (today / week / month, in dollars).
    /// Example: agentwatch config budget today=20 week=100 month=400
    Budget {
        /// Key=value pairs, e.g. today=20 week=100 month=400 - or `show` / `clear`.
        specs: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum PricingAction {
    /// Show snapshot date, model count, source, and staleness.
    Show,
    /// List all known model names.
    List,
    /// Download latest LiteLLM pricing to ~/.agentwatch/pricing.json (Day 8 work).
    Refresh,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DisplayMode { Friendly, Technical, Auto }

#[derive(Copy, Clone, Debug, ValueEnum)]
enum StatusFormat { Compact, Ascii, Json }

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Span { Today, Week, Month }

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ExportFormat { Html, Json }

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, FmtSubscriber};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = FmtSubscriber::builder().with_env_filter(filter).try_init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        None => {
            // Default: silent auto-detect, then launch the TUI.
            run_first_run_or_tui()?;
        }
        Some(Command::Serve { port }) => {
            let mut cfg = agentwatch_web::ServeConfig::new();
            cfg.preferred_port = port;
            agentwatch_web::serve(cfg)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        Some(Command::Demo { seed, no_tui }) => {
            run_demo(seed)?;
            if !no_tui {
                agentwatch_tui::run().map_err(|e| anyhow::anyhow!(e.to_string()))?;
            } else {
                println!("(Skipped TUI. Run `agentwatch` to launch it.)");
            }
        }
        Some(Command::Report { since: _ }) => {
            println!("(scaffold) plain-text report");
        }
        Some(Command::Summary { span, vibe }) => {
            let cfg = agentwatch_summary::SummaryConfig {
                span: match span {
                    Span::Today | Span::Week => agentwatch_summary::Span::Week,
                    Span::Month => agentwatch_summary::Span::Month,
                },
                vibe,
            };
            let md = agentwatch_summary::render(cfg)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            print!("{md}");
        }
        Some(Command::Export { format: _, with_snippets }) => {
            if with_snippets {
                eprintln!("⚠ --with-snippets opted in; export will include code content.");
            }
            println!("(scaffold) export");
        }
        Some(Command::Once) => {
            println!("(scaffold) once snapshot");
        }
        Some(Command::Doctor) => {
            run_doctor();
        }
        Some(Command::Repair) => {
            println!("(scaffold) rebuild DB from raw log files");
        }
        Some(Command::Proxy { port, log_bodies }) => {
            if log_bodies {
                eprintln!("⚠ --log-bodies enabled; bodies stored under ~/.agentwatch/proxy/ with 24h TTL.");
            }
            let cfg = agentwatch_proxy::ProxyConfig { port, log_bodies };
            agentwatch_proxy::run(cfg)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        Some(Command::Status { format }) => {
            run_status(format)?;
        }
        Some(Command::Config { action }) => match action {
            ConfigAction::Show => println!("(scaffold) print ~/.agentwatch/config.toml"),
            ConfigAction::Plan { spec } => println!("(scaffold) set plan: {spec}"),
            ConfigAction::Display { mode } => println!("(scaffold) set display mode: {mode:?}"),
            ConfigAction::Enable { agent } => println!("(scaffold) enable agent: {agent}"),
            ConfigAction::Disable { agent } => println!("(scaffold) disable agent: {agent}"),
            ConfigAction::Budget { specs } => {
                if specs.is_empty() || specs.iter().any(|s| s == "show") {
                    println!("(scaffold) no budgets configured yet. Example:");
                    println!("  agentwatch config budget today=20 week=100 month=400");
                } else if specs.iter().any(|s| s == "clear") {
                    println!("(scaffold) budgets cleared");
                } else {
                    for s in specs {
                        println!("(scaffold) set budget: {s}");
                    }
                }
            }
        },
        Some(Command::Pricing { action }) => run_pricing(action)?,
        Some(Command::Ingest { agent, verbose }) => run_ingest(agent.as_deref(), verbose)?,
        Some(Command::Banner { mini }) => print_banner(mini),
    }
    Ok(())
}

const BANNER_FULL: &str = include_str!("../../../assets/logo.txt");
const BANNER_MINI: &str = include_str!("../../../assets/logo-mini.txt");

// Brand gradient: subtle vertical green sweep, taken from the TUI's "agent
// active" bar color (Catppuccin Mocha Green #a6e3a1). Per-block, bottom-up:
// each contiguous text block (logo, wordmark, tagline) gets its own
// top-to-bottom sweep so the gradient repeats per letter instead of stretching
// across the whole banner.
const GRADIENT_TOP:    (u8, u8, u8) = (0xc4, 0xec, 0xc0); // light green (top of letter)
const GRADIENT_BOTTOM: (u8, u8, u8) = (0x6f, 0xb2, 0x66); // deep green (bottom of letter)
const RESET: &str = "\x1b[0m";

fn print_banner(mini: bool) {
    use std::io::IsTerminal;
    let body = if mini { BANNER_MINI } else { BANNER_FULL };
    if std::io::stdout().is_terminal() {
        print_gradient(body);
    } else {
        // Piped / redirected - emit plain text so logs and tests stay clean.
        print!("{body}");
    }
}

/// Print text with a per-block vertical light-to-dark green gradient.
/// Blocks are separated by blank lines; each block restarts the gradient at
/// its top, giving the impression that every letter is lit from above.
fn print_gradient(body: &str) {
    let mut out = String::new();
    let lines: Vec<&str> = body.split('\n').collect();
    let mut block_start = 0usize;
    let mut i = 0usize;
    while i <= lines.len() {
        let at_end = i == lines.len();
        let is_blank = !at_end && lines[i].trim().is_empty();
        if at_end || is_blank {
            paint_block(&lines[block_start..i], &mut out);
            if !at_end {
                out.push('\n');
            }
            block_start = i + 1;
        }
        i += 1;
    }
    print!("{out}");
}

fn paint_block(block: &[&str], out: &mut String) {
    let n = block.len();
    if n == 0 {
        return;
    }
    for (row, line) in block.iter().enumerate() {
        let t = if n > 1 {
            row as f32 / (n - 1) as f32
        } else {
            0.0
        };
        let (r, g, b) = lerp_rgb(GRADIENT_TOP, GRADIENT_BOTTOM, t);
        // One color per row - every char in this row gets it. This is the
        // bottom-up gradient: row 0 = light (top of letter), row N = dark.
        out.push_str(&format!("\x1b[38;2;{};{};{}m", r, g, b));
        out.push_str(line);
        out.push_str(RESET);
        out.push('\n');
    }
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| -> u8 {
        let xf = x as f32;
        let yf = y as f32;
        (xf + (yf - xf) * t).round().clamp(0.0, 255.0) as u8
    };
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

fn run_ingest(agent_filter: Option<&str>, verbose: bool) -> Result<()> {
    use agentwatch_adapters::{Adapter, ParseResult};
    use std::io::{BufRead, BufReader};

    let db = agentwatch_core::paths::db_path();
    let mut writer = agentwatch_store::Writer::open(&db)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    const BATCH_SIZE: usize = 1000;

    // Build the canonical adapter set. We must call parse_line on a mutable
    // adapter, so spin up fresh instances here (Adapter trait already requires
    // Send; new() is cheap).
    let mut adapters: Vec<(String, Box<dyn Adapter>)> = vec![
        ("claude-code".into(), Box::new(agentwatch_adapters::claude_code::ClaudeCodeAdapter::new())),
        ("claude-desktop".into(), Box::new(agentwatch_adapters::claude_desktop::ClaudeDesktopAdapter::new())),
        ("codex-cli".into(), Box::new(agentwatch_adapters::codex_cli::CodexCliAdapter::new())),
        ("cursor".into(), Box::new(agentwatch_adapters::cursor::CursorAdapter::new())),
        ("gemini-cli".into(), Box::new(agentwatch_adapters::gemini_cli::GeminiCliAdapter::new())),
        ("windsurf".into(), Box::new(agentwatch_adapters::windsurf::WindsurfAdapter::new())),
        ("opencode".into(), Box::new(agentwatch_adapters::opencode::OpenCodeAdapter::new())),
    ];

    if let Some(filter) = agent_filter {
        adapters.retain(|(name, _)| name == filter);
        if adapters.is_empty() {
            anyhow::bail!("unknown agent: {filter}");
        }
    }

    let mut total_files = 0usize;
    let mut total_lines = 0usize;
    let mut total_events = 0usize;
    let mut total_unknown = 0usize;
    let mut total_skipped = 0usize;

    println!();
    println!("agentwatch - ingest");
    println!();

    for (name, adapter) in adapters.iter_mut() {
        let sources = adapter.discover_sources()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if sources.is_empty() {
            println!("  {name:<16} no sources");
            continue;
        }
        let started = std::time::Instant::now();
        let mut agent_lines = 0usize;
        let mut agent_events = 0usize;
        let mut agent_unknown = 0usize;
        let mut agent_skipped = 0usize;

        let mut batch: Vec<agentwatch_core::AgentEvent> = Vec::with_capacity(BATCH_SIZE);
        let mut skipped_unchanged = 0usize;
        for source in &sources {
            // Skip files whose mtime + size haven't changed since last ingest.
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
            let needs = writer.source_needs_ingest(name, &path_str, mtime_ms, size)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            if !needs {
                skipped_unchanged += 1;
                continue;
            }

            let file = match std::fs::File::open(&source.path) {
                Ok(f) => f,
                Err(e) => {
                    if verbose {
                        eprintln!("  skip {} ({e})", source.path.display());
                    }
                    continue;
                }
            };
            let reader = BufReader::new(file);
            let mut offset: u64 = 0;
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                let line_len = line.len() as u64 + 1;
                let parsed = adapter.parse_line(source, &line, offset)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                offset += line_len;
                agent_lines += 1;
                for result in parsed {
                    match result {
                        ParseResult::Event(ev) => {
                            batch.push(ev);
                            if batch.len() >= BATCH_SIZE {
                                let inserted = writer.insert_batch(&batch)
                                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                                agent_events += inserted;
                                batch.clear();
                            }
                        }
                        ParseResult::UnknownLine { .. } => {
                            agent_unknown += 1;
                        }
                        ParseResult::Skip { .. } => {
                            agent_skipped += 1;
                        }
                    }
                }
            }
            if verbose {
                println!(
                    "  {:<16} {} ({} lines)",
                    name,
                    source.path.display(),
                    agent_lines
                );
            }
            writer.mark_source_ingested(name, &path_str, mtime_ms, size)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        // Flush remaining batch.
        if !batch.is_empty() {
            let inserted = writer.insert_batch(&batch)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            agent_events += inserted;
            batch.clear();
        }
        let elapsed = started.elapsed();
        println!(
            "  {:<16} {} files ({} unchanged) · {} lines · {} events · {} skipped · {} unknown · {:.1}s",
            name,
            sources.len(),
            skipped_unchanged,
            agent_lines,
            agent_events,
            agent_skipped,
            agent_unknown,
            elapsed.as_secs_f32(),
        );
        total_files += sources.len();
        total_lines += agent_lines;
        total_events += agent_events;
        total_unknown += agent_unknown;
        total_skipped += agent_skipped;
    }

    println!();
    println!(
        "Done. {total_files} file(s) · {total_lines} line(s) · {total_events} event(s) written · {total_skipped} skipped · {total_unknown} unknown line(s)."
    );
    println!("→ Run `agentwatch status` to see the latest activity.");
    println!();

    Ok(())
}

fn run_pricing(action: PricingAction) -> Result<()> {
    match action {
        PricingAction::Show => {
            let meta = agentwatch_core::pricing::snapshot_meta();
            let stale = agentwatch_core::pricing::snapshot_is_stale(60);
            let n = agentwatch_core::pricing::known_model_count();
            println!();
            println!("agentwatch - pricing snapshot");
            println!();
            println!("  snapshot date:  {}", meta.snapshot_date);
            println!("  source:         {}", meta.source_url);
            println!("  source commit:  {}", meta.source_commit);
            println!("  models known:   {n}");
            if !meta.note.is_empty() {
                println!("  note:           {}", meta.note);
            }
            if stale {
                println!();
                println!("⚠ This snapshot is more than 60 days old. Numbers may be slightly off.");
                println!("  Run `agentwatch pricing refresh` to download the latest LiteLLM data,");
                println!("  or update agentwatch to the latest release.");
            }
            println!();
        }
        PricingAction::List => {
            for m in agentwatch_core::pricing::known_models() {
                println!("{m}");
            }
        }
        PricingAction::Refresh => {
            eprintln!("(Day 8 work) downloads latest LiteLLM pricing to ~/.agentwatch/pricing.json");
            eprintln!("For now, embedded snapshot is current as of:");
            eprintln!("  {}", agentwatch_core::pricing::snapshot_meta().snapshot_date);
        }
    }
    Ok(())
}

fn run_demo(seed: Option<u64>) -> Result<()> {
    let cfg = agentwatch_demo::DemoConfig {
        seed,
        ..Default::default()
    };
    let db = agentwatch_core::paths::db_path();
    let count = agentwatch_demo::populate(&db, cfg)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("Generated {count} synthetic events.");
    println!("→ Run `agentwatch status` for a one-line view.");
    println!("→ Run `agentwatch` (no args) to launch the TUI.");
    Ok(())
}

fn run_doctor() {
    let results = agentwatch_adapters::discover();
    println!();
    println!("agentwatch - agents detected on this machine");
    println!();
    let now = Utc::now();
    for r in &results {
        let path = r
            .session_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(unknown path)".to_string());
        let last = match r.last_activity {
            Some(t) => format_relative(now - t),
            None => "-".to_string(),
        };
        let status = match r.status {
            agentwatch_adapters::DetectionStatus::Active => "active",
            agentwatch_adapters::DetectionStatus::InstalledOnly => "installed, no recent sessions",
            agentwatch_adapters::DetectionStatus::NotDetected => "not detected",
        };
        let cap = r.capability.label();
        println!(
            "  {}  {:<18}  {:<60}",
            r.glyph(),
            r.agent.display_name(),
            path,
        );
        println!(
            "     status: {:<12}  capability: {:<10}  sessions(30d): {:>3}  last: {}",
            status, cap, r.session_count_30d, last
        );
    }
    let active = results
        .iter()
        .filter(|r| r.status == agentwatch_adapters::DetectionStatus::Active)
        .count();
    println!();
    println!("Tracking {active} agent(s) by default. Edit ~/.agentwatch/config.toml to change.");
    println!();
}

fn run_first_run_or_tui() -> Result<()> {
    // Launch the calm front page. The TUI itself handles the "no data yet"
    // state friendly-style, so we don't need to gate on detection here.
    agentwatch_tui::run().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}

fn run_status(format: StatusFormat) -> Result<()> {
    let db = agentwatch_core::paths::db_path();

    // If the DB doesn't exist yet, render the day-zero state without erroring.
    if !db.exists() {
        return print_status_empty(format);
    }

    let reader = match agentwatch_store::Reader::open(&db) {
        Ok(r) => r,
        Err(_) => return print_status_empty(format),
    };

    let latest = reader
        .latest()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let today_events = reader
        .today_event_count()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    match (latest, format) {
        (None, fmt) => print_status_empty(fmt),
        (Some(latest), StatusFormat::Compact) => {
            let age = Utc::now() - latest.timestamp();
            let (verb, target) = latest.verb_and_target();
            let agent_emoji = agent_dot(&latest.agent);
            let status_emoji = "🟢"; // Day 9 work: compute from cap usage.
            let target_str = target.unwrap_or_else(|| "".to_string());
            let suffix = if age < Duration::minutes(2) {
                format!("{} {} {} {}", agent_emoji, short_agent(&latest.agent), verb, target_str)
            } else {
                format!(
                    "{} {} {}",
                    agent_emoji,
                    short_agent(&latest.agent),
                    format_idle(age)
                )
            };
            println!("{} {} ({} today)", status_emoji, suffix.trim(), today_events);
            Ok(())
        }
        (Some(latest), StatusFormat::Ascii) => {
            let age = Utc::now() - latest.timestamp();
            let (verb, target) = latest.verb_and_target();
            let target_str = target.unwrap_or_else(|| "".to_string());
            let suffix = if age < Duration::minutes(2) {
                format!("{} {} {}", short_agent(&latest.agent), verb, target_str)
            } else {
                format!("{} {}", short_agent(&latest.agent), format_idle(age))
            };
            println!("[OK] {} ({} today)", suffix.trim(), today_events);
            Ok(())
        }
        (Some(latest), StatusFormat::Json) => {
            let age = Utc::now() - latest.timestamp();
            let (verb, target) = latest.verb_and_target();
            let obj = serde_json::json!({
                "status": "good",
                "agent": latest.agent,
                "verb": verb,
                "target": target,
                "idle_seconds": age.num_seconds().max(0),
                "today_events": today_events,
                "time_left_human": null,
            });
            println!("{}", obj);
            Ok(())
        }
    }
}

fn print_status_empty(format: StatusFormat) -> Result<()> {
    match format {
        StatusFormat::Compact => println!("🟢 waiting for first event"),
        StatusFormat::Ascii => println!("[?] waiting for first event"),
        StatusFormat::Json => println!(
            r#"{{"status":"unknown","verb":"waiting","target":null,"today_events":0}}"#
        ),
    }
    Ok(())
}

fn agent_dot(agent: &str) -> &'static str {
    // Brand-colored dots for the major adapters. Falls back to a generic dot.
    match agent {
        "Claude Code" | "Claude Desktop" => "🟣",
        "Codex CLI" => "🌊",
        "Cursor" => "🟧",
        "Gemini CLI" => "🔵",
        "Windsurf" => "🟦",
        "OpenCode" => "🟩",
        _ => "●",
    }
}

fn short_agent(agent: &str) -> &str {
    // Two-letter codes for compact status lines.
    match agent {
        "Claude Code" => "CC",
        "Claude Desktop" => "CD",
        "Codex CLI" => "CX",
        "Cursor" => "Cu",
        "Gemini CLI" => "Gm",
        "Windsurf" => "Wf",
        "OpenCode" => "OC",
        other => other,
    }
}

fn format_relative(age: Duration) -> String {
    if age < Duration::seconds(60) {
        format!("{}s ago", age.num_seconds().max(0))
    } else if age < Duration::minutes(60) {
        format!("{}m ago", age.num_minutes())
    } else if age < Duration::hours(24) {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

fn format_idle(age: Duration) -> String {
    if age < Duration::minutes(5) {
        "active just now".to_string()
    } else if age < Duration::hours(1) {
        format!("finished {}m ago", age.num_minutes())
    } else if age < Duration::hours(24) {
        format!("finished {}h ago", age.num_hours())
    } else {
        "idle".to_string()
    }
}
