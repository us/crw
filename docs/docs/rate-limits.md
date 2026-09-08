# Rate Limits

## Window

CRW uses a **token-bucket rate limiter** scoped to the server process. Tokens refill continuously at the configured rate (`rate_limit_rps`). A burst can drain the bucket immediately; tokens replenish every second proportional to elapsed time.

Self-hosted instances set `[server].rate_limit_rps` in their config (default: 10 RPS, 0 = unlimited). Cloud plans (fastcrw.com) enforce per-API-key plan limits instead.

## Concurrency is the public entitlement

Cloud plans are governed by two different caps, and they are not interchangeable:

- **Concurrency**: how many of your requests may be in flight (POSTed and not yet completed) at the same moment. This is what you are actually paying for when you pick a plan; it is the number that decides how much parallel work you can run.
- **Request-rate guard**: a secondary abuse control sized above ordinary use at the advertised concurrency. It is operational protection, not a per-plan product entitlement.

If you are designing for throughput, size your worker pool against the concurrency limit below. Internal abuse-control thresholds may change and are intentionally not published as a per-plan contract.

:::note
Cloud only (fastcrw.com) -- self-hosted instances can configure their own rate limits and have no concurrency cap of this kind.
:::

## Concurrency limits (simultaneous in-flight requests)

| Plan | Concurrent requests |
| --- | --- |
| FREE | 3 |
| HOBBY | 10 |
| STANDARD | 50 |
| GROWTH | 100 |
| SCALE | 150 |

A batch request (for example a batch scrape) counts as one POST for this limit: it does not itself grant extra parallelism beyond what the plan allows for other in-flight requests.

## Three `429` causes, and one `402`

A `429` response can mean three different things on the cloud API, and a `402` means a fourth. None of these currently carry a machine-readable `error_code` (or `errorCode`) field in the response body, only a human-readable `error` string. Do not write client logic that branches on an error code for these; branch on the HTTP status and, if you need to tell the causes apart, on the response headers noted below.

### Cause 1: Request-rate guard exceeded

You exceeded the Cloud request-rate guard. The response includes `Retry-After`, `X-RateLimit-Limit`, and `X-RateLimit-Remaining` headers. Treat those response headers as the authority for that request; do not hard-code an internal threshold.

**Action:** back off and retry. Apply exponential backoff starting at 1 second:

```ts
async function callWithRetry(fn: () => Promise<Response>, maxRetries = 5): Promise<Response> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    const res = await fn();
    if (res.status !== 429) return res;

    // Distinguish by header, not by a machine-readable error code (none exists
    // for any 429 cause today). Credit exhaustion carries an
    // X-FASTCRW-Credits-Available header; do not retry that one.
    if (res.headers.has("X-FASTCRW-Credits-Available")) {
      throw new Error("Credit balance exhausted, top up at fastcrw.com/billing");
    }

    const backoffMs = Math.min(1000 * 2 ** attempt, 30_000);
    await new Promise(r => setTimeout(r, backoffMs));
  }
  throw new Error("Max retries exceeded");
}
```

### Cause 2: Concurrency cap reached

You already have as many requests in flight as your plan allows. The response includes a `Retry-After` header and an `X-Concurrency-Limit` header naming your plan's cap.

**Action:** retry once an in-flight request completes, or reduce your worker pool's parallelism to stay under the cap. This is not a request-rate problem, so simply slowing down your request rate without also lowering concurrency will not fix it.

### Cause 3: Credits exhausted

Once your available balance reaches zero, the API returns `429`. This covers a FREE account that has spent its lifetime 500 credits (plus up to 500 more from onboarding tasks), and a paid account that is out of credits without an active auto-recharge attempt in progress. The response includes `X-FASTCRW-Credits-Available`, `X-FASTCRW-Included-Remaining`, `X-FASTCRW-Purchased-Remaining`, and `X-FASTCRW-Upgrade-Url` headers. There is no reset header: a FREE account's credits are a one-time lifetime grant, not a monthly allowance, so there is nothing to reset.

**Action:** do NOT retry. Retrying burns no credits (the request is rejected before processing) but creates noise in your logs and may hide the real cause. Alert or pause your pipeline and top up your balance or upgrade your plan.

### Cause 4: `402`, paid account with auto-recharge stopped

A paid account whose auto-recharge attempted to run and stopped (spending cap reached, card declined, bank confirmation required, or no payment method on file) gets `402`, not `429`. This is a distinct, retryable-after-payment signal. The response includes the same credit headers plus `X-FASTCRW-Stop-Reason` naming why auto-recharge stopped.

**Action:** do NOT retry as-is. Resolve the payment issue (raise the spending cap, update the card, authorize the charge) and retry after.

```ts
if (res.status === 429 && res.headers.has("X-FASTCRW-Credits-Available")) {
  // Alert your team, halt the pipeline.
  throw new Error("Credit balance exhausted, top up at fastcrw.com/billing");
}
if (res.status === 402) {
  const body = await res.json();
  throw new Error(`Auto-recharge stopped: ${body.error}`);
}
// Otherwise a 429 is request-rate or concurrency: backoff and retry (see above).
```

## Handling `503`

A `503 Service Unavailable` response means the server is up but a dependency is temporarily unavailable. Two cases:

| Source | `error_code` | Meaning |
| --- | --- | --- |
| `/ready` readiness probe | JSON body: `{"status":"degraded","renderers":{...}}` | A JS renderer (Lightpanda / Chrome) is down. The liveness endpoint `/health` still returns `200`. |
| `/v1/search` or `/firecrawl/v2/search` | `"search_disabled"` | Search is enabled in config but `search_backend_url` is not configured, or the backend is unreachable. |

**Action for 503:** retry with exponential backoff; this is a transient condition. If it persists beyond a few minutes, check renderer health at `/ready` or your search backend container.

```ts
if (res.status === 503) {
  // Transient — back off and retry, then alert if it persists.
  await sleep(backoffMs);
  return retryRequest();
}
```

## Practical Client Behavior

A well-behaved client should:

- inspect the response headers on every `429`/`402` to tell RPM, concurrency, and credit exhaustion apart (there is no machine-readable error code to branch on),
- use exponential backoff (not a fixed delay) for request-rate `429`, concurrency `429`, and `503` responses,
- NOT retry on a credit-exhaustion `429` or on a `402` (alert and halt instead),
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

- Retrying a credit-exhaustion `429` or a `402` -- the request will keep failing until you top up or fix the payment method.
- Assuming a `429` response carries a machine-readable `error_code` field -- it does not; branch on headers or HTTP status instead.
- Hard-coding an observed request-rate threshold instead of using response headers and the published concurrency entitlement.
- Confusing API plan limits with the target website's own anti-bot or rate-limit behavior.

For rollout work, pair this page with [credit costs](/docs/credit-costs) so request throttling and credit monitoring stay aligned.
