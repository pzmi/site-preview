//! One [`Driver`](crate::Driver) per Quint model, each over the real code with real adapters,
//! and the projection of that model's state into the driver's observation.

pub mod lifecycle;
pub mod reload;
pub mod watcher;
