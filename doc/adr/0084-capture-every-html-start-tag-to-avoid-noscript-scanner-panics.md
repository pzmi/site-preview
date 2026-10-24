# 0084. Capture every HTML start tag to avoid noscript scanner panics

Date: 2026-10-18

## Status

Proposed

## Context

Scheduled fuzz run 39107602968 found a panic in `lol_html` 3.0.1's dispatcher:
`Tag should be a start tag at this point`. The original input contains malformed
`noscript` markup; the reduced reproducer is `<noscript c=>`. The verification
parse can leave the fast scanner with a pending start-tag attribute request and
then supply an end tag. Strict mode does not turn this internal assertion into a
normal parsing error.

ADR 0043 requires safe injection or `Rewrite::Failed`, so the caller can serve the
original bytes. The fuzz target must continue checking arbitrary bytes without
catching panics or weakening its independent output oracle.

## Decision

Add a no-op universal element handler to the shared rewriter settings. Capturing
every start tag keeps tags that match no functional selector on the lexer path,
avoiding the faulty scanner transition. Apply it to insertion, snapshot refresh,
and verification through `settings_base`. Keep strict mode, byte preservation
checks and client-tag verification unchanged. The fuzz oracle retains its original
selectors and settings.

Retain both the exact crashing input and its reduced reproducer in unit tests,
and commit the reduced input as a fuzz seed. Both return `NotAnElement` with no
partial output exposed.

## Consequences

Malformed input falls back to the original page instead of panicking. No parser
fork, dependency change, panic catcher or target exemption is introduced.
Capturing otherwise uninteresting start tags adds parsing and callback work; this
is a correctness workaround for the pinned parser and can be revisited after an
upstream fix passes the regression tests and the independent fuzz oracle.
