//! The native file watcher (plan.md section 8, ADR 0047).
//!
//! [`NotifyWatcher`] registers one non-recursive `notify` watch per eligible directory, the set
//! [`crate::eligible_dirs`] computes with the predicate serving uses, so an ignored subtree
//! (`.git`, `node_modules`, a hidden directory) is never registered with the operating system
//! (plan.md section 2.1). `notify` types stay inside this module.

mod registration;

pub use registration::{NotifyWatcher, RegistrationError, WatchFault};

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
