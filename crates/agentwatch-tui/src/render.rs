//! Drawing the htop-style front page - Charm-style polish via ratatui's
//! first-class widgets.

use chrono::{DateTime, Duration, Utc};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Axis, Block, BorderType, Borders, Cell, Chart, Dataset, GraphType, Padding, Paragraph,
        Row, Table, Wrap,
    },
    Frame,
};

use crate::app::App;
use agentwatch_store::{
    AgentLiveStatus, AgentSummary, Breakdown, SessionStatus, SessionSummary,
};

// Catppuccin Mocha palette
const MOCHA_TEXT: Color = Color::Rgb(205, 214, 244);
const MOCHA_MUTED: Color = Color::Rgb(108, 112, 134);
const MOCHA_SURFACE: Color = Color::Rgb(69, 71, 90);
const MOCHA_GREEN: Color = Color::Rgb(166, 227, 161);
const MOCHA_BLUE: Color = Color::Rgb(137, 180, 250);
const MOCHA_YELLOW: Color = Color::Rgb(249, 226, 175);
const MOCHA_PEACH: Color = Color::Rgb(250, 179, 135);
const MOCHA_PINK: Color = Color::Rgb(243, 139, 168);
const MOCHA_LAVENDER: Color = Color::Rgb(180, 190, 254);
const MOCHA_TEAL: Color = Color::Rgb(148, 226, 213);
const BRAND_CLAUDE: Color = Color::Rgb(203, 166, 247);

const BORDER: BorderType = BorderType::Rounded;

/// Horizontal gutter (in terminal cells) between adjacent boxes / cells in any
/// horizontal Layout. Define it once here so every row in the TUI shares the
/// same rhythm.
const HSPACING: u16 = 1;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    // Outer frame with live indicators on the right
    let live_indicator = if app.has_activity() && app.metrics.tokens_per_minute > 0 {
        format!(
            " {}/min · {}ms latency ",
            format_tokens(app.metrics.tokens_per_minute),
            app.metrics.avg_latency_ms
        )
    } else {
        " idle ".to_string()
    };
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .title(Span::styled(
            format!(
                " agentwatch · {} ",
                chrono::Local::now().format("%H:%M:%S")
            ),
            Style::default().fg(MOCHA_BLUE).add_modifier(Modifier::BOLD),
        ))
        .title_top(
            Line::from(Span::styled(
                live_indicator,
                Style::default().fg(MOCHA_LAVENDER),
            ))
            .alignment(Alignment::Right),
        )
        .title_bottom(Line::from(Span::styled(
            " F1:Help · F5:Refresh · F10:Quit · q:quit ",
            Style::default().fg(MOCHA_MUTED),
        )))
        .border_style(Style::default().fg(MOCHA_SURFACE))
        .style(Style::default().fg(MOCHA_TEXT));
    f.render_widget(outer, area);

    let inner = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(live_signal_height(app)),  // claude-mem live banner
            Constraint::Length(1),                        // padding above agent bars
            Constraint::Length(agent_bars_height(app)),
            Constraint::Length(1),                        // padding below agent bars
            Constraint::Length(2),                        // today summary
            Constraint::Length(2),                        // power-user metrics
            Constraint::Length(loop_warning_height(app)),
            Constraint::Min(8),                           // sessions table
            Constraint::Length(1),                        // padding below sessions
            Constraint::Length(11),                       // chart row
            Constraint::Length(1),                        // padding below chart row
            Constraint::Length(11),                       // rollup row
        ])
        .split(inner);

    draw_live_banner(f, chunks[0], app);
    draw_agent_bars(f, chunks[2], app);
    draw_today_summary(f, chunks[4], app);
    draw_metrics_line(f, chunks[5], app);
    draw_loop_warning(f, chunks[6], app);
    draw_middle_row(f, chunks[7], app);
    draw_chart_row(f, chunks[9], app);
    draw_rollup_row(f, chunks[11], app);
}

fn live_signal_height(app: &App) -> u16 {
    if app.live.is_some() {
        2
    } else {
        0
    }
}

fn draw_live_banner(f: &mut Frame, area: Rect, app: &App) {
    let Some(live) = &app.live else { return };
    if area.height == 0 {
        return;
    }
    let age_secs = (Utc::now() - live.timestamp).num_seconds().max(0);
    let age = if age_secs < 10 {
        "just now".to_string()
    } else if age_secs < 60 {
        format!("{}s ago", age_secs)
    } else {
        format!("{}m ago", age_secs / 60)
    };
    // Compress the prompt: single line, leading whitespace stripped, truncated
    // to fit the row.
    let max_width = area.width.saturating_sub(20) as usize;
    let prompt = sanitize_prompt(&live.user_prompt, max_width);
    let para = Paragraph::new(Line::from(vec![
        Span::styled("● now ", Style::default().fg(MOCHA_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("({age} · via {}) ", live.source),
            Style::default().fg(MOCHA_MUTED),
        ),
        Span::styled(
            prompt,
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::ITALIC),
        ),
    ]));
    f.render_widget(para, area);
}

fn sanitize_prompt(prompt: &str, max_width: usize) -> String {
    // Single line, collapse internal whitespace, fence-quote, truncate.
    let collapsed: String = prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= max_width.saturating_sub(2) {
        format!("\"{}\"", collapsed)
    } else {
        let take = max_width.saturating_sub(4);
        let mut t: String = collapsed.chars().take(take).collect();
        t.push('…');
        format!("\"{}\"", t)
    }
}

fn draw_middle_row(f: &mut Frame, area: Rect, app: &App) {
    // Sessions ~70%, cost insights ~30%.
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ])
        .spacing(HSPACING)
        .split(area);
    draw_sessions(f, cells[0], app);
    draw_cost_insights(f, cells[1], app);
}

