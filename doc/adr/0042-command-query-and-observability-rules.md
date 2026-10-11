# 0042. Command, query and observability rules: selectors and syntax-level flow

Date: 2026-10-11

## Status

Proposed (planner default — owner may override)

## Context

foundation.md section 3.3 (bullets 4, 5 and 7) and section 2.4 require that application
command modules import no query handler, read-side implementation or HTTP DTO, that queries
neither mutate domain state nor call command handlers, and that observability cannot change
domain decisions. ARCH-09 enforces them on the `syn` collector of ADR 0035, which resolves
names but does not type-check: method calls are not resolved to a receiver type and values
are not tracked across functions. Section 3.3 also forbids reading a missing selector or an
unanalysable construct as a pass. `ReloadState::publish_next` takes `&self` (the latest-state
channel mutates through a shared handle), so "queries do not mutate" cannot be reduced to "queries take no
`&mut`".

## Decision

- **Selectors.** The command code of `preview-application` is its `commands` module (by
  module path in a target, or by file under `src/commands/`); the query code is `queries`
  likewise. When `preview-application` is a production member, its production code must have
  both modules and an `impl ReloadState`, or `ARCH-missing-selector` fails. A rename
  therefore fails instead of turning the rules into no-ops. Test code is not checked: a test
  may run a command to set up a query.
- **Through re-exports.** Name resolution also records every module of the target a path
  passes through. A path through `crate::queries` (for example `crate::Read`, re-exported
  from the crate root) names `crate::queries` even when the item is defined elsewhere.
- **`ARCH-command-isolation`.** Command code names nothing under `crate::queries` and no
  `axum`, `axum_core`, `http`, `http_body`, `hyper`, `tower`, `tower_http` or
  `preview_adapter_http` path. The HTTP part overlaps `ARCH-layer-isolation` on purpose, so
  each rule holds if the other is changed.
- **`ARCH-query-purity`.** Query code names nothing under `crate::commands`; takes no
  `&mut self` and no `&mut` of a type that resolves into `preview_domain` or into the
  application itself, or that is a generic parameter or `Self` (it may be state); and calls
  no `ReloadState` method outside the read allowlist `new`, `current`, `subscribe`. The
  forbidden set is every other method of the production `impl ReloadState` blocks, so a
  mutating method added later is forbidden without a rule change. Method calls are matched
  by name (also inside macro tokens), and type-qualified calls
  (`ReloadState::publish_next(..)`) by path.
- **`ARCH-observability-isolation`.** Domain production code names no `tracing`,
  `tracing_core`, `tracing_subscriber`, `tracing_log`, `log`, `metrics` or `opentelemetry`
  path. In domain and application production code, instrumentation (a path into `tracing`,
  `tracing_core` or `log`; a path, module or field with a segment `observability` or
  `metrics`; a local bound or assigned from either, in one function) must not appear in a
  decision: an `if` or `while` condition, a `match` scrutinee or guard, a `let .. else`
  initializer, a `return` value, or the tail of a function that returns a value. Only the
  value-producing parts of a block count, so a traced statement beside a decision passes.
  Code inside a module named `observability` or `metrics` is the instrumentation itself and
  is exempt from the flow part.

## Consequences

- The checks over-approximate: a method named like a `ReloadState` mutator on another type,
  a `&mut` port in a query, or a local named `metrics` is reported. The fix is to rename or
  restructure, never to loosen the rule (ADR 0035).
- They under-approximate data flow: a query that hands its state handle to a helper outside
  `queries`, an instrumentation value passed to another function or stored in a field, and
  decisions written inside a `macro_rules!` body are not seen. Behaviour tests remain the
  second line (foundation.md section 3.3), as recorded in `doc/architecture-rules.md`.
- The positive fixture carries a command that publishes and is traced, a query that reads
  and fills a non-state buffer, and an instrumentation module, so the rules are shown not to
  be merely over-restrictive (foundation.md section 3.4).
- Fixtures: `command-imports-query`, `command-names-http-dto`, `query-calls-command`,
  `query-mutates-state`, `instrumentation-drives-decision` and `missing-query-selector`.
