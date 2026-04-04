# Compatibility Matrix

## What "Compatible" Means Here

CRW is built around **Firecrawl-compatible workflows**, not a blanket "drop-in replacement" claim. The goal is to preserve the core request mental model so migrations are manageable, while still documenting the places where behavior differs.

## Supported alignment

| Area | Status |
| --- | --- |
| `/v1/scrape`, `/v1/crawl`, `/v1/map` route shape | Supported |
| `limit`, `maxPages`, `max_pages` for crawl caps | Supported |
| Numeric `waitFor` for JS rendering | Supported |
| `cssSelector`, `xpath`, `chunkStrategy`, `filterMode` | Supported |

## Known differences

| Area | Current behavior |
| --- | --- |
| Screenshot output | Not supported in this release |
| `success` semantics | `success: false` when target returns HTTP 4xx/5xx with minimal content; `success: true` with `warning` when target returns error status but has real content |
| JS waiting | Numeric delay only; no selector-based wait primitive |
| `extract` format | Accepted as alias for `json`. Use `formats: ["json"]` with `jsonSchema` for structured extraction |
| SDKs | Raw HTTP examples only, no official language SDK package |

Treat this page as the source of truth during migrations.

## Migration Checklist

If you are moving an existing Firecrawl-style integration:

1. verify `scrape`, `crawl`, and `map` request bodies against real targets,
2. confirm how your code interprets `success`, `warning`, and target-side HTTP statuses,
3. remove dependencies on unsupported features such as screenshots,
4. and compare output quality, not just endpoint shape.

Compatibility at the request level is useful, but output semantics and operational behavior matter just as much.

## Example Evaluation Workflow

A practical migration test usually looks like this:

1. take one production URL that already works in the old integration,
2. run it through [scrape](/docs/scraping) with the same high-level options,
3. compare markdown quality, warnings, and target status behavior,
4. repeat the test on one JS-heavy page and one failure-prone page,
5. only then update the calling code.

That sequence catches the gap between request-shape compatibility and output-quality compatibility.

## When Compatibility Is "Good Enough"

Compatibility is good enough when your migration goal is:

- preserving the existing mental model,
- keeping endpoint names and common options familiar,
- and minimizing changes in the application layer.

It is not good enough if your current system depends on features this page already marks as unsupported or behaviorally different.

## Common Mistakes

- Assuming route-name compatibility means output semantics are identical.
- Migrating a whole workload before testing `warning` handling and failure cases.
- Ignoring unsupported capabilities such as screenshots and then discovering the gap in production.

Use this page with [output formats](/docs/output-formats) and [error codes](/docs/error-codes) so your migration covers both payload shape and operational behavior.
