# 0047. The `notify` watcher: per-directory registration, failure reports, scoped CC0-1.0 exception

Date: 2026-10-12

## Status

Proposed (planner default — owner may override)

## Context

plan.md §2 and §2.1 choose `notify` 8.2 with `NonRecursive` watches on the eligible
directories only, so an ignored subtree is never registered with the operating system;
§8.1 steps 3 to 6 ask for one long-lived watcher, a record of the watched set, and a report
of exactly which directory failed and why, with HTTP kept up. `notify` registers by path and,
for a single watch, follows a symbolic link; the site's eligible directories come from a walk
through the root capability (FS-05, ADR 0004), which never follows one. `notify` 8.2.0 is
licensed CC0-1.0, which `deny.toml` did not allow. The latest `notify` is a 9.0 release
candidate, which plan.md §2.3 rules out without a reason.

## Decision

- `preview-adapter-fs` depends on `notify` 8.2.0 without default features and with
  `macos_fsevent` only (the default backend on macOS, FSEvents; inotify on Linux needs no
  feature); `architecture.toml` allows exactly that feature. Only `preview-adapter-fs` names
  `notify` (`ARCH-fs-authority`); no `notify` type appears in a port.
- `NotifyWatcher` registers `RecursiveMode::NonRecursive` for every directory `eligible_dirs`
  returns, parents first, and records successes in the domain `WatchSet`. The absolute path
  is built in one function from the resolved root and the directory's validated components.
- After each watch the directory is reopened through the root capability (`O_NOFOLLOW` per
  component); if a link or file now stands there the watch is withdrawn and reported. A link
  swapped in after the walk and back out between the watch and the check is not detected:
  the residual window is a local race at registration time, and its worst effect is
  notifications from the link's target reported under the eligible name, never a served file.
- A failure is a `RegistrationError { dir: RelPath, fault: WatchFault }`, displayed as
  ``cannot watch directory `<relative dir>`: <reason>``; walk errors (FS-05) are reported the
  same way, before registration failures. After the watch limit is reached no further watch is
  attempted, and every remaining directory is reported with `WatchLimitReached`. A watcher
  that cannot be created at all is an `Err(WatchFault)`; the caller reports it as a `Failed`
  batch so the coordinator degrades while HTTP keeps serving.
- `deny.toml` accepts CC0-1.0 for the `notify` crate only, as a scoped licence exception.

## Consequences

- On macOS, FSEvents streams are recursive by nature: `notify` filters a non-recursive
  watch's events to the directory's direct children, so ignored subtrees produce no
  notification, but the kernel stream still covers them. Each `watch` call restarts the
  stream, so start-up cost grows with the number of eligible directories.
- `notify` 8.2's FSEvents backend panics on a path that is not UTF-8 (registering one, or a
  notification naming one). A root whose resolved path is not UTF-8 is refused there as
  `WatchFault::Unavailable`; a non-UTF-8 name created inside the site is an upstream
  limitation, recorded here until `notify` handles it.
- FS-08 wraps the watcher in the `ChangeSource` adapter; FS-09 adds unwatching and
  re-registration through the same `WatchSet` and failure type.
- A new CC0-1.0 dependency elsewhere still fails `cargo deny`.
