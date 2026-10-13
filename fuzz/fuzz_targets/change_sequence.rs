//! Fuzz target for the change pipeline: arbitrary sequences of change batches, health reports
//! and clock ticks into the real `ChangeCoordinator` (plan.md §8.2 to §8.4, SEC-05).
//!
//! Input layout (so the committed seeds stay readable): byte 0 picks the debounce window
//! (`10 + 40 * b` ms, at most 10 s); then a sequence of operations, one opcode byte each
//! (taken modulo 8) followed by its operands:
//!
//! | Opcode | Operation | Operands |
//! |:--|:--|:--|
//! | 0 | `receive(Events)` | count (mod 8), then per event: kind (mod 8), depth (mod 4), and two bytes per component |
//! | 1 | `receive(Overflow)` | none |
//! | 2 | `receive(Failed)` | one byte: even is `RegistrationFailed`, odd `RuntimeFailure` |
//! | 3 | `watcher_ready` | directories (one byte) |
//! | 4 | `watcher_reconciled` | directories (one byte) |
//! | 5 | advance the clock | milliseconds (two bytes, big-endian) |
//! | 6 | `settle` now | none |
//! | 7 | advance the clock to the pending deadline, then `settle` | none |
//!
//! A component is two bytes `(a, b)`: `a < 16` names one of [`NAMES`] (ordinary, hidden,
//! excluded, reserved and editor-artifact names); otherwise it is `f<b>.<ext>` with the
//! extension `EXTENSIONS[a % len]`, so more than 64 distinct hot paths are reachable.
//!
//! The coordinator must never panic, and:
//!
//! 1. the published revision is monotonic: nothing but `settle` changes it, and a publish
//!    advances it by exactly one;
//! 2. nothing is published for ignored-only input: a publish needs a non-ignored event
//!    ([`preview_domain::classify`]), an overflow or a recovery since the previous publish;
//! 3. a published `ChangeSet` that is not full has at most `MAX_CHANGES` (64) entries, all of
//!    kind css, js or asset; a full one has at most 64 too, and a rescan is always full.

#![no_main]

use libfuzzer_sys::fuzz_target;
use preview_application::{ChangeBatch, ChangeCoordinator, Received, ReloadState, SourceFailure};
use preview_domain::{
    AssetKind, ChangeEvent, ChangeKind, Classification, Component, DebounceWindow, DomainEvent,
    InstanceId, MAX_CHANGES, RelPath, Tick, classify,
};

const KINDS: [ChangeKind; 8] = [
    ChangeKind::Create,
    ChangeKind::Modify,
    ChangeKind::Remove,
    ChangeKind::Rename,
    ChangeKind::DirectoryChanged,
    ChangeKind::AccessOnly,
    ChangeKind::MetadataOnly,
    ChangeKind::Ambiguous,
];

const NAMES: [&str; 16] = [
    "index.html",
    "style.css",
    "app.js",
    "logo.png",
    ".git",
    "node_modules",
    "vendor",
    "docs",
    "notes.txt",
    "module.mjs",
    "font.woff2",
    "__site_preview",
    ".env",
    "THEME.CSS",
    "page.htm",
    "backup.html~",
];

const EXTENSIONS: [&str; 8] = ["css", "js", "png", "html", "txt", "svg", "swp", ""];

/// A byte cursor; an exhausted input reads as zeros, so every input is a valid program.
struct Input<'a> {
    bytes: &'a [u8],
}

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        match self.bytes.split_first() {
            Some((first, rest)) => {
                self.bytes = rest;
                *first
            }
            None => 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// The oracle's view: what the properties need to know about the sequence so far.
struct Oracle {
    now: Tick,
    revision: u64,
    /// A non-ignored event, an overflow or a recovery happened since the last publish.
    publishable: bool,
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input { bytes: data };
    let window = DebounceWindow::from_millis((10 + 40 * u64::from(input.byte())).min(10_000))
        .expect("a window within the bounds");
    let reload = ReloadState::new(InstanceId::parse("fuzz").expect("a valid instance id"));
    let mut coordinator = ChangeCoordinator::new(window, reload.clone());
    let mut oracle = Oracle {
        now: Tick::from_millis(0),
        revision: reload.current().revision().get(),
        publishable: false,
    };

    while !input.is_empty() {
        let published_by_this_step = step(&mut coordinator, &mut input, &mut oracle);
        let revision = reload.current().revision().get();
        if published_by_this_step {
            assert_eq!(
                revision,
                oracle.revision + 1,
                "a publish skipped or repeated a revision"
            );
            oracle.revision = revision;
            oracle.publishable = false;
        } else {
            assert_eq!(
                revision, oracle.revision,
                "the revision moved without a publish"
            );
        }
    }
});

