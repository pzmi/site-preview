# 0050. Watch-set reconciliation: source-held hints, voided entries, in-place refresh, bounded retries

Date: 2026-10-12

## Status

Proposed (planner default — owner may override). Amended by ADR 0051: the pipeline runs
the reconcile under `block_in_place`, off the async workers.

## Context

plan.md §8.1 ("New directories") asks that a created or renamed-in eligible directory be
watched with all its eligible descendants, that a removed one lose its watches and that the
nearest existing parent be rescanned, and that atomic saves work; §8.4 allows only a small,
bounded retry. ADR 0049 left the composition hook `WatchControl::reconcile(scope)` with a scope
and no paths, and returning only `Option<usize>`, so a reconcile could neither say which
directory failed nor degrade the watcher. `inotify` watches an inode, not a name: `notify`
drops the watch of a directory that is deleted or moved away by itself, a directory created
again under the same name is unwatched, and the same inode watched under two names shares one
watch. File systems reuse a freed inode number at once, within one timestamp tick, so device,
inode and birth time cannot tell a replaced directory from the original.

## Decision

- **The source keeps the hints.** `NotifyChangeSource` turns each change it delivers into a
  hint when the domain says the directory structure may have changed
  (`Classification::requests_reconcile`): `Entry(path)` for a create, remove or rename,
  `Subtree(path)` for a change the OS did not attribute, `Full` for one naming the root or when
  more than 256 distinct hints pile up. The composition hook keeps its scope-only signature:
  `Changed` uses and clears the hints, `Full` walks everything. Ignored names never become
  hints, so their subtrees are never walked or watched.
- **Voided entries first.** For `Entry(path)` every watch recorded at or under the path is
  removed (an error from `notify`, which may have dropped it already, is ignored), and all
  removals of a call happen before any registration. Then the path is walked with
  `eligible_dirs` and registered through `NotifyWatcher::register`, whose `O_NOFOLLOW` check
  after each watch withdraws a link swapped in after the listing.
- **Nearest existing parent.** When the path no longer holds a directory, the nearest ancestor
  that exists (found through `WatchSet::nearest_existing_ancestor`, then confirmed through the
  capability) is listed one level deep: unwatched eligible children are registered with their
  subtrees, watched children that are gone are dropped with theirs. One `readdir` per removal,
  not a walk of the site.
- **Diffs refresh in place.** `Subtree` and `Full` apply the domain's `WatchSet::reconcile`
  diff and watch every kept directory again: for the same inode that is a no-op, and a
  directory replaced while its events were lost gets its own watch, with no moment unwatched.
  Each kept directory is then listed again, as each new one is after its watch (ADR 0047), so
  a child created in a replaced directory before its new watch existed is found. A directory
  the walk could not list keeps its watched descendants.
- **Failures and retries.** The watcher keeps the eligible directories it could not walk or
  watch, with the reason. A changes-only reconcile (one per batch that may change the
  directory structure, so one per atomic save) retries at most 16 of them, round robin; a full
  reconcile retries all. Never a nesting that is too deep; no loop, no timer. Once the watch
  limit is reached nothing is retried until one of the watcher's own watches is removed or a
  full reconcile probes the limit again, so a site far over the limit does not pay a walk and
  a failing `inotify_add_watch` per outstanding directory on every save. A failure is reported once, when it appears or its reason
  changes; a directory that vanished before it could be watched is not a failure, since its
  removal is a change of its own. A reconcile is complete only when nothing is outstanding.
- **Composition reports it.** `WatchControl::reconcile` returns `Reconciled { ready, problems }`.
  Each problem (relative path and reason, at most ten named, the rest counted) is warned as
  `watch-registration-failed`; problems without readiness degrade the watcher
  (`RegistrationFailed`, no further warning); `ready` is `Some(dirs)` only when every eligible
  directory is watched, and is reported to `watcher_ready`, so a degraded watcher recovers.

Rejected: passing paths through the hook (composition would have to classify again, and the
hook would carry adapter knowledge); comparing device and inode, or birth time, to detect a
replaced directory (inode reuse defeats it); unwatching and re-registering everything on a full
reconcile (a gap in which edits are missed); retrying failed directories on a timer (§8.4).

## Consequences

- A reconcile runs synchronously on the pipeline task. A full one walks every eligible
  directory and, where watches follow inodes (inotify; kqueue), watches and lists each again;
  it runs only after a loss or an unattributed change. A changes-only one costs at most 16
  retry walks beyond its hints.
- A watch limit freed by another process on the system is noticed at the next removal of one
  of this watcher's watches or the next full reconcile, not before. FSEvents (macOS) watches paths and restarts
  its stream on every `watch` call, so the in-place refresh is skipped there.
- Rename and delete of a watched directory, recreation under the same name, trees moved in
  from outside the root and `mkdir -p` are covered by real-OS tests in the adapter and one
  black-box reload test; E2E-03 extends the black-box suite.
- FORMAL-02's watcher model can treat a reconcile as: void the entries, walk the hinted
  subtrees, list the nearest existing parent, retry the outstanding set, then `Ready` only for
  an empty outstanding set.
