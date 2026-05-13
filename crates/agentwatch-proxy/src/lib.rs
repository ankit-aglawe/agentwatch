//! Opt-in HTTP proxy mode for direct-API users (those who call Anthropic /
//! OpenAI / Google SDKs directly without a coding agent in the loop).
//!
//! Usage: `agentwatch proxy` boots a CONNECT proxy on 127.0.0.1:7777. The
//! user exports `HTTPS_PROXY=http://localhost:7777` and direct-API calls flow
//! through us. We READ USAGE FROM RESPONSE HEADERS ONLY by default — never
//! the request/response bodies. Bodies require `--log-bodies` opt-in, which
//! writes to `~/.agentwatch/proxy/` with 24h TTL and a loud warning banner.
//!
//! No TLS termination by default (no local CA to trust). Anthropic and OpenAI
//! expose usage in response headers like `x-ratelimit-*` and `openai-*-usage`,
//! which are sufficient for cost tracking.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("port {0} already in use")]
    PortInUse(u16),
}

pub struct ProxyConfig {
    pub port: u16,
    pub log_bodies: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self { port: 7777, log_bodies: false }
    }
}

pub async fn run(_config: ProxyConfig) -> Result<(), ProxyError> {
    // Day 8 work: CONNECT proxy on 127.0.0.1, intercept headers, emit
    // AgentEvent::ModelCall for known API hosts.
    Ok(())
}
