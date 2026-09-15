//! Host-side adapters around the OS-independent `lab-core` domain.
//!
//! M3 keeps JSON parsing here so Core does not depend on a serialization format.

pub mod application;
pub mod definition;
/// Explicit monotonic scheduling and trusted virtual host composition.
pub mod host;
/// Startup ownership, loopback binding and process-local boot identity.
pub mod service;
/// Finite process-local operation retention and reconnect deduplication.
pub mod sessions;
/// Bounded version-one NDJSON framing and strict host-side DTO validation.
pub mod wire;
