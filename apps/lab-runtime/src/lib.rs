//! Process orchestration and adapters around the OS-independent `lab-core` domain.
//!
//! # Ownership index
//!
//! - [`lab_core::Runtime`] is the sole authoritative mutable experiment owner.
//! - [`host::HostCore`] owns that Runtime and drives monotonic schedules, events,
//!   Recorder admission and adapter progress without transferring domain authority.
//! - [`service::ServiceHost`] owns process startup, loopback readiness, deployment,
//!   reconnect and finite shutdown around `HostCore`.
//! - [`application::Application`] owns bounded client/session/delivery state and maps
//!   the accepted local API to the serialized service owner. A connection never owns
//!   experiment lifetime.
//! - [`recorder::RecorderWorker`] and [`recorder::SqliteStore`] own durable recording
//!   machinery, not experiment state.
//!
//! # Navigation index
//!
//! - Periodic acquisition and controller cadence: [`host`], then
//!   [`lab_core::Runtime`] and [`lab_core::transport`].
//! - Physical protocol and COM bytes: [`definition`], [`lab_core::metakon`], and the
//!   crate-private `serial` adapter.
//! - Physical output safety: [`lab_core::output`] and Runtime's private
//!   `OutputAuthority` path through [`lab_core::transport::ResourceExecutor`].
//! - Application protocol, framing and delivery: [`protocol`], [`wire`], [`server`]
//!   and [`application`].
//! - Recorder contract, bounded ingress, history/provenance and SQLite: [`recorder`]
//!   plus storage-independent [`lab_core::recording`].
//! - Native managed components: [`managed_executor`] behind
//!   [`lab_core::managed::ComponentExecutor`].
//! - Persistent deployment and staged apply: [`configuration`] and [`deployment`].

/// Accepted Application API dispatch and bounded per-client delivery state.
pub mod application;
/// Cached content identity for built-in managed-component provenance.
pub mod build_identity;
/// Strict bounded declarative deployment loading and validation.
pub mod configuration;
/// Semantic resource and validated property projections for the Application API.
pub mod configuration_api;
/// Strict declarative instrument-definition adapter to trusted Core operations.
pub mod definition;
/// Staged configuration diff and explicit apply lifecycle.
pub mod deployment;
/// Bounded transient semantic-event projection; not durable Recorder history.
pub mod events;
/// Explicit monotonic scheduling and trusted virtual host composition.
pub mod host;
/// Compile-time managed implementation registry and shared bounded worker pool.
pub mod managed_executor;
/// Stable public discovery and measurement DTO builders.
pub mod measurements;
/// Stable Application-protocol identity, operation registry, and public errors.
pub mod protocol;
/// Bounded durable-history storage adapter and host-owned recording ingress.
pub mod recorder;
/// Semantic Recorder projections kept independent from its SQLite implementation.
pub mod recorder_api;
/// Bounded worker-backed Windows COM byte adapter, private to trusted host orchestration.
pub(crate) mod serial;
pub mod server;
/// Startup ownership, loopback binding and process-local boot identity.
pub mod service;
/// Finite process-local operation retention and reconnect deduplication.
pub mod sessions;
/// Bounded version-one NDJSON framing and strict host-side DTO validation.
pub mod wire;
