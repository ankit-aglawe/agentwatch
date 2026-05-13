//! Local read-only dashboard.
//!
//! Hard-bound to `127.0.0.1` (Invariant: never `0.0.0.0`). Random URL token
//! issued at boot, printed to the TUI / stdout. Token lives only for the
//! lifetime of this `serve` invocation — rotating tokens limit screenshot
//! leak risk.
//!
//! Read-only: the webapp never mutates SQLite. Settings live in
//! `~/.agentwatch/config.toml` and are edited by hand for v0.1.

pub mod routes;
pub mod assets;

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("port allocation failed after retries")]
    PortAllocation,
}

pub struct ServeConfig {
    pub preferred_port: u16,
    pub max_port_retries: u16,
    pub token: Uuid,
}

impl ServeConfig {
    pub fn new() -> Self {
        Self {
            preferred_port: 7878,
            max_port_retries: 20,
            token: Uuid::new_v4(),
        }
    }
}

impl Default for ServeConfig {
    fn default() -> Self { Self::new() }
}

pub async fn serve(_config: ServeConfig) -> Result<(), WebError> {
    // Day 8 work: bind to 127.0.0.1, walk ports if 7878 taken, mount routes,
    // print the share URL with token, run until SIGINT.
    Ok(())
}
