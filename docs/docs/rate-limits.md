# Rate Limits

## Window

CRW uses a **per 60-second sliding window** for API-key rate limiting. The window slides continuously -- it counts requests in the last 60 seconds from the current moment, not fixed calendar minutes.

That means a burst at `12:00:20` still affects what you can send at `12:01:00`. Think in rolling windows, not top-of-minute resets.

## Current per-plan limits

:::note
Cloud only (fastcrw.com) -- self-hosted instances can configure their own rate limits.
:::

| Plan | Requests / minute |
| --- | --- |
| FREE | 10 |
| HOBBY | 30 |
| STANDARD | 200 |
| GROWTH | 400 |
| SCALE | 600 |

## Example: Handling `429` Correctly

When the API limit is hit, the recovery path should be mechanical:

```ts
if (res.status === 429) {
  const retryAfter = Number(res.headers.get("Retry-After") ?? "1");
  await sleep(retryAfter * 1000);
  return retryRequest();
}
```

That is different from a target-site `429`. If the target site rate-limits you, the CRW request may still complete at the HTTP layer while reporting the target failure inside `metadata.statusCode` or `warning`.

## Headers

- `Retry-After`
- `X-RateLimit-Limit`
- `X-RateLimit-Remaining`

When you receive a `429`, back off for the number of seconds specified in `Retry-After` before retrying. Sending requests during the backoff period will not reset the window but will be rejected.

## Practical Client Behavior

A well-behaved client should:

- read `X-RateLimit-Remaining` on every response,
- reduce concurrency before it reaches zero,
- and honor `Retry-After` exactly when a `429` arrives.

If you are running many workers in parallel, centralize throttling instead of letting each worker discover the limit independently.

## When To Add Client-Side Throttling

Add a shared limiter before production when:

- multiple workers share one API key,
- one request can fan out into many crawl polls,
- or you are likely to burst after a queue drain or deploy.

The problem with per-worker retry logic is that it reacts too late. A central limiter prevents the avoidable `429`s in the first place.

## Rate Limits vs Target Limits

The CRW API rate limit is separate from the target website's own rate limit.

- CRW may return `429` because your API key exceeded plan limits,
- or the target site may return `429`, which appears in `metadata.statusCode` or as a warning.

Those are different problems and should be handled differently.

## Common Mistakes

- Assuming the window resets exactly at the top of the minute.
- Retrying immediately after a `429` instead of honoring `Retry-After`.
- Confusing API plan limits with the target website's own anti-bot or rate-limit behavior.

For rollout work, pair this page with [credit costs](/docs/credit-costs) so request throttling and credit monitoring stay aligned.