fn draw_cost_insights(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .border_style(Style::default().fg(MOCHA_SURFACE))
        .title(Span::styled(
            " Cost insights ",
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " today vs history ",
            Style::default().fg(MOCHA_MUTED),
        )))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Compute key numbers
    let today_cost = app.daily_costs.first().map(|d| d.cost_microcents.as_u64()).unwrap_or(0);
    let yesterday_cost = app.daily_costs.get(1).map(|d| d.cost_microcents.as_u64()).unwrap_or(0);
    let last_7_avg: u64 = if app.daily_costs.len() > 1 {
        let prior: Vec<u64> = app
            .daily_costs
            .iter()
            .skip(1) // exclude today (partial)
            .take(7)
            .map(|d| d.cost_microcents.as_u64())
            .collect();
        if prior.is_empty() {
            0
        } else {
            prior.iter().sum::<u64>() / prior.len() as u64
        }
    } else {
        0
    };
    let projected = project_today_cost(today_cost);
    let cache_saved = cache_savings_estimate(app);
    let loop_waste = loop_waste_estimate(app);

    // Build lines top-to-bottom
    let mut lines: Vec<Line> = Vec::new();

    // Today big number
    lines.push(Line::from(Span::styled(
        "today",
        Style::default().fg(MOCHA_MUTED),
    )));
    lines.push(Line::from(Span::styled(
        format_cost(today_cost),
        Style::default()
            .fg(MOCHA_TEXT)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Projected
    lines.push(stat_line(
        "projected",
        &format_cost(projected),
        Some(&format!("if you keep pace")),
    ));

    // Comparison vs yesterday
    let (delta_y, delta_y_color) = delta_pct(today_cost, yesterday_cost);
    lines.push(stat_line(
        "yesterday",
        &format_cost(yesterday_cost),
        Some(&format!("{}", delta_y)),
    ));
    let _ = delta_y_color;

    // Comparison vs 7d avg
    let (delta_a, _delta_a_color) = delta_pct(today_cost, last_7_avg);
    lines.push(stat_line(
        "7-day avg",
        &format_cost(last_7_avg),
        Some(&format!("{}", delta_a)),
    ));

    lines.push(Line::from(""));

    lines.push(stat_line(
        "cache saved",
        &format_cost(cache_saved),
        Some("vs no cache"),
    ));
    lines.push(stat_line(
        "loop waste",
        &format_cost(loop_waste),
        Some("est. retries"),
    ));

    lines.push(Line::from(""));

    // 7-day trend sparkline (oldest → newest)
    let max = app
        .daily_costs
        .iter()
        .map(|d| d.cost_microcents.as_u64())
        .max()
        .unwrap_or(1)
        .max(1);
    let sparkline: String = app
        .daily_costs
        .iter()
        .rev()
        .map(|d| sparkline_glyph(d.cost_microcents.as_u64(), max))
        .collect();
    lines.push(Line::from(Span::styled(
        "7-day trend",
        Style::default().fg(MOCHA_MUTED),
    )));
    lines.push(Line::from(Span::styled(
        sparkline,
        Style::default().fg(MOCHA_LAVENDER),
    )));
    let labels = if app.daily_costs.len() >= 7 {
        format!(
            "{}            today",
            app.daily_costs
                .get(6)
                .map(|d| d.date_local.as_str().get(5..10).unwrap_or(""))
                .unwrap_or("")
        )
    } else {
        "older            today".to_string()
    };
    lines.push(Line::from(Span::styled(
        labels,
        Style::default().fg(MOCHA_MUTED),
    )));

    lines.push(Line::from(""));

    // ===== Quotas =====
    lines.push(Line::from(Span::styled(
        "QUOTAS",
        Style::default()
            .fg(MOCHA_MUTED)
            .add_modifier(Modifier::BOLD),
    )));

    // 5-hour window vs plan cap (using the default plan from runway state).
    let plan_cap_5h: u64 = match app.runway.plan {
        crate::runway::Plan::Pro => 44_000,
        crate::runway::Plan::Max5 => 88_000,
        crate::runway::Plan::Max20 => 220_000,
    };
    quota_row(
        &mut lines,
        "5h window",
        app.runway.tokens_5h,
        Some(plan_cap_5h),
        app.runway.plan.label(),
    );
    quota_row(
        &mut lines,
        "today",
        app.today.input_tokens + app.today.output_tokens,
        None,
        "no cap",
    );
    quota_row(&mut lines, "this week", app.week_tokens, None, "rolling 7d");

    lines.push(Line::from(""));

    // Top spender (the priciest session that's still active or recent)
    if let Some(top) = app
        .sessions
        .iter()
        .max_by_key(|s| s.total_cost.as_u64())
    {
        lines.push(Line::from(Span::styled(
            "top session",
            Style::default().fg(MOCHA_MUTED),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                truncate(&top.project, 20),
                Style::default()
                    .fg(MOCHA_TEAL)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format_cost(top.total_cost.as_u64()),
                Style::default().fg(MOCHA_TEXT),
            ),
        ]));
    }

    // Hottest project today
    if let Some(hot) = app.project_breakdown.first() {
        lines.push(Line::from(Span::styled(
            "hottest project",
            Style::default().fg(MOCHA_MUTED),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                truncate(&hot.label, 20),
                Style::default()
                    .fg(MOCHA_PEACH)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format_cost(hot.tokens),
                Style::default().fg(MOCHA_TEXT),
            ),
        ]));
    }

    // Peak day in the last 7
    if let Some(peak) = app
        .daily_costs
        .iter()
        .max_by_key(|d| d.cost_microcents.as_u64())
    {
        if peak.cost_microcents.as_u64() > 0 {
            lines.push(Line::from(Span::styled(
                "peak day (7d)",
                Style::default().fg(MOCHA_MUTED),
            )));
            lines.push(Line::from(vec![
                Span::styled(
                    peak.date_local
                        .get(5..10)
                        .unwrap_or(&peak.date_local)
                        .to_string(),
                    Style::default()
                        .fg(MOCHA_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format_cost(peak.cost_microcents.as_u64()),
                    Style::default().fg(MOCHA_TEXT),
                ),
            ]));
        }
    }

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn stat_line(label: &str, value: &str, hint: Option<&str>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("{:<11}", label),
            Style::default().fg(MOCHA_MUTED),
        ),
        Span::styled(
            format!("{:>10}", value),
            Style::default()
                .fg(MOCHA_TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(h) = hint {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            h.to_string(),
            Style::default().fg(MOCHA_MUTED),
        ));
    }
    Line::from(spans)
}

fn project_today_cost(today_cost: u64) -> u64 {
    // Linear projection from time-of-day. Crude but useful.
    let now = chrono::Local::now();
    let hours_elapsed = now.format("%H").to_string().parse::<u32>().unwrap_or(0) as f64
        + now.format("%M").to_string().parse::<u32>().unwrap_or(0) as f64 / 60.0;
    if hours_elapsed < 0.5 {
        return today_cost; // not enough data to project
    }
    let frac_of_day = hours_elapsed / 24.0;
    (today_cost as f64 / frac_of_day) as u64
}

fn delta_pct(now: u64, baseline: u64) -> (String, Color) {
    if baseline == 0 {
        return ("-".to_string(), MOCHA_MUTED);
    }
    let diff = now as i64 - baseline as i64;
    let pct = (diff as f64 / baseline as f64 * 100.0).round() as i64;
    let arrow = if pct > 5 {
        "↑"
    } else if pct < -5 {
        "↓"
    } else {
        "·"
    };
    let color = if pct > 20 {
        MOCHA_PEACH
    } else if pct < -10 {
        MOCHA_GREEN
    } else {
        MOCHA_MUTED
    };
    (format!("{arrow} {:>3}%", pct.abs()), color)
}

fn cache_savings_estimate(app: &App) -> u64 {
    // Cache reads are typically 0.1× the regular price. Saved $ ≈ 0.9 × cache_read tokens × avg price.
    // We don't have the per-model breakdown of cache reads here, so use an estimate against opus prices.
    // For v0.1 just multiply cache_read_tokens by a rough constant: $1.50/M for opus cache_read
    // savings would be ~$13.50/M (because regular would have been $15/M). Use $10/M as conservative avg.
    let saved_microcents_per_million = 100_000u64; // $10/M
    (app.metrics.cache_read_tokens as u128 * saved_microcents_per_million as u128 / 1_000_000) as u64
}

fn loop_waste_estimate(app: &App) -> u64 {
    app.sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Looping))
        .map(|s| s.total_cost.as_u64() / 4)
        .sum()
}

