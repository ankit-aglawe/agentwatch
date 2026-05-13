//! agentwatch-core
//!
//! The shared spine. Every other crate in the workspace depends on this one,
//! and only this one. Adapters cannot import storage, storage cannot import
//! TUI/web - the `AgentEvent` type defined here is the only contract.
//!
//! See `PLAN.md` for the locked v0.1 schema.

pub mod event;
pub mod money;
pub mod paths;
pub mod plan_detect;
pub mod pricing;

pub use event::{Agent, AgentEvent, Capability, EventKind, SessionEndReason};
pub use money::Microcents;
