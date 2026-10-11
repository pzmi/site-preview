//! Commands: use cases that change application state.
//!
//! A command returns its outcome together with the [`DomainEvent`](crate::events::DomainEvent)s
//! it produced, as a value; there is no command or event bus (foundation.md section 2.4), so
//! the caller decides where each event goes. Every command has a test entry point
//! (foundation.md section 5.2).
//!
//! | Command | On [`ChangeCoordinator`] | Module |
//! |:--|:--|:--|
//! | record a batch of changes, a lost-change overflow, or any [`ChangeBatch`](crate::ChangeBatch) | `record`, `overflow`, `receive` | [`record_change`] |
//! | publish the settled changes as the next revision | `settle` | [`settle`] |
//! | report the watcher failed or registered | `watcher_failed`, `watcher_ready` | [`watcher_health`] |

mod change_coordinator;
pub mod record_change;
pub mod settle;
pub mod watcher_health;

#[cfg(test)]
mod test_support;

pub use change_coordinator::ChangeCoordinator;
pub use record_change::{Received, RecordOutcome};
pub use settle::{SettleOutcome, Settled};