/// Render one quota row. When `cap` is provided, draws a 14-char bar with
/// percentage and color thresholds. When `cap` is None, just shows the total.
fn quota_row(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    used: u64,
    cap: Option<u64>,
    hint: &str,
) {
    let label_str = format!("{:<10}", label);
    match cap {
        Some(cap_v) if cap_v > 0 => {
            let frac = (used as f64 / cap_v as f64).clamp(0.0, 1.5);
            let bar = horizontal_bar(frac.min(1.0), 14);
            let pct = (frac * 100.0).round() as u16;
            let bar_color = if frac >= 0.95 {
                MOCHA_PINK
            } else if frac >= 0.80 {
                MOCHA_PEACH
            } else if frac >= 0.60 {
                MOCHA_YELLOW
            } else {
                MOCHA_GREEN
            };
            lines.push(Line::from(vec![
                Span::styled(label_str, Style::default().fg(MOCHA_MUTED)),
                Span::styled(bar, Style::default().fg(bar_color)),
                Span::styled(
                    format!(" {:>3}%", pct.min(999)),
                    Style::default()
                        .fg(MOCHA_LAVENDER)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("          ", Style::default()),
                Span::styled(
                    format!("{}/{}", format_tokens(used), format_tokens(cap_v)),
                    Style::default().fg(MOCHA_TEXT),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("({hint})"),
                    Style::default().fg(MOCHA_MUTED),
                ),
            ]));
        }
        _ => {
            lines.push(Line::from(vec![
                Span::styled(label_str, Style::default().fg(MOCHA_MUTED)),
                Span::styled(
                    format!("{:>11}", format_tokens(used)),
                    Style::default()
                        .fg(MOCHA_TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    hint.to_string(),
                    Style::default().fg(MOCHA_MUTED),
                ),
            ]));
        }
    }
}

/// Model context window in tokens. Approximate; sourced from vendor docs.
fn model_context_window(model: &str) -> u64 {
    if model.contains("opus-4-7-1m") {
        1_000_000
    } else if model.contains("opus") || model.contains("sonnet") || model.contains("haiku") {
        200_000
    } else if model.contains("gpt-5") || model.contains("gpt-4o") {
        128_000
    } else if model.contains("o1") || model.contains("o3") {
        200_000
    } else if model.contains("gemini-2.5") {
        2_000_000
    } else if model.contains("gemini") {
        1_000_000
    } else {
        128_000 // sensible default
    }
}

fn short_model(model: &str) -> String {
    // Display-friendly short name: "claude-opus-4-7" → "Opus 4.7".
    if let Some(rest) = model.strip_prefix("claude-") {
        let pretty = rest.replace('-', " ");
        return capitalize_first(&pretty);
    }
    if let Some(rest) = model.strip_prefix("gpt-") {
        return format!("GPT-{rest}");
    }
    if let Some(rest) = model.strip_prefix("gemini-") {
        return format!("Gemini {rest}");
    }
    model.to_string()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn sparkline_glyph(value: u64, max: u64) -> char {
    if max == 0 || value == 0 {
        return '·';
    }
    let frac = (value as f64 / max as f64).clamp(0.0, 1.0);
    let idx = (frac * 8.0).ceil() as usize;
    match idx.min(8) {
        0 => '·',
        1 => '▁',
        2 => '▂',
        3 => '▃',
        4 => '▄',
        5 => '▅',
        6 => '▆',
        7 => '▇',
        _ => '█',
    }
}

fn agent_bars_height(app: &App) -> u16 {
    if app.agents.is_empty() {
        2
    } else {
        (app.agents.len() as u16).min(6) + 1
    }
}

fn loop_warning_height(app: &App) -> u16 {
    if app
        .sessions
        .iter()
        .any(|s| matches!(s.status, SessionStatus::Looping))
    {
        2
    } else {
        0
    }
}

// ============ AGENT BARS ============

fn draw_agent_bars(f: &mut Frame, area: Rect, app: &App) {
    if app.agents.is_empty() {
        let para = Paragraph::new(Line::from(Span::styled(
            "  (no agent activity in the last 5 hours - run agentwatch ingest or use Claude Code)",
            Style::default().fg(MOCHA_MUTED),
        )));
        f.render_widget(para, area);
        return;
    }

    // Stack per-agent rows. Each row uses a horizontal layout so the gauge stretches.
    let constraints: Vec<Constraint> = (0..app.agents.len().min(6))
        .map(|_| Constraint::Length(1))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let max_tokens = app.agents.iter().map(|a| a.tokens_5h).max().unwrap_or(1).max(100_000);

    for (i, agent) in app.agents.iter().take(6).enumerate() {
        draw_agent_row(f, rows[i], agent, max_tokens);
    }
}

fn draw_agent_row(f: &mut Frame, area: Rect, a: &AgentSummary, max_tokens: u64) {
    // Layout: [● Name][bar stretches][pct][tokens][cost][status]
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),  // dot + agent name
            Constraint::Fill(1),     // bar stretches
            Constraint::Length(6),   // percentage
            Constraint::Length(18),  // tokens / 5h (needs room for "347.4k tok/5h")
            Constraint::Length(9),   // cost
            Constraint::Length(10),  // status
        ])
        .spacing(HSPACING)
        .split(area);

    let agent_color = brand_color(&a.agent);
    let label = Paragraph::new(Line::from(vec![
        Span::styled("● ", Style::default().fg(agent_color)),
        Span::styled(
            truncate(&a.agent, 13),
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ),
    ]));
    f.render_widget(label, cells[0]);

    // Thick block bar that stretches to fill the column. The empty portion
    // uses ░ light shade so the bar's full extent is still visible.
    let frac = (a.tokens_5h as f64 / max_tokens.max(1) as f64).clamp(0.0, 1.0);
    let bar_width = cells[1].width as usize;
    let total_eighths = (frac * (bar_width * 8) as f64).round() as usize;
    let full = total_eighths / 8;
    let partial = total_eighths % 8;
    let mut filled_str = String::new();
    for _ in 0..full.min(bar_width) {
        filled_str.push('█');
    }
    if full < bar_width && partial > 0 {
        filled_str.push(eighth_block(partial as u8));
    }
    let consumed_chars = filled_str.chars().count();
    let empty_str: String = "░".repeat(bar_width.saturating_sub(consumed_chars));
    let bar = Paragraph::new(Line::from(vec![
        Span::styled(filled_str, Style::default().fg(agent_color)),
        Span::styled(empty_str, Style::default().fg(MOCHA_SURFACE)),
    ]));
    f.render_widget(bar, cells[1]);

    let pct = Paragraph::new(Line::from(Span::styled(
        format!("{:>3.0}%", frac * 100.0),
        Style::default().fg(MOCHA_MUTED),
    )))
    .alignment(Alignment::Right);
    f.render_widget(pct, cells[2]);

    // "347.4k tokens / 5h window" - explicit so the unit is unambiguous.
    let tokens = Paragraph::new(Line::from(vec![
        Span::styled(
            format_tokens(a.tokens_5h),
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" tok / 5h", Style::default().fg(MOCHA_MUTED)),
    ]))
    .alignment(Alignment::Right);
    f.render_widget(tokens, cells[3]);

    let cost = Paragraph::new(Line::from(Span::styled(
        format_cost(a.cost_5h.as_u64()),
        Style::default().fg(MOCHA_TEXT),
    )))
    .alignment(Alignment::Right);
    f.render_widget(cost, cells[4]);

    let status_color = match a.status {
        AgentLiveStatus::Heavy => MOCHA_PEACH,
        AgentLiveStatus::Active => MOCHA_GREEN,
        AgentLiveStatus::Idle => MOCHA_BLUE,
        AgentLiveStatus::Quiet => MOCHA_LAVENDER,
        AgentLiveStatus::Inactive => MOCHA_MUTED,
    };
    let status = Paragraph::new(Line::from(Span::styled(
        a.status.label(),
        Style::default().fg(status_color).add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Right);
    f.render_widget(status, cells[5]);
}

// ============ TODAY SUMMARY ============

fn draw_today_summary(f: &mut Frame, area: Rect, app: &App) {
    if !app.has_activity() {
        let para = Paragraph::new(Line::from(Span::styled(
            "(no agent activity today)",
            Style::default().fg(MOCHA_MUTED),
        )))
        .alignment(Alignment::Left);
        f.render_widget(para, area);
        return;
    }
    let total_tokens = app.today.input_tokens + app.today.output_tokens;
    let total_cost = app.agents.iter().map(|a| a.cost_5h.as_u64()).sum::<u64>();

    // Spread across 5 evenly-sized cells so the line uses full width.
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .spacing(HSPACING)
        .split(area);

    let stats = [
        ("tokens", format_tokens(total_tokens), "today"),
        ("cost", format_cost(total_cost), "today"),
        ("sessions", format!("{}", app.sessions.len()), "active"),
        ("files", format!("{}", app.today.distinct_files_touched), "touched"),
        ("tools", format!("{}", app.today.tool_calls), "calls"),
    ];
    for (i, (label, value, suffix)) in stats.iter().enumerate() {
        let para = Paragraph::new(vec![
            Line::from(Span::styled(
                label.to_string(),
                Style::default().fg(MOCHA_MUTED),
            )),
            Line::from(vec![
                Span::styled(
                    value.clone(),
                    Style::default()
                        .fg(MOCHA_TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if suffix.is_empty() {
                        String::new()
                    } else {
                        format!(" {suffix}")
                    },
                    Style::default().fg(MOCHA_MUTED),
                ),
            ]),
        ])
        .alignment(Alignment::Left);
        f.render_widget(para, cells[i]);
    }
}

// ============ AI LINGO METRICS LINE ============

fn draw_metrics_line(f: &mut Frame, area: Rect, app: &App) {
    let m = &app.metrics;
    if m.model_calls == 0 {
        let para = Paragraph::new(Line::from(Span::styled(
            "(no model calls in the last 30 minutes)",
            Style::default().fg(MOCHA_MUTED),
        )));
        f.render_widget(para, area);
        return;
    }

    let cache_color = if m.cache_hit_rate >= 50.0 {
        MOCHA_GREEN
    } else if m.cache_hit_rate >= 20.0 {
        MOCHA_YELLOW
    } else {
        MOCHA_PEACH
    };
    let lat_color = if m.avg_latency_ms == 0 {
        MOCHA_MUTED
    } else if m.avg_latency_ms < 1500 {
        MOCHA_GREEN
    } else if m.avg_latency_ms < 4000 {
        MOCHA_YELLOW
    } else {
        MOCHA_PEACH
    };
    let lat_display = if m.avg_latency_ms == 0 {
        "-".to_string()
    } else {
        format!("{}ms", m.avg_latency_ms)
    };
    let io = m.io_ratio_output_per_input;
    let io_label = if io < 0.05 {
        "context-heavy"
    } else if io < 0.2 {
        "balanced"
    } else {
        "output-heavy"
    };

    // Two compact lines instead of five sparse cells.
    // Line 1: burn rate + cache stats (read/write separately).
    // Line 2: i/o ratio + latency + model-call count.
    let line1 = Line::from(vec![
        Span::styled("burn ", Style::default().fg(MOCHA_MUTED)),
        Span::styled(
            format!("{}/min", format_tokens(m.tokens_per_minute)),
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({}/sec)", format_tokens(m.tokens_per_second)),
            Style::default().fg(MOCHA_MUTED),
        ),
        Span::raw("    "),
        Span::styled("cache ", Style::default().fg(MOCHA_MUTED)),
        Span::styled(
            format!("{:.0}%", m.cache_hit_rate),
            Style::default().fg(cache_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " (read {} · write {})",
                format_tokens(m.cache_read_tokens),
                format_tokens(m.cache_write_tokens),
            ),
            Style::default().fg(MOCHA_MUTED),
        ),
    ]);
    let line2 = Line::from(vec![
        Span::styled("i/o ", Style::default().fg(MOCHA_MUTED)),
        Span::styled(
            format!("1:{:.2}", io),
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({io_label})"),
            Style::default().fg(MOCHA_MUTED),
        ),
        Span::raw("    "),
        Span::styled("latency ", Style::default().fg(MOCHA_MUTED)),
        Span::styled(
            lat_display,
            Style::default().fg(lat_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::styled("calls ", Style::default().fg(MOCHA_MUTED)),
        Span::styled(
            format!("{}", m.model_calls),
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" / 30m", Style::default().fg(MOCHA_MUTED)),
    ]);
    let para = Paragraph::new(vec![line1, line2]);
    f.render_widget(para, area);
}

fn metric_cell(
    f: &mut Frame,
    area: Rect,
    label: &str,
    value: String,
    suffix: Option<String>,
    value_color: Color,
) {
    let suffix_line = match suffix {
        Some(s) => Line::from(Span::styled(s, Style::default().fg(MOCHA_MUTED))),
        None => Line::from(""),
    };
    let para = Paragraph::new(vec![
        Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(MOCHA_MUTED),
        )),
        Line::from(vec![
            Span::styled(
                value,
                Style::default()
                    .fg(value_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            suffix_line.spans.first().cloned().unwrap_or(Span::raw("")),
        ]),
    ]);
    f.render_widget(para, area);
}

// ============ LOOPING WARNING ============

fn draw_loop_warning(f: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let loopers: Vec<_> = app
        .sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Looping))
        .collect();
    if loopers.is_empty() {
        return;
    }
    let wasted_tokens: u64 = loopers.iter().map(|s| s.total_tokens / 4).sum();
    let wasted_cost: u64 = loopers.iter().map(|s| s.total_cost.as_u64() / 4).sum();
    let para = Paragraph::new(Line::from(vec![
        Span::styled(
            "⚠ ",
            Style::default().fg(MOCHA_PINK).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "{} session{} stuck in a loop - est. ~{} tokens / {} wasted on retries",
                loopers.len(),
                if loopers.len() == 1 { "" } else { "s" },
                format_tokens(wasted_tokens),
                format_cost(wasted_cost),
            ),
            Style::default().fg(MOCHA_PINK),
        ),
    ]));
    f.render_widget(para, area);
}

// ============ SESSIONS TABLE ============

fn draw_sessions(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .border_style(Style::default().fg(MOCHA_SURFACE))
        .title(Span::styled(
            " Sessions ",
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " sorted by activity ↓ ",
            Style::default().fg(MOCHA_MUTED),
        )))
        .padding(Padding::horizontal(1));

    if app.sessions.is_empty() {
        let para = Paragraph::new(Line::from(Span::styled(
            "(no sessions yet)",
            Style::default().fg(MOCHA_MUTED),
        )))
        .block(block);
        f.render_widget(para, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("AGNT"),
        Cell::from("MODEL"),
        Cell::from("PROJECT"),
        Cell::from(Line::from("CTX").alignment(Alignment::Right)),
        Cell::from(Line::from("AGE").alignment(Alignment::Right)),
        Cell::from(Line::from("TOKENS").alignment(Alignment::Right)),
        Cell::from(Line::from("COST").alignment(Alignment::Right)),
        Cell::from("STATUS"),
    ])
    .style(
        Style::default()
            .fg(MOCHA_MUTED)
            .add_modifier(Modifier::BOLD),
    );

    let now = Utc::now();
    let rows: Vec<Row> = app
        .sessions
        .iter()
        .take(area.height.saturating_sub(3) as usize)
        .map(|s| session_row(s, now))
        .collect();

    // PROJECT column hugs the longest project name so AGE sits right next to
    // it. No more "where did the right half of the row go" gap.
    let project_w = app
        .sessions
        .iter()
        .map(|s| s.project.chars().count())
        .max()
        .unwrap_or(20)
        .clamp(10, 40) as u16
        + 1; // a single character of breathing room

    let widths = [
        Constraint::Length(4),          // AGNT
        Constraint::Length(26),         // MODEL
        Constraint::Length(project_w),  // PROJECT (hugs longest name)
        Constraint::Length(8),          // CTX
        Constraint::Length(6),          // AGE
        Constraint::Length(10),         // TOKENS
        Constraint::Length(10),         // COST
        Constraint::Length(11),         // STATUS
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(HSPACING)
        .flex(Flex::Start)
        .block(block);
    f.render_widget(table, area);
}

fn session_row(s: &SessionSummary, now: DateTime<Utc>) -> Row<'_> {
    let age = now - DateTime::<Utc>::from_timestamp_millis(s.last_event_ms).unwrap_or(now);
    let agent_color = brand_color(&s.agent);
    // Visual hierarchy: ACTIVE pops (bright green + star), idle is neutral
    // (blue, no decoration), done fades into the background (very muted).
    let (status_color, star) = match s.status {
        SessionStatus::Active => (MOCHA_GREEN, "★"),
        SessionStatus::Idle => (MOCHA_BLUE, "·"),
        SessionStatus::Done => (MOCHA_SURFACE, " "),
        SessionStatus::Looping => (MOCHA_PINK, "⚠"),
    };
    // Context window % - per-session, capped at 100 since usage above the
    // window is technically impossible (Claude truncates older context).
    let ctx_window = model_context_window(&s.model);
    let ctx_pct = if ctx_window > 0 && s.peak_context > 0 {
        ((s.peak_context as f64 / ctx_window as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round() as u16
    } else {
        0
    };
    let ctx_color = if ctx_pct >= 90 {
        MOCHA_PINK
    } else if ctx_pct >= 70 {
        MOCHA_PEACH
    } else if ctx_pct >= 40 {
        MOCHA_YELLOW
    } else if ctx_pct == 0 {
        MOCHA_MUTED
    } else {
        MOCHA_GREEN
    };
    let ctx_text = if s.peak_context == 0 {
        "-".to_string()
    } else {
        format!("{}%", ctx_pct.min(999))
    };
    Row::new(vec![
        Cell::from(Span::styled(
            short_agent(&s.agent).to_string(),
            Style::default()
                .fg(agent_color)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            truncate(&s.model, 24),
            Style::default().fg(MOCHA_LAVENDER),
        )),
        Cell::from(Span::styled(
            s.project.clone(),
            Style::default().fg(MOCHA_TEAL),
        )),
        Cell::from(
            Line::from(Span::styled(
                ctx_text,
                Style::default().fg(ctx_color).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Right),
        ),
        Cell::from(
            Line::from(Span::styled(
                format_age(age),
                Style::default().fg(MOCHA_MUTED),
            ))
            .alignment(Alignment::Right),
        ),
        Cell::from(
            Line::from(Span::styled(
                format_tokens(s.total_tokens),
                Style::default().fg(MOCHA_TEXT),
            ))
            .alignment(Alignment::Right),
        ),
        Cell::from(
            Line::from(Span::styled(
                format_cost(s.total_cost.as_u64()),
                Style::default().fg(MOCHA_TEXT),
            ))
            .alignment(Alignment::Right),
        ),
        Cell::from(Span::styled(
            format!(" {star} {} ", s.status.label()),
            if matches!(s.status, SessionStatus::Looping) {
                // Inverse video for proper alarm visibility - pink background,
                // dark foreground.
                Style::default()
                    .bg(MOCHA_PINK)
                    .fg(Color::Rgb(30, 30, 46))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(status_color).add_modifier(Modifier::BOLD)
            },
        )),
    ])
}

// ============ CHART ROW ============

fn draw_chart_row(f: &mut Frame, area: Rect, app: &App) {
    // Mirror the middle row's 70/30 split so Tokens/min + Tools occupy the same
    // horizontal span as the Sessions table above, and Models lines up with
    // Cost insights.
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ])
        .spacing(HSPACING)
        .split(area);
    let left = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .spacing(HSPACING)
        .split(outer[0]);
    draw_sparkline(f, left[0], app);
    draw_bar_chart(
        f,
        left[1],
        "Tools",
        "24h",
        &app.tool_breakdown,
        MOCHA_TEAL,
        BreakdownKind::Count,
    );
    draw_bar_chart(
        f,
        outer[1],
        "Models",
        "today",
        &app.model_breakdown,
        BRAND_CLAUDE,
        BreakdownKind::Tokens,
    );
}

fn draw_rollup_row(f: &mut Frame, area: Rect, app: &App) {
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(HSPACING)
        .split(area);
    draw_bar_chart(
        f,
        cells[0],
        "Hot projects",
        "24h",
        &app.project_breakdown,
        MOCHA_PEACH,
        BreakdownKind::Cost,
    );
    draw_hot_files(f, cells[1], app);
}

fn draw_hot_files(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .border_style(Style::default().fg(MOCHA_SURFACE))
        .title(Span::styled(
            " Hot files ",
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " 24h ",
            Style::default().fg(MOCHA_MUTED),
        )))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.hot_files.is_empty() {
        let para = Paragraph::new(Line::from(Span::styled(
            "(no files edited yet)",
            Style::default().fg(MOCHA_MUTED),
        )));
        f.render_widget(para, inner);
        return;
    }

    let max = app
        .hot_files
        .iter()
        .map(|b| b.count)
        .max()
        .unwrap_or(1)
        .max(1);
    let total: u64 = app.hot_files.iter().map(|b| b.count).sum();

    // Two rows per entry: file path on line 1, bar + stats on line 2.
    // Fits 4 entries in 9 rows comfortably with a blank line in between.
    let label_max = inner.width as usize - 2;
    let bar_width = (inner.width as usize / 2).clamp(10, 30);

    let mut lines: Vec<Line> = Vec::new();
    let entries_visible = ((inner.height as usize) / 2).max(1);
    for b in app.hot_files.iter().take(entries_visible) {
        // File path row - uses full pane width.
        lines.push(Line::from(Span::styled(
            truncate(&b.label, label_max),
            Style::default()
                .fg(MOCHA_TEAL)
                .add_modifier(Modifier::BOLD),
        )));
        // Bar + edits + percentage row
        let frac = b.count as f64 / max as f64;
        let bar = horizontal_bar(frac, bar_width);
        let pct = if total > 0 {
            (b.count as f64 / total as f64 * 100.0).round() as u16
        } else {
            0
        };
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(bar, Style::default().fg(MOCHA_YELLOW)),
            Span::styled(
                format!(" {:>3}%", pct),
                Style::default()
                    .fg(MOCHA_LAVENDER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} edits", b.count),
                Style::default().fg(MOCHA_MUTED),
            ),
        ]));
    }
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn draw_sparkline(f: &mut Frame, area: Rect, app: &App) {
    if app.sparkline.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BORDER)
            .border_style(Style::default().fg(MOCHA_SURFACE))
            .title(Span::styled(
                " Tokens / minute ",
                Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Line::from(Span::styled(
                " last 60m ",
                Style::default().fg(MOCHA_MUTED),
            )))
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let para = Paragraph::new(Line::from(Span::styled(
            "(no data yet)",
            Style::default().fg(MOCHA_MUTED),
        )));
        f.render_widget(para, inner);
        return;
    }

    // Build (x, y) dataset. x is bucket index (0..60), y is tokens.
    let data: Vec<(f64, f64)> = app
        .sparkline
        .iter()
        .enumerate()
        .map(|(i, b)| (i as f64, b.tokens as f64))
        .collect();
    let max_y = app
        .sparkline
        .iter()
        .map(|b| b.tokens)
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let bucket_count = app.sparkline.len() as f64;

    // Braille markers + Line graph = Grafana-style smooth line.
    let dataset = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(MOCHA_LAVENDER))
        .data(&data);

    let chart = Chart::new(vec![dataset])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BORDER)
                .border_style(Style::default().fg(MOCHA_SURFACE))
                .title(Span::styled(
                    " Tokens / minute ",
                    Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
                ))
                .title_bottom(Line::from(Span::styled(
                    " last 60m ",
                    Style::default().fg(MOCHA_MUTED),
                ))),
        )
        .x_axis(
            Axis::default()
                .style(Style::default().fg(MOCHA_SURFACE))
                .bounds([0.0, bucket_count - 1.0])
                .labels(vec![
                    Span::styled("60m", Style::default().fg(MOCHA_MUTED)),
                    Span::styled("30m", Style::default().fg(MOCHA_MUTED)),
                    Span::styled(
                        "now",
                        Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
                    ),
                ]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(MOCHA_SURFACE))
                .bounds([0.0, max_y * 1.1])
                .labels(vec![
                    Span::styled("0", Style::default().fg(MOCHA_MUTED)),
                    Span::styled(
                        format_tokens((max_y / 2.0) as u64),
                        Style::default().fg(MOCHA_MUTED),
                    ),
                    Span::styled(
                        format_tokens(max_y as u64),
                        Style::default().fg(MOCHA_MUTED),
                    ),
                ]),
        );
    f.render_widget(chart, area);
}

