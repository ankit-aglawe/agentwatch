//! HTTP routes.
//!
//! All routes require the URL token issued at `serve` boot. Tokens are
//! verified by middleware; never logged.
//!
//! GET /                       → static SPA shell (embedded)
//! GET /api/today              → today's totals + per-agent breakdown
//! GET /api/hourly/:date       → 24 hourly buckets for a local day
//! GET /api/sessions           → recent sessions list
//! GET /api/sessions/:id       → one session's event timeline (paths only)
//! GET /api/badges             → per-adapter capability + last-event timestamp
//! GET /share/:token           → static HTML snapshot for sharing
//! GET /events                 → SSE stream of new AgentEvents
