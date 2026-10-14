# 0012. Security and change policies are pure domain code

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

foundation.md says adapters contain no domain policy. plan.md requires one eligibility predicate shared by serving and watching. Security decisions must be easy to test, fuzz and mutation-test.

## Decision

Pure functions in `preview-domain` own: URL decoding to `SitePath`, component eligibility, Host and Origin policy, injection eligibility, debounce, change classification, change sets and watch-set reconciliation. Adapters translate headers, fds and notify events into domain inputs, and translate domain decisions back out.

## Consequences

- The domain needs no I/O, clock or runtime; time is an explicit input.
- Serving (APP-03) and watching (FS-05, DOM-05) cannot drift apart.
