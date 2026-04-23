# Credit Costs

:::note
Cloud only (fastcrw.com) -- self-hosted instances do not have credit-based billing.
:::

## Current billing rules

| Operation | Credit cost |
| --- | --- |
| `scrape` | 1 credit |
| `map` | 1 credit |
| `crawl` start | 1 credit |
| `crawl` polling | New pages discovered since the previous poll |
| `search` | 1 credit |
| `search` + scrape | 1 credit + 1 per scraped result |
| `browse` session | 1 credit (planned cloud rate; the self-hosted `crw-browse` binary is free) |

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
