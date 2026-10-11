# 0004. Confined file access via rustix openat capability handle

Date: 2026-10-10

## Status

Accepted

## Context

plan.md §5.3 requires every served path component to be opened relative to the previous directory fd with `O_NOFOLLOW`. foundation.md §7 suggests `cap-std`. `cap-std` permits symlinks that stay inside the root, which plan.md §15 forbids.

## Decision

Implement a capability handle type (`RootDir`) in `preview-adapter-fs` over `rustix::fs::openat`. Intermediate components use `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`, and the final component uses `O_RDONLY|O_NOFOLLOW|O_CLOEXEC` plus an `fstat` regular-file check. Only `RootDir::open_root` uses ambient authority. `cap-std` is rejected.

## Consequences

- `unsafe_code = "forbid"` still holds (rustix is a safe API).
- This is HTTP-serving confinement, not a sandbox against same-user processes.
- Implemented by FS-01 in [`capability.rs`](../../crates/preview-adapter-fs/src/capability.rs); race-tested by FS-04; contract-tested by TEST-04.
- The opens also carry `O_NONBLOCK` (an intermediate, and the final component, cleared again on a regular file) so that a FIFO is refused instead of blocking the open, and the final open carries `O_NOCTTY` so that a terminal device never becomes the controlling terminal before it is refused.
- `ENXIO`, `ENODEV` and `EOPNOTSUPP` from the final open (a socket or a device without a driver) are refused as not regular, like any non-regular object `fstat` finds.
- Before the final open, an `fstatat(AT_SYMLINK_NOFOLLOW)` of the name refuses a character or block device, a FIFO or a socket as not regular (and a link as a link), so a device driver's `open` routine never runs for a request. It is a pre-filter only: the object can change between the two calls, so the `fstat` of the opened descriptor stays the authoritative check, and nothing the pre-filter saw is trusted afterwards.
- When an intermediate directory open fails with `ENOTDIR` (Linux and macOS may give it for a link as well as a file), an `fstatat(AT_SYMLINK_NOFOLLOW)` of that name classifies the refusal: a link is refused as a link, any other non-directory is `NotFound`, so `/index.html/x` does not reveal that `index.html` exists. The look opens nothing and never lets the walk continue; a directory seen there (a swap after the open) or a failed look keeps the link refusal.
- A name longer than `NAME_MAX` (`ENAMETOOLONG`) is `NotFound`, like any name that cannot exist, never an I/O failure and a server error.
