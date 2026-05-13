//! Live terminal HUD - the calm front page.
//!
//! Design philosophy (locked in PLAN.md "TUI design spec"):
//!   - Three sections only: status headline · Right now · Today
//!   - Plain English everywhere ("editing src/auth.ts", not "Edit tool call")
//!   - Calm at rest, lights up when something matters
//!   - 10 Hz redraw cap; reads from SQLite once per refresh tick

pub mod app;
pub mod live_signal;
pub mod panes;
pub mod render;
pub mod runway;
pub mod watch;

use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use thiserror::Error;

use crate::app::App;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("terminal init failed")]
    TerminalInit,
}

/// Boot the TUI event loop. Returns on Ctrl-C / 'q' / Esc.
pub fn run() -> Result<(), TuiError> {
    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Spawn the background watcher. It re-ingests changed session files every
    // 3 s; the TUI poll picks up the new events naturally on its 2 s tick.
    // Drop on the Watcher signals the thread to stop and joins it.
    let _watcher = watch::Watcher::spawn(Duration::from_secs(3));

    let mut app = App::new();
    let result = run_loop(&mut terminal, &mut app);

    // Tear down terminal even if the loop errored.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
    )?;
    terminal.show_cursor()?;

    result
}

fn run_loop<B>(terminal: &mut Terminal<B>, app: &mut App) -> Result<(), TuiError>
where
    B: ratatui::backend::Backend,
{
    let tick = Duration::from_millis(100); // 10 Hz draw cap
    let data_refresh = Duration::from_millis(2000); // poll DB every 2 s
    let mut last_data_refresh = Instant::now() - data_refresh;

    loop {
        // Refresh data if interval elapsed.
        if last_data_refresh.elapsed() >= data_refresh {
            app.refresh();
            last_data_refresh = Instant::now();
        }

        terminal.draw(|f| render::draw(f, app))?;

        // Wait for next event or tick.
        let timeout = tick;
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if should_quit(key) {
                    return Ok(());
                }
                if let KeyCode::Char('r') = key.code {
                    app.refresh();
                    last_data_refresh = Instant::now();
                }
            }
        }
    }
}

fn should_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}