/// Runs one operation; returns whether it published a revision.
fn step(coordinator: &mut ChangeCoordinator, input: &mut Input<'_>, oracle: &mut Oracle) -> bool {
    match input.byte() % 8 {
        0 => {
            let events = events(input);
            if events
                .iter()
                .any(|event| !matches!(classify(event), Classification::Ignored(_)))
            {
                oracle.publishable = true;
            }
            let received = coordinator.receive(ChangeBatch::Events(events), oracle.now);
            assert!(matches!(received, Received::Recorded(_)));
        }
        1 => {
            oracle.publishable = true;
            let _ = coordinator.receive(ChangeBatch::Overflow, oracle.now);
        }
        2 => {
            let failure = if input.byte().is_multiple_of(2) {
                SourceFailure::RegistrationFailed
            } else {
                SourceFailure::RuntimeFailure
            };
            let _ = coordinator.receive(ChangeBatch::Failed(failure), oracle.now);
        }
        3 => {
            let _ = coordinator.watcher_ready(usize::from(input.byte()));
        }
        4 => {
            let dirs = usize::from(input.byte());
            if let Ok(events) = coordinator.watcher_reconciled(dirs, oracle.now)
                && events.contains(&DomainEvent::WatcherRecovered)
            {
                oracle.publishable = true;
            }
        }
        5 => {
            let millis = u16::from_be_bytes([input.byte(), input.byte()]);
            oracle.now = Tick::from_millis(oracle.now.as_millis().saturating_add(millis.into()));
        }
        6 => return settle(coordinator, oracle),
        _ => {
            if let Some(deadline) = coordinator.deadline()
                && deadline.at() > oracle.now
            {
                oracle.now = deadline.at();
            }
            return settle(coordinator, oracle);
        }
    }
    false
}

fn settle(coordinator: &mut ChangeCoordinator, oracle: &Oracle) -> bool {
    let outcome = coordinator.settle(oracle.now);
    let Some(settled) = outcome.settled else {
        return false;
    };
    assert!(
        oracle.publishable,
        "published a revision for ignored-only input"
    );
    assert_eq!(
        settled.state.revision().get(),
        oracle.revision + 1,
        "the settled revision is not the next one"
    );
    let changes = &settled.changes;
    assert!(
        changes.changes().len() <= MAX_CHANGES,
        "{} changes exceed the cap",
        changes.changes().len()
    );
    if settled.rescan {
        assert!(changes.is_full(), "a rescan revision is not a full reload");
    }
    if !changes.is_full() {
        for changed in changes.changes() {
            assert!(
                matches!(
                    changed.kind(),
                    AssetKind::Css | AssetKind::Js | AssetKind::Asset
                ),
                "a hot change set holds {:?} ({})",
                changed.kind(),
                changed.url_path()
            );
        }
    }
    true
}

fn events(input: &mut Input<'_>) -> Vec<ChangeEvent> {
    let count = input.byte() % 8;
    (0..count)
        .map(|_| {
            let kind = KINDS[usize::from(input.byte() % 8)];
            let depth = input.byte() % 4;
            let components = (0..depth).map(|_| component(input)).collect();
            ChangeEvent {
                kind,
                path: RelPath::new(components),
            }
        })
        .collect()
}

fn component(input: &mut Input<'_>) -> Component {
    let (selector, number) = (input.byte(), input.byte());
    let name = match NAMES.get(usize::from(selector)) {
        Some(name) => (*name).to_owned(),
        None => match EXTENSIONS[usize::from(selector) % EXTENSIONS.len()] {
            "" => format!("f{number}"),
            extension => format!("f{number}.{extension}"),
        },
    };
    Component::new(&name).expect("a valid component name")
}
