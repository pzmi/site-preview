//! Drives the real code from Quint model traces and compares observable state; test only.
//!
//! The explicit replay path (foundation.md section 6.3, ADR 0053):
//!
//! - [`itf`] decodes the ITF traces Quint writes (`quint run --out-itf ... --mbt`);
//! - [`replay`](mod@replay) feeds each step's action and picked parameters to a [`Driver`]
//!   over the real code and compares its observation with the model's projected state after
//!   every step;
//! - [`report`] formats a [`Divergence`]: model, seed, trace, step, action, expected, actual
//!   and a line diff.
//!
//! Production crates never depend on this crate (`architecture.toml`, `test_only`).

pub mod itf;
pub mod replay;
pub mod report;

pub use itf::{DecodeError, State, Trace, Value};
pub use replay::{Cause, Divergence, Driver, DriverError, Params, ProjectionError, replay};
