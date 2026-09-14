//! Synchronous, deterministic Milestone 1 domain boundary.

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
