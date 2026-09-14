//! Synchronous, deterministic instrument and output-authority domain boundary.
//!
//! [`Runtime`] owns instrument state and accepts explicit commands with caller-supplied
//! time. Queries return owned observations without executing work. The [`output`]
//! module adds bounded leases and a queue/send/complete simulation: a requested value
//! is not evidence that an actuator received it. No OS transport or worker thread is
//! hidden behind this boundary.

mod model;
pub mod output;
mod runtime;
mod signal;
mod virtual_instrument;

pub use model::*;
pub use runtime::*;
pub use signal::{MAX_HISTORY_CAPACITY, Sample, SampleQuality};
pub use virtual_instrument::{
    BASE_TEMPERATURE, HEATER_POWER, MEASUREMENT_ENABLED, TEMPERATURE, VirtualInstrumentConfig,
};
