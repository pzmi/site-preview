//! The native file watcher (plan.md section 8, ADR 0047).
//!
//! [`NotifyChangeSource`] is the application's `ChangeSource`. Inside it, `NotifyWatcher`
//! registers one non-recursive `notify` watch per eligible directory, the set
//! [`crate::eligible_dirs`] computes with the predicate serving uses, so an ignored subtree
//! (`.git`, `node_modules`, a hidden directory) is never registered with the operating system
//! (plan.md section 2.1); `translate` turns each `notify` event into the domain's root-relative
//! change events; and the bridge carries them from the native callback to the consumer without
//! ever blocking it. `notify` types stay inside this crate.

mod bridge;
mod registration;
mod translate;

pub use bridge::{DEFAULT_CAPACITY, NotifyChangeSource, RuntimeError, StartReport};
#[cfg(test)]
pub(crate) use registration::NotifyWatcher;
pub use registration::{RegistrationError, WatchFault};
#[cfg(test)]
pub(crate) use translate::translate;

use preview_domain::{Component, RelPath};

/// `dir` as the user sees it below the site root: components joined with `/`, the root as
/// `.`. Never an absolute path.
fn display_rel(dir: &RelPath) -> String {
    if dir.is_root() {
        return ".".to_owned();
    }
    dir.components()
        .iter()
        .map(Component::as_str)
        .collect::<Vec<_>>()
        .join("/")
}