#[derive(Copy, Clone)]
enum BreakdownKind {
    Count,
    Tokens,
    Cost,
}

/// Charm-style horizontal bar chart: rounded border, percentage column,
/// dynamically sized bar.
fn draw_bar_chart(
    f: &mut Frame,
    area: Rect,
    title: &str,
    subtitle: &str,
    items: &[Breakdown],
    bar_color: Color,
    kind: BreakdownKind,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BORDER)
        .border_style(Style::default().fg(MOCHA_SURFACE))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(MOCHA_TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            format!(" {subtitle} "),
            Style::default().fg(MOCHA_MUTED),
        )))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if items.is_empty() {
        let para = Paragraph::new(Line::from(Span::styled(
            "(no data)",
            Style::default().fg(MOCHA_MUTED),
        )));
        f.render_widget(para, inner);
        return;
    }

    let value = |b: &Breakdown| match kind {
        BreakdownKind::Count => b.count,
        BreakdownKind::Tokens => b.tokens,
        BreakdownKind::Cost => b.tokens,
    };
    let max = items.iter().map(value).max().unwrap_or(1).max(1);
    let total: u64 = items.iter().map(value).sum();

    // Compute responsive widths from the actual inner area.
    // For path-heavy data (Hot files), give labels much more room and shrink
    // the bar. For short-label data (Tools, Models), keep a balanced layout.
    let total_width = inner.width as usize;
    let is_path_heavy = items.iter().any(|b| b.label.contains('/'));
    let value_str_width = 9usize;
    let pct_width = 5usize;
    let (label_width, bar_width) = if is_path_heavy {
        // Path data - labels are the point, bar is supporting.
        let label = total_width.saturating_sub(8 + value_str_width + pct_width + 3).clamp(20, 45);
        (label, 8usize)
    } else {
        // Short-label data - balanced.
        let label = (total_width / 3).clamp(10, 22);
        let bar = total_width
            .saturating_sub(label)
            .saturating_sub(value_str_width)
            .saturating_sub(pct_width)
            .saturating_sub(3);
        (label, bar)
    };

    let rows_visible = inner.height as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(items.len());
    for b in items.iter().take(rows_visible) {
        let v = value(b);
        let frac = v as f64 / max as f64;
        let pct = if total > 0 {
            (v as f64 / total as f64 * 100.0).round() as u16
        } else {
            0
        };
        let value_str = match kind {
            BreakdownKind::Count => format!("{}", v),
            BreakdownKind::Tokens => format_tokens(v),
            BreakdownKind::Cost => format_cost(v),
        };
        // Sub-character precision: 8 levels per character (▏▎▍▌▋▊▉█).
        // gives smooth bar edges and creates natural visual rhythm vertically.
        let bar = horizontal_bar(frac, bar_width);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<width$}", truncate(&b.label, label_width), width = label_width),
                Style::default().fg(MOCHA_TEXT),
            ),
            Span::raw(" "),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::styled(
                format!(" {:>3}%", pct),
                Style::default().fg(MOCHA_LAVENDER).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {:>width$}", value_str, width = value_str_width - 1),
                Style::default().fg(MOCHA_MUTED),
            ),
        ]));
    }
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

