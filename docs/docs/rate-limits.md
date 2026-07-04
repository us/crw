# Rate Limits

## Window

CRW uses a **token-bucket rate limiter** scoped to the server process. Tokens refill continuously at the configured rate (`rate_limit_rps`). A burst can drain the bucket immediately; tokens replenish every second proportional to elapsed time.

Self-hosted instances set `[server].rate_limit_rps` in their config (default: 10 RPS, 0 = unlimited). Cloud plans (fastcrw.com) enforce per-API-key plan limits instead.

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

## Two distinct `429` branches

A `429` response means one of two different things. Handle them differently.

### Branch 1 — RPM rate limit exceeded

Your request rate exceeded the plan's requests-per-minute cap (or the self-hosted `rate_limit_rps` setting). The response body carries `error_code: "rate_limited"`.

**Action:** back off and retry. Apply exponential backoff starting at 1 second:

```ts
async function callWithRetry(fn: () => Promise<Response>, maxRetries = 5): Promise<Response> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    const res = await fn();
    if (res.status !== 429) return res;

    const body = await res.json();
    // Only retry on RPM rate limit, not credit exhaustion.
    if (body.error_code !== "rate_limited") throw new Error(body.error ?? "unknown error");

    const backoffMs = Math.min(1000 * 2 ** attempt, 30_000);
    await new Promise(r => setTimeout(r, backoffMs));
  }
  throw new Error("Max retries exceeded");
}
```

:::note
The open-source server does not currently send `Retry-After`, `X-RateLimit-Limit`, or `X-RateLimit-Remaining` response headers. Use client-side exponential backoff instead of reading a header.
:::

### Branch 2 — Credits exhausted (cloud only)

On fastcrw.com, once your account balance reaches zero the API returns `429` with `error_code: "insufficient_credits"`.

:::note
`insufficient_credits` is a fastcrw.com-only error code added by the SaaS billing layer. The open-source engine does not produce this code — it is not part of `CrwError::error_code()` in crw-core.
:::

**Action:** do NOT retry. Retrying burns no credits (the request is rejected before processing) but creates noise in your logs and may hide the real cause. Alert or pause your pipeline and top up your balance.

```ts
if (res.status === 429) {
  const body = await res.json();
  if (body.error_code === "insufficient_credits") {
    // Alert your team, halt the pipeline.
    throw new Error("Credit balance exhausted — top up at fastcrw.com/billing");
  }
  // Otherwise it's an RPM limit — backoff and retry (see above).
}
```

## Handling `503`

A `503 Service Unavailable` response means the server is up but a dependency is temporarily unavailable. Two cases:

| Source | `error_code` | Meaning |
| --- | --- | --- |
| `/ready` readiness probe | JSON body: `{"status":"degraded","renderers":{...}}` | A JS renderer (Lightpanda / Chrome) is down. The liveness endpoint `/health` still returns `200`. |
| `/v1/search` or `/firecrawl/v2/search` | `"search_disabled"` | Search is enabled in config but `searxng_url` is not configured, or SearXNG is unreachable. |

**Action for 503:** retry with exponential backoff; this is a transient condition. If it persists beyond a few minutes, check renderer health at `/ready` or your SearXNG container.

```ts
if (res.status === 503) {
  // Transient — back off and retry, then alert if it persists.
  await sleep(backoffMs);
  return retryRequest();
}
```

## Practical Client Behavior

A well-behaved client should:

- inspect `error_code` in every non-2xx response body before deciding whether to retry,
- use exponential backoff (not a fixed delay) for `429` RPM and `503` responses,
- NOT retry on `429` with `error_code: "insufficient_credits"` — alert and halt instead,
- centralize throttling when multiple workers share one API key.

The problem with per-worker retry logic is that it reacts too late. A central limiter prevents avoidable `429`s in the first place.

## When To Add Client-Side Throttling

Add a shared limiter before production when:

- multiple workers share one API key,
- one request can fan out into many crawl polls,
- or you are likely to burst after a queue drain or deploy.

## Rate Limits vs Target Limits

The CRW API rate limit is separate from the target website's own rate limit.

- CRW returns `429` (at the API layer) when your key exceeds plan limits or your balance is exhausted.
- The target site may return `429` to CRW's crawler; this appears in `metadata.statusCode` or as a `warning` field in the scrape response -- it is NOT an API-level `429`.

Those are different problems and should be handled differently.

## Common Mistakes

- Retrying on `429` with `error_code: "insufficient_credits"` -- the request will keep failing until you top up.
- Reading `Retry-After` or `X-RateLimit-Remaining` headers that the server does not send.
- Confusing API plan limits with the target website's own anti-bot or rate-limit behavior.

For rollout work, pair this page with [credit costs](/docs/credit-costs) so request throttling and credit monitoring stay aligned.
