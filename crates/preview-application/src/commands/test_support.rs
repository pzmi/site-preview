//! Builders shared by the command unit tests.

use preview_domain::{ChangeEvent, ChangeKind, Component, RelPath, Tick};

/// A change of `kind` at the root-relative path made of `parts`.
pub(crate) fn event(kind: ChangeKind, parts: &[&str]) -> ChangeEvent {
    ChangeEvent {
        kind,
        path: RelPath::new(
            parts
                .iter()
                .map(|p| Component::new(p).expect("valid test component"))
                .collect(),
        ),
    }
}

/// The instant `millis` milliseconds after the clock's origin.
pub(crate) const fn ms(millis: u64) -> Tick {
    Tick::from_millis(millis)
}
