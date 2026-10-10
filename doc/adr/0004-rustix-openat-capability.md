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
- Implemented by FS-01; race-tested by FS-04; contract-tested by TEST-04.
