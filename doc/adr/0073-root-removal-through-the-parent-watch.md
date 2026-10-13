# 0073. A root's removal is notified through a watch on its parent

Date: 2026-10-13

## Status

Proposed (planner default — owner may override)

Amends [ADR 0067](0067-root-removal-and-watcher-review-fixes.md) (how a root removal reaches
a reconcile).

## Context

ADR 0067 makes every reconcile check that the root path still names the served directory, and
degrade with `RootRemoved` when it does not. The check only runs when something asks for a
reconcile. `should_report_root_removed_once_when_site_root_is_deleted` (the binary) timed out
at 30 s in gate runs under heavy load, and failed 2 times in 30 under 12 busy processes on 4
CPUs.

The cause is in the kernel. The server holds the root's descriptor for its whole life (plan.md
section 5.3). An open descriptor pins the directory's dentry, and inotify reports
`IN_DELETE_SELF` and `IN_IGNORED` only when the inode is released. So `rmdir` of the root sends
nothing at all to the root's own watch while site-preview runs (checked on Linux 6.18: with a
descriptor held, a watched directory's removal gives only the `IN_DELETE` of its last entry;
the `IN_DELETE_SELF` comes when the descriptor is closed). `rm -r <root>` is two steps for
the watcher: the removals of the entries below the root, each notified and each asking for a
reconcile, then the root's `rmdir`, notified to nobody. When the pipeline ran its reconcile
between the two steps (the root still standing, empty), nothing ever asked again and live
reload stayed claimed for a root that was gone. Without load the pipeline usually lagged
behind `rm`, so the race was rarely lost. Under load `rm` was preempted between its last
`unlinkat` and the root's `rmdir`.

The model had the same blind spot: its `rootDelete` was one atomic step that notified the
root's own removal (`KRemove` on `Root`) through the root's watch.

## Decision

- **On Linux the root's parent directory is watched too**, non-recursively, in the same
  `notify` watcher, outside the `WatchSet` (it is not a site directory and is never counted,
  unwatched or reconciled). The parent reports the root entry's `IN_DELETE` (or
  `IN_MOVED_FROM`, or `IN_CREATE` when a directory is made at the path again) with the root's
  own path. The bridge's `translate` makes that a change on the root (`DirectoryChanged` for a
  directory removed or created), which asks for a reconcile, and that reconcile finds the root
  removed. Every other entry of the parent lies outside the root, and `translate` drops it
  before anything is queued, so no name from outside the root is kept, logged or sent.
- **Best effort.** When the parent cannot be watched (no permission to read it, the watch
  limit), or the root is `/`, nothing is reported for it, and a root removal is found by the
  next reconcile some other change asks for, as before. A withdrawal of a link that took the
  parent's watch (`inotify` hands back the existing watch for a link to the same inode)
  restores it in `repair`, as it does for the site's directories.
- **Not on macOS.** FSEvents streams are recursive by nature, so a watch on the parent (often
  the home directory) would stream all of it; and FSEvents reports by path, so a held
  descriptor does not hide the root's removal. OPS-03 verifies it.
- **Model first.** `watcher.qnt` splits `rm -r` of the root into `rootEmpty` (the entries,
  notified through their directories' watches) and `rootDelete` (the root, notified only
  through the parent's watch, `emitParent`); `rootRecreate` is notified the same way. The run
  `rootRemovedAfterEmptyReconcile` puts the pipeline's reconcile between the two steps. With
  the root's removal notified by nothing, that run, `rootDeleteRecreate` and
  `rootRemovalEscalates` fail, and a random run (seed 7) violates `noLiveClaimWithoutRoot`.
  A parent that cannot be watched is outside the model.

Rejected: polling the root's identity on a timer (plan.md section 8.4 has no polling fallback,
and it would wake an idle process for ever); a reconcile at every settle (narrows the window
to one debounce period but does not close it, since `rm` may still be preempted for longer);
not holding the root's descriptor (plan.md section 5.3 serves through it).

## Consequences

- `rm -r` of the root reports `root-removed` whatever the timing, on Linux.
  `should_notify_root_removal_when_emptied_root_is_removed` fails without the parent watch,
  and `should_keep_watching_root_parent_when_a_link_to_it_is_withdrawn` fails without its
  repair. Under the same load the binary's test failed 0 times in 30.
- One more inotify watch per server. The parent's other traffic (often a busy directory such
  as a home directory) wakes the native watcher's thread and is dropped there; it never reaches
  the channel. That includes the root's own `IN_OPEN` and `IN_CLOSE_NOWRITE`, which the parent
  reports for the root's path whenever the root directory is opened (a reconcile's walk, a
  listing): `translate` makes them access-only changes, which the bridge drops before the
  channel. It shares the instance's kernel queue, so a very busy parent makes a queue
  overflow (a full reconcile and a full reload) a little more likely.
- `notify` drops its own watch of the root when the parent reports the root removed or moved
  away; the watcher has degraded for good by then, so nothing is lost.
- `mv site x && mv x site` while site-preview runs is reported by the parent as two changes
  on the root, so a full reconcile follows. Run after both moves, it finds the same directory
  at the path, keeps live reload and re-watches the root in place
  (`should_keep_watching_root_when_it_is_renamed_away_and_back`); run between them, it finds
  the root removed and degrades for good, as for any move away (ADR 0067).
- The bounded check's depth table changes: a root deletion is two steps now, so the watcher's
  bounded run reaches `reachesRootDeleted` but no longer `reachesRootRemoved` (3 steps).
