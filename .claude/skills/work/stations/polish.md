# polish

Dispatch: subagent (general-purpose)

## Inputs

Rust files in the diff: `git diff --name-only HEAD -- '*.rs'` plus untracked new `*.rs` files (`git ls-files --others --exclude-standard -- '*.rs'`).

## Prompt

Delete comments that:

1. restate the code (`// set the value`)
2. reference a ticket or a task (`// added for S7`)
3. record who uses the code (`// used by the SSE handler`)
4. mirror a doc comment or add nothing beyond the names
5. narrate obvious control flow
6. are commented-out code

Keep: `// given`, `// when`, `// then` markers; invariant comments (`// SAFETY`-style, ordering and locking rules, "subscribe before snapshot"); why-comments for non-obvious constraints; the reason on a bounded poll in a watcher test; doc comments (`///`, `//!`) that state behaviour and name the test that pins it. Check doc comments for stale behavioural claims and for a claim with no test named. Do not change behaviour. `cargo fmt --all` afterwards, then the gate once more.

## Output

```
{ commentsRemoved: int, files[], checkPassed: bool, issues[] }
```

## Sensors

`cargo fmt --all --check` exit 0 and the gate exit 0.

## Gates

To review: MEDIUM or LARGE. To qa: SMALL.
