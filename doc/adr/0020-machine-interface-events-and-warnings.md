# 0020. JSONL events and warning format

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

plan.md §3 requires strict JSONL on stdout, a single `ready` event and a documented watcher status event. It also mentions structured warnings while sending warnings to stderr.

## Decision

In `--json` mode stdout carries only `{"event":"ready",url,pid,watching}` (once) and `{"event":"watcher",state,reason}` on changes. Warnings always go to stderr as `site-preview: warning[<code>]: <message>` with stable codes listed in the README.

## Consequences

- Agents parse stdout safely.
- DOC-05 checks that every emitted warning code is documented.
