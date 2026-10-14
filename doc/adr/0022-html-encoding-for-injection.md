# 0022. HTML charset labeling and injection eligibility

Date: 2026-10-10

## Status

Accepted (owner, 2026-10-14)

## Context

plan.md §5.1 and §6 forbid mislabeling non-UTF-8 HTML and forbid corrupting HTML to achieve live reload.

## Decision

Label HTML `text/html; charset=utf-8` and make it eligible for injection only when the bytes are valid UTF-8 and any `<meta charset>` agrees. Otherwise serve unchanged as `text/html` and warn once per path (`html-not-utf8`).

## Consequences

- Legacy-encoded pages preview without live reload.

## Amendments

- **2026-10-12 (HTTP-07).** "Any `<meta charset>`" means the first charset declaration the
  HTML tokenizer sees anywhere in the page (not in a comment or script text): a
  `<meta charset>` element or a `<meta http-equiv="content-type">` whose `content` names a
  charset (the WHATWG extraction algorithm). The whole page is scanned, because a browser
  that finds a declaration after its 1024-byte prescan still switches to it. A declaration
  that names no UTF-8 label, or that cannot be read, disagrees. Pages over 8 MiB are not
  read ahead, so they are labeled plain `text/html` and warned `html-too-large` instead
  (ADR 0046).
- **2026-10-12 (E2E-02 review).** A UTF-8 byte order mark at the start of the page decides
  first, as in a browser (WHATWG encoding sniffing puts the BOM ahead of the transport label
  and any `<meta>`): valid UTF-8 bytes after a BOM are labeled and injected whatever the
  page's `<meta>` declares.
