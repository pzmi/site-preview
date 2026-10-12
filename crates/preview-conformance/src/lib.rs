//! Drives the real code from Quint model traces and compares observable state; test only.
//!
//! The explicit replay path (foundation.md section 6.3, ADR 0053):
//!
//! - [`itf`] decodes the ITF traces Quint writes (`quint run --out-itf ... --mbt`);
//! - [`replay`](mod@replay) feeds each step's action and picked parameters to a [`Driver`]
//!   over the real code and compares its observation with the model's projected state after
//!   every step;
//! - [`report`] formats a [`Divergence`]: model, seed, trace, step, action, expected, actual
//!   and a line diff;
//! - [`drivers`] holds one driver per model over the real code (the lifecycle driver drives
//!   the composition's `Server`, FORMAL-05; the reload driver drives the real `ReloadState`
//!   and SSE handler in process, FORMAL-07);
//! - [`generate`] writes seeded random traces with the locked Quint at test time.
//!
//! Production crates never depend on this crate (`architecture.toml`, `test_only`).

pub mod drivers;
pub mod generate;
pub mod itf;
pub mod replay;
pub mod report;

pub use itf::{DecodeError, State, Trace, Value};
pub use replay::{Cause, Divergence, Driver, DriverError, Params, ProjectionError, replay};
