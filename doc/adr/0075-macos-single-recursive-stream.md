# 0075. macOS: one recursive FSEvents stream, ignored subtrees dropped in the bridge

Date: 2026-10-13

## Status

Proposed (planner default — owner may override). Amends 0047 (the watcher), whose consequence
on FSEvents restarts OPS-03 was to verify.

## Context

The first macOS runs of the platform matrix (ADR 0074) failed in ways the Linux suite could
not show. `notify` 8.2's FSEvents backend (read in the vendored source) does three things the
per-directory registration of ADR 0047 did not allow for:

- every `watch` and `unwatch` stops the stream and builds a new one from "now"; events pending
  or occurring meanwhile are lost. Registering the directories of a copied-in tree one by one
  (hundreds of restarts) lost the events that announced the later directories, so they were
  never registered, and since a non-recursive watch only passes events of its direct children,
  edits in them were never delivered. This is a live-reload bug, not a test artifact.
- `watch` and `unwatch` key their bookkeeping by canonical path. Withdrawing a watch of a link
  removed the watch of its target.
- the stream is recursive in the kernel, so it also reports ignored subtrees, and events from
  just before the stream starts (history) reach the callback.

## Decision

- On macOS the first registration starts **one recursive watch of the resolved root**; every
  later `watch_one` only confirms through the root capability that a real directory stands
  there, and `unwatch` only updates the record. Nothing is restarted or removed afterwards. The
  `WatchSet` still records the eligible directories, so reconciliation, failures and the
  reports are unchanged. Linux keeps one non-recursive watch per directory.
- The bridge drops every change whose path the domain classifies as ineligible
  (`Ignored(Ineligible(_))`) on all platforms, so no source delivers a change inside an
  ignored subtree (contract case `ignored_subtree`) however the platform watches. Linux
  behaviour is unchanged: it never watched those directories.
- Platform facts that tests cannot share are gated with a reason: APFS cannot create a
  non-UTF-8 name, and `INDEX.HTML` is `index.html` on the default case-insensitive volume.

## Amendment (OPS-04 review)

The original text said the Quint model needed no change. That held for the watch set, but
not for the bridge: the drop of ineligible changes before the bounded channel is a change
on every platform, and `watcher.qnt` still queued `.git` and editor temporary churn, where
it took a channel slot and could overflow it. `emitOne` now drops a notification on an
ineligible path after the OS's own loss and before `offer`; `onlyEligibleTriggerReload` gains
the conjunct that no queued change names an ineligible path, `gitOnlyNoReload` expects an
empty channel (and no longer receives a batch: there is none), and `atomicSave` expects only
the rename onto the target queued. Without the drop, `gitOnlyNoReload` and `atomicSave` fail
and a random run (seed 7) violates the strengthened invariant.

macOS reports a superset of what the model and Linux report. With one recursive stream, a
change in an eligible directory that failed registration (no permission, too deep) is still
delivered on macOS, while on Linux, with no watch of its own, it is not. The watcher's health
is still derived from the `WatchSet` (degraded while the directory is outstanding), so
nothing claims more than Linux would; the extra notifications only cause reloads the model
leaves to the recovery's full reload, and an extra reload is harmless. This lies outside the
model (doc/formal-models.md).

## Consequences

- A macOS watcher costs one stream however many directories the site has, and cannot lose a
  registration to a restart. Changes in ignored directories wake the process before the
  bridge drops them.
- The watch-limit faults never occur on macOS.
- Raw `notify` notifications on macOS include ignored paths; adapter tests that assert on raw
  notifications say so where they skip that assertion.