/// Render a horizontal bar with 8 sub-character levels per cell.
/// Empty cells render as space (not dots) for a cleaner Charm look.
fn horizontal_bar(frac: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let frac = frac.clamp(0.0, 1.0);
    let total_eighths = (frac * (width * 8) as f64).round() as usize;
    let full = total_eighths / 8;
    let partial = total_eighths % 8;
    let mut s = String::with_capacity(width * 3);
    for _ in 0..full.min(width) {
        s.push('█');
    }
    if full < width && partial > 0 {
        s.push(eighth_block(partial as u8));
        for _ in (full + 1)..width {
            s.push(' ');
        }
    } else {
        for _ in full..width {
            s.push(' ');
        }
    }
    s
}

fn eighth_block(eighths: u8) -> char {
    // Left-aligned partial blocks, 1..=7 eighths.
    match eighths.min(7) {
        0 => ' ',
        1 => '▏',
        2 => '▎',
        3 => '▍',
        4 => '▌',
        5 => '▋',
        6 => '▊',
        7 => '▉',
        _ => '█',
    }
}

// ============ helpers ============

fn brand_color(agent: &str) -> Color {
    match agent {
        "Claude Code" | "Claude Desktop" => BRAND_CLAUDE,
        "Codex CLI" => MOCHA_TEAL,
        "Cursor" => MOCHA_PEACH,
        "Gemini CLI" => MOCHA_BLUE,
        "Windsurf" => MOCHA_BLUE,
        "OpenCode" => MOCHA_GREEN,
        _ => MOCHA_TEXT,
    }
}

