//! Synchronous, deterministic experiment-domain boundary.
//!
//! # Ownership
//!
//! [`Runtime`] is the sole authoritative mutable owner of instruments, committed
//! measurements, references, controllers, output authority, resource executors and
//! managed-component state. [`Command`] performs explicit mutation or work with
//! caller-supplied monotonic time; [`Query`] returns an owned committed projection
//! without hidden polling. No OS transport, SQLite connection, network client or
//! worker thread is hidden behind this boundary.
//!
//! # Architecture index
//!
//! - [`instrument`] and [`metakon`] define the physical instrument contract and pure
//!   protocol codec; [`transport`] serializes bounded byte transactions.
//! - [`control`], [`mod@reference`] and [`output`] contain native control and the output
//!   safety vocabulary. The private `OutputAuthority` state machine is owned only by
//!   [`Runtime`].
//! - [`managed`] defines the language-neutral invocation/result/executor contract;
//!   implementations run outside the owner and only [`Runtime`] can validate and
//!   commit their results.
//! - [`recording`] contains storage-independent semantic facts and the bounded
//!   handoff contract. Durable SQLite storage belongs to the host crate.
//!
//! A requested output is not proof of authorization, bytes sent, ACK, readback or
//! physical effect. Those stages remain separate throughout the domain.

pub mod control;
pub mod instrument;
pub mod managed;
pub mod metakon;
mod model;
pub mod output;
pub mod plant;
pub mod processing;
pub mod recording;
pub mod reference;
mod runtime;
mod signal;
pub mod transport;
mod virtual_instrument;

pub use model::*;
pub use runtime::*;
pub use signal::{MAX_HISTORY_CAPACITY, Sample, SampleQuality};
pub use virtual_instrument::{
    BASE_TEMPERATURE, HEATER_POWER, MEASUREMENT_ENABLED, TEMPERATURE, VirtualInstrumentConfig,
};
