# 0038. Application port shapes: synchronous content, async change source, no adapter types

Date: 2026-10-11

## Status

Accepted (owner, 2026-10-14)

## Context

foundation.md §2.3 asks for traits only at nondeterministic boundaries, generics where the
implementation is known at compile time, and object-safe async interfaces with explicit
ownership and cancellation contracts where `dyn` is needed. APP-01 defines the first three
ports. The domain's change event (DOM-05) is built in parallel and does not exist yet, and
`preview-application` may not depend on a runtime (ADR 0013).

## Decision

- `SiteContent` is synchronous, `Send + Sync` and object safe; queries take
  `&dyn SiteContent`. It returns owned handles (`Box<dyn ContentFile>`, `Read + Send`) that
  outlive the store's borrow, so an async caller moves them onto a blocking thread.
  Refusals are categories (`NotFound`, `Forbidden(reason)`, `Io(ErrorKind)`) with no path
  text. It does not apply eligibility: the caller does, with the domain predicate (ADR 0012).
- `ChangeSource` is asynchronous: `next(&mut self) -> impl Future + Send`, used through
  generics, not `dyn`. Its contract requires cancellation safety, so the consumer can race
  it against a deadline and shutdown. The batch is generic over the event type
  (`ChangeBatch<E>`, `type Event`) so the port does not wait for or duplicate DOM-05.
- `Clock` is synchronous and object safe: `now() -> Tick`, monotonic.
- No port names an HTTP, watcher, runtime or file-descriptor type; `tokio` is not a
  dependency until APP-02 needs `sync`.

Rejected: an async `SiteContent` (needs boxed futures for `dyn`, and file reads block
anyway); a boxed-future `ChangeSource` (an allocation per batch for a dispatch nobody needs);
a callback or channel type in the port (would name a runtime).

## Consequences

- TEST-03 fakes are a `SiteContent` store, a `Clock`, and a `ChangeSource` generic over or
  fixed to the event type; TEST-04 and TEST-05 test the documented contracts.
- APP-04, once DOM-05 lands, bounds its source with `ChangeSource<Event = ChangeEvent>`.
- If a runtime-selected source is ever needed, a boxed wrapper is added beside the trait.

## Amendments

- **2026-10-11 (APP-04 checkpoint review).** `ChangeBatch::Failed` carries a
  `SourceFailure::{RegistrationFailed, RuntimeFailure}` instead of the domain's
  `DegradeReason`. A source can no longer report `RevisionExhausted`, which only the
  application decides (ADR 0040) and which makes the coordinator inert for good; the
  application maps a `SourceFailure` to its `DegradeReason` itself. The testkit's
  `ScriptedHandle::fail` takes a `SourceFailure`.