fn short_agent(agent: &str) -> &str {
    match agent {
        "Claude Code" => "CC",
        "Claude Desktop" => "CD",
        "Codex CLI" => "CX",
        "Cursor" => "Cur",
        "Gemini CLI" => "Gm",
        "Windsurf" => "Wf",
        "OpenCode" => "OC",
        other => other,
    }
}

fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f32 / 1_000.0)
    } else if n < 1_000_000_000 {
        format!("{:.1}M", n as f32 / 1_000_000.0)
    } else {
        format!("{:.1}B", n as f32 / 1_000_000_000.0)
    }
}

fn format_cost(microcents: u64) -> String {
    let cents = microcents / 100;
    let dollars = cents / 100;
    let remainder = cents % 100;
    if dollars >= 1000 {
        format!("${:.1}k", dollars as f32 / 1000.0)
    } else {
        format!("${}.{:02}", dollars, remainder)
    }
}

fn format_age(d: Duration) -> String {
    let s = d.num_seconds().max(0);
    if s < 10 {
        "now".to_string()
    } else if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86400)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    // For path-like strings, the meaningful part is the tail (file name and
    // last few directories). Truncate from the start: `…/end/of/path/file.ext`.
    if looks_like_path(s) {
        let tail: String = s
            .chars()
            .rev()
            .take(max - 1)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return format!("…{tail}");
    }
    let mut t: String = s.chars().take(max - 1).collect();
    t.push('…');
    t
}

fn looks_like_path(s: &str) -> bool {
    // Heuristic: contains a `/` or `\` and is longer than a typical word.
    (s.contains('/') || s.contains('\\')) && s.len() > 8
}
