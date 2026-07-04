# Conventions

Project-wide style and API conventions for the CRW engine. These are enforced
in CI (`.github/workflows/ci.yml`) and by the opt-in pre-commit hook
(`git config core.hooksPath .githooks`). Where a convention is machine-checked,
the check is named below.

## Identifier naming (Rust)

Standard Rust casing, enforced by `clippy`:

| Kind | Convention | Example |
|---|---|---|
| Functions, variables, modules | `snake_case` | `fetch_result` |
| Types, traits, enum variants | `PascalCase` | `ScrapeData` |
| Constants, statics | `SCREAMING_SNAKE_CASE` | `DEFAULT_MAX_INPUT_BYTES` |

## API JSON naming — **camelCase everywhere**

Every field a public API response serializes is **camelCase**. This is the
single most important cross-cutting rule.

- **Responses**: camelCase. Two deliberate Firecrawl-compatible exceptions with
  non-standard capitalization — `sourceURL` and `numPages` — have no underscore
  and are intentional; nothing else deviates.
- **Requests**: primary field names are camelCase. For Firecrawl compatibility
  we ALSO accept `snake_case` via `#[serde(alias = "...")]`, but that is input
  tolerance, not our convention — never rely on snake_case in docs or examples.
- **Verbatim external keys** (`metadata`'s meta-tag map: `og:site_name`,
  `twitter:creator`, …) are the site's own HTML names, passed through as-is.
  They are not typed fields and are exempt.
- **Error shape**: `{ "success": false, "error": "...", "errorCode": "..." }`.

**Enforced by:**
- `crates/crw-core/tests/api_casing.rs` — serializes the public response types
  and fails on any snake_case key. Add new response types there.
- `.github/workflows/openapi-check.yml` — the served `/openapi.json` must stay
  byte-equal to `docs/openapi.json`.

To add a response type: derive `#[serde(rename_all = "camelCase")]` (or rename
the offending field), update `crates/crw-server/openapi/openapi.json` +
`docs/openapi.json`, and add an instance to the casing guard.

## Formatting

`rustfmt` defaults. Run `cargo fmt --all`. **Enforced:** `cargo fmt --all -- --check`.

## Linting

`clippy` with warnings-as-errors. **Enforced:** `cargo clippy --workspace --all-targets -- -D warnings`.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/) — `feat:`, `fix:`,
`chore:`, `docs:`, `refactor:`, `test:`, `ci:`, `perf:`. `release-please` reads
these to compute versions and the changelog; never bump versions or edit
`CHANGELOG.md` by hand.

## API versioning

Paths are versioned (`/v1`, `/v2`). Response shapes are additive within a
version; the OpenAPI spec is the contract and is drift-checked against the
binary.
