# 0043. The `lol_html` rewriter: strict parsing, verified output, scoped MPL-2.0 exceptions

Date: 2026-10-11

## Status

Proposed (planner default — owner may override)

## Context

plan.md §6 (steps 4 to 6) requires the reload script to be inserted before the real `</body>`
end tag, never into script, style, comment, attribute or text content, never twice, and never
at the price of corrupted HTML; plan.md §2.3 names `lol_html` for the parse. `lol_html` is a
streaming tokenizer with a lightweight tree-builder simulator, not a full HTML tree builder:
a few non-conforming inputs are ambiguous to it, and an append at document end lands wherever
the tokenizer is (possibly inside an unterminated script or comment).

`lol_html` 3.0.1 is BSD-3-Clause, but its CSS selector engine depends on `cssparser`,
`cssparser-macros`, `dtoa-short` and `selectors`, which are MPL-2.0. `deny.toml` allowed only
permissive licences.

## Decision

- `preview-adapter-http` depends on `lol_html` 3.0.1 without default features (it has none
  but test and trace switches); `architecture.toml` allows it there with no features.
- The rewriter (`inject::rewrite`) pins `strict` mode, UTF-8, no ESI tags and no charset
  adjustment explicitly, so a library default change cannot loosen it. A parsing ambiguity or
  a memory-limit error is `Rewrite::Failed`.
- It inserts before the first body end tag the tokenizer emits (or the `</html>` that closes
  the body), skips one inside a closed `<template>`, and otherwise appends at document end.
- Every result is verified before it is returned: the output must equal the input with the
  tag inserted at one offset, and a second parse must see exactly one client `<script>`
  element. Either failure is `Rewrite::Failed`, so the caller serves the original bytes.
- "Already present" is a `<script>` element whose `src` is exactly `/__site_preview/client.js`.
- `deny.toml` accepts MPL-2.0 through `[[licenses.exceptions]]` for exactly those four crates,
  not through the global `allow` list.

## Consequences

- A page whose end lies inside an unterminated script, comment, `<textarea>`, `<style>` or
  `<plaintext>` is served without live reload and with a warning, instead of with a script
  tag that would show as text or change a script's source.
- The verification costs a second tokenizer pass over at most 8 MiB per injected response.
- MPL-2.0 applies per file to the unmodified upstream crates; their source is published on
  crates.io. Release packaging (OPS-05) must ship third-party licence notices that include
  them. Any other MPL-2.0 crate still fails `cargo deny`.
- `lol_html` brings a second `syn` major into the lock file (a `multiple-versions` warning).
