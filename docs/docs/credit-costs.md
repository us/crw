# Credit Costs

:::note
Cloud only (fastcrw.com) -- self-hosted instances do not have credit-based billing.
:::

## Current billing rules

| Operation | Credit cost |
| --- | --- |
| `scrape` (any render — HTTP, lightpanda, chrome, or `chrome_proxy`) | 1 credit |
| `scrape` with `playwright` render | SaaS billing only; engine `data.creditCost` is omitted (0) |
| `scrape` with structured extraction (`formats: ["json"]` / `summary`) | 1 credit + LLM token cost |
| `map` | 1 credit |
| `crawl` start | 1 credit |
| `crawl` polling | New pages discovered since the previous poll |
| `search` | 1 credit |
| `search` + scrape | 1 credit + 1 per scraped result |
| `browse` session | 1 credit (planned cloud rate; the self-hosted `crw-browse` binary is free) |

Every renderer costs 1 credit per page — the engine's `credit_for` returns a flat 1 (`crw-renderer/src/lib.rs`), and `data.creditCost` matches the charge regardless of which renderer (HTTP, lightpanda, Chrome, chrome_proxy) actually served the page.
LLM-backed extraction (`json`/`summary` formats) is **not** a flat fee. It costs the 1-credit base render plus the LLM token cost. The open-source engine itself adds no surcharge beyond the 1-credit render cost — the LLM portion is applied by the managed cloud billing layer and tracked separately in `creditsUsed`.

:::note
**Managed-LLM billing (fastcrw.com cloud only)** — token usage for managed LLM features (`json`/`summary` extraction, `answer`, `summarizeResults`) is billed on top of the base credit by the cloud platform, not by the open-source engine. Self-hosted deployments have no billing layer and are unaffected.
:::

## Why crawl billing looks different

The crawl start reserves the job. Subsequent polls charge only for newly materialized pages, not for the total accumulated page count each time.

That prevents the same already-seen pages from being charged again and again just because you are checking progress.

## Simple Examples

| Scenario | Credit effect |
| --- | --- |
| One `scrape` request | 1 credit |
| One `map` request | 1 credit |
| Start one crawl job | 1 credit |
| Poll a crawl and receive 7 new pages | 7 additional credits |
| Poll again with no new pages | No new page credits |
| Search for "AI tools" with 5 results | 1 credit |
| Search + scrape 3 results | 1 + 3 = 4 credits |
| Search + scrape, 1 scrape fails | 1 + 2 = 3 credits (failed scrape refunded) |

## What Usually Does Not Consume Permanent Credits

The billing logic is designed to avoid charging you for requests that never become real usable work. Validation failures and certain upstream failures are refunded rather than treated like successful paid execution.

The safest way to confirm actual consumption is still the balance endpoint before and after a test.

## The `creditCost` response field

Every successful scrape response (v1 `/v1/scrape`) includes a `creditCost` field inside the `data` object:

```json
{
  "success": true,
  "data": {
    "markdown": "...",
    "metadata": { ... },
    "creditCost": 1
  }
}
```

The value reflects the renderer cost only (a flat 1 credit for every renderer). On the managed cloud the SaaS billing layer may charge additional credits for LLM features (extraction, summary), which are tracked separately in `creditsUsed` on v2 responses and in your account billing dashboard.

The field is omitted when its value would be 0 (internal paths that have not yet been priced).

## Balance check

Use `GET /api/v1/account/balance` (cloud only — fastcrw.com) with your API key to inspect included credits, purchased balance, and total available credits.

## Example Monitoring Pattern

A simple integration-safe pattern is:

1. read balance before a new workflow rollout,
2. run a bounded test batch,
3. read balance again,
4. compare expected consumption with actual consumption.

That is especially useful for crawl jobs because the start request and the page-materialization charges happen at different times.

## When To Watch Credits Closely

Watch credits closely when:

- you are polling crawl jobs at high frequency,
- many workers share one account balance,
- or you are benchmarking output quality across multiple target sites.

If request rate is the only thing you monitor, you can still be surprised by crawl-heavy usage. Pair billing checks with [rate limits](/docs/rate-limits) so throughput and credit consumption are interpreted together.

## Operational Advice

- Use small test batches before large crawls.
- Check balance before and after integration changes.
- Separate "request volume" monitoring from "credit consumption" monitoring; they are related but not identical.

## Common Mistakes

- Assuming every crawl poll re-bills the full job instead of only newly materialized pages.
- Launching large crawls before validating cost on a much smaller limit.
- Treating validation failures and refunded work as if they were successful billable jobs.
