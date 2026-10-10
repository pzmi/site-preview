# 0021. Allowed Host authorities

Date: 2026-10-10

## Status

Proposed (planner default — owner may override)

## Context

plan.md §5.4 allows a narrowly defined `localhost` alias and numeric addresses in remote mode, to mitigate DNS rebinding.

## Decision

Allow the bound numeric IP with the bound port, plus `localhost:<port>` only when the listener is loopback. In `--allow-remote` mode allow only numeric IPs (the bound IP; any numeric IP for a wildcard listener). The port must always match. Anything else gets 421.

## Consequences

- DNS names other than loopback `localhost` never reach handlers.
