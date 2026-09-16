//! Host-side adapters around the OS-independent `lab-core` domain.
//!
//! M3 keeps JSON parsing here so Core does not depend on a serialization format.

pub mod application;
/// Strict bounded declarative deployment loading and validation.
pub mod configuration;
pub mod definition;
pub mod events;
/// Explicit monotonic scheduling and trusted virtual host composition.
pub mod host;
/// Bounded durable-history storage adapter and host-owned recording ingress.
pub mod recorder;
pub mod server;
/// Startup ownership, loopback binding and process-local boot identity.
pub mod service;
/// Finite process-local operation retention and reconnect deduplication.
pub mod sessions;
/// Bounded version-one NDJSON framing and strict host-side DTO validation.
pub mod wire;
