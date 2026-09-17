//! Bounded embedded Lua observation adapter outside the Rust output authority.
//!
//! Every invocation owns a disposable Lua VM on the host's common bounded
//! managed-component executor. The VM receives copied scalar
//! input/configuration/state, not Runtime handles.

mod runner;

pub mod fixtures;

pub use runner::run_bounded;

/// Stable semantic implementation identity understood by this adapter.
pub const IMPLEMENTATION_ID: &str = "lua.v1";
