//! Bounded embedded Lua observation adapter outside the Rust output authority.
//!
//! Every invocation owns a disposable Lua VM on one of two fixed workers.
//! The VM receives copied scalar input/configuration/state, not Runtime handles.

mod runner;
mod supervisor;

pub mod fixtures;

pub use runner::run_bounded;
pub use supervisor::LuaSupervisor;
