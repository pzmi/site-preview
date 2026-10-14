# 0017. Scope of reload-model conformance

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

Conformance must drive real code and must not reimplement it. The browser algorithm runs in JavaScript.

## Decision

Conformance against `reload.qnt` drives the real latest-state channel and SSE handler in-process and observes emitted events. Browser-side decisions are verified by `client.js` unit tests and Playwright, never by a Rust reimplementation of the client.

## Consequences

- Model invariants about browser counters are checked indirectly through server observations plus E2E.
