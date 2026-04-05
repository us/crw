# SDK Examples

## TypeScript

```ts
const res = await fetch("https://fastcrw.com/api/v1/scrape", {
  method: "POST",
  headers: {
    Authorization: "Bearer YOUR_API_KEY",
    "Content-Type": "application/json",
  },
  body: JSON.stringify({ url: "https://example.com", formats: ["markdown"] }),
});

if (!res.ok) {
  throw new Error(`CRW error: ${res.status}`);
}

const payload = await res.json();
console.log(payload.data?.markdown);
```

This is enough for most Node.js backends, server actions, and edge handlers.

## Python

```py
import requests

res = requests.post(
    "https://fastcrw.com/api/v1/scrape",
    headers={
        "Authorization": "Bearer YOUR_API_KEY",
        "Content-Type": "application/json",
    },
    json={"url": "https://example.com", "formats": ["markdown"]},
)

res.raise_for_status()
payload = res.json()
print(payload["data"]["markdown"])
```

## Go

```go
package main

import (
  "bytes"
  "fmt"
  "io"
  "net/http"
)

func main() {
  body := []byte(`{"url":"https://example.com","formats":["markdown"]}`)
  req, _ := http.NewRequest("POST", "https://fastcrw.com/api/v1/scrape", bytes.NewBuffer(body))
  req.Header.Set("Authorization", "Bearer YOUR_API_KEY")
  req.Header.Set("Content-Type", "application/json")

  res, err := http.DefaultClient.Do(req)
  if err != nil {
    panic(err)
  }
  defer res.Body.Close()

  b, _ := io.ReadAll(res.Body)
  fmt.Println(string(b))
}
```

## Search

### TypeScript

```ts
const res = await fetch("https://fastcrw.com/api/v1/search", {
  method: "POST",
  headers: {
    Authorization: "Bearer YOUR_API_KEY",
    "Content-Type": "application/json",
  },
  body: JSON.stringify({ query: "web scraping tools 2026", limit: 5 }),
});

if (!res.ok) {
  throw new Error(`CRW error: ${res.status}`);
}

const { data } = await res.json();
```

### Python

```python
resp = requests.post(
    "https://fastcrw.com/api/v1/search",
    headers={"Authorization": "Bearer YOUR_API_KEY"},
    json={"query": "web scraping tools 2026", "limit": 5},
)

resp.raise_for_status()
data = resp.json()["data"]
```

### Go

```go
body := `{"query":"web scraping tools 2026","limit":5}`
req, _ := http.NewRequest("POST", "https://fastcrw.com/api/v1/search",
    strings.NewReader(body))
req.Header.Set("Authorization", "Bearer YOUR_API_KEY")
req.Header.Set("Content-Type", "application/json")

resp, err := http.DefaultClient.Do(req)
if err != nil {
    panic(err)
}
defer resp.Body.Close()
```

### Search + Scrape

Add `scrapeOptions` to fetch page content for each result in one call:

```ts
const res = await fetch("https://fastcrw.com/api/v1/search", {
  method: "POST",
  headers: {
    Authorization: "Bearer YOUR_API_KEY",
    "Content-Type": "application/json",
  },
  body: JSON.stringify({
    query: "machine learning papers",
    limit: 3,
    scrapeOptions: { formats: ["markdown"] },
  }),
});
```

## Crawl Polling Pattern

Every language follows the same basic loop:

1. `POST /crawl` to start the job.
2. Read the returned crawl id.
3. `GET /crawl/:id` until the job reaches a terminal state.

That is why the examples here stay close to raw HTTP instead of pretending there is an official SDK package.

## When To Use Raw HTTP

Raw HTTP is the right default when you are:

- already inside a backend service that owns retry logic,
- calling CRW from a queue worker or cron job,
- or integrating through an environment where extra SDK abstraction does not buy much.

That includes server actions, background jobs, serverless functions, and internal APIs that just need a predictable request shape. The main advantage is operational clarity: what you send over the wire is exactly what CRW receives.

## Example: Start a Crawl and Poll It

The same shape works in every language:

```ts
const start = await fetch("https://fastcrw.com/api/v1/crawl", {
  method: "POST",
  headers: {
    Authorization: "Bearer YOUR_API_KEY",
    "Content-Type": "application/json",
  },
  body: JSON.stringify({ url: "https://example.com/docs", limit: 10 }),
});

const { id } = await start.json();

const status = await fetch(`https://fastcrw.com/api/v1/crawl/${id}`, {
  headers: { Authorization: "Bearer YOUR_API_KEY" },
});
```

If your app already has a standard HTTP client with auth, tracing, and retries, adding a dedicated SDK layer too early usually just hides useful operational details.

## Common Mistakes

- Do not treat `response.ok` as the full success signal. You still need to inspect `warning` and `metadata.statusCode`.
- Do not hardcode retry timing for rate limits. Read `Retry-After` from the API response.
- Do not jump straight into crawl jobs before validating a single page through [scrape](/docs/scraping) or [getting started](/docs/quick-start).

## What To Read Next

- Use [rate limits](/docs/rate-limits) before adding parallel workers.
- Use [error codes](/docs/error-codes) when you need machine-readable failure handling.
- Use [output formats](/docs/output-formats) when you are deciding between markdown, html, and structured extraction output.

## Production Advice

- Load the API key from an environment variable.
- Respect `Retry-After` on `429`.
- Log `warning` and `metadata.statusCode`, not just the HTTP status.
- Start with `scrape` before wiring in `crawl` or extraction.
