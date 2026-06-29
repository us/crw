<div class="page-intro">
  <div class="page-kicker">More APIs</div>
  <h1>Monitoring</h1>
  <p class="page-subtitle">Schedule recurring scrapes or crawls, detect when a page actually changes, and notify your agent by signed webhook or email — with a structured diff and an optional LLM judge that filters out noise. A self-hostable, Firecrawl-compatible alternative to <code>/monitor</code>.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Best for:</strong> change detection on pages you rely on</div>
    <div class="page-capability"><strong>Hosted:</strong> fastcrw.com (full scheduler + notifications)</div>
    <div class="page-capability"><strong>Self-hosted:</strong> changeTracking primitive + optional <code>monitor</code> mode</div>
    <div class="page-capability"><strong>Start with:</strong> one scrape target, daily schedule</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="https://fastcrw.com/register" target="_blank" rel="noopener">Get API Key</a>
    <a class="page-btn secondary" href="#crawling">View Crawl</a>
  </div>
</div>

## What this is for

Use monitoring when you need to know the moment a page changes and only care about the changes that matter — competitor pricing, product catalogs, job listings, docs, changelogs, research papers, or government filings. A monitor runs scheduled scrapes or crawls, diffs each result against the last retained snapshot, classifies every page (`same`, `new`, `changed`, `removed`, or `error`), and delivers a structured diff. The output is just the change, so your agent ingests far fewer tokens than re-scraping everything.

Reach for `monitoring` instead of polling `/v1/scrape` yourself when you want the schedule, snapshot storage, diffing, retries, and noise filtering handled for you.

:::note
**Self-hosted users**: the full scheduler + notification control plane is part of the hosted product. The open-core engine ships the **stateless `changeTracking` primitive** (diff one scrape against a snapshot you supply) plus an optional, feature-gated **`monitor` mode** (SQLite scheduler, default OFF). See [self-hosting monitoring](#monitoring) below.
:::

:::info
**Two-namespace split**

**Monitor CRUD** (`/v1/monitor` and all sub-resources) is a **SaaS-only control-plane feature** served at `https://fastcrw.com/api`. It is not present in the open-source engine — no `/v1/monitor` route ships in `crw-server`.

**Engine endpoints** (scrape, crawl, map, search, change-tracking) are served at `https://api.fastcrw.com` on the hosted product and at your own origin when self-hosting.

**Self-hosted installs** have no monitor API. Use the stateless `changeTracking` primitive or build the engine with the optional `monitor` Cargo feature (SQLite scheduler) as described in [Self-hosting monitoring](#self-hosting-monitoring) below.
:::

## Endpoints

All monitor endpoints require a Bearer API key on the hosted API.

```http
POST   /v1/monitor                          # create
GET    /v1/monitor                          # list
GET    /v1/monitor/{id}                      # get
PATCH  /v1/monitor/{id}                      # update
DELETE /v1/monitor/{id}                      # delete
POST   /v1/monitor/{id}/run                  # run now (409 if a check is in flight)
GET    /v1/monitor/{id}/checks               # list checks
GET    /v1/monitor/{id}/checks/{checkId}     # get one check + its pages
```

Base URL: `https://fastcrw.com/api` (hosted).

## Create a monitor

Describe what to watch and how often. A `goal` enables the LLM judge so you are only alerted on meaningful changes.

:::tabs
```bash
curl -s -X POST "https://fastcrw.com/api/v1/monitor" \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Pricing monitor",
    "schedule": { "text": "every 30 minutes", "timezone": "UTC" },
    "goal": "Alert when a pricing tier, price, or headline feature changes.",
    "targets": [
      { "type": "scrape", "urls": ["https://example.com/pricing"] }
    ],
    "notification": {
      "email": { "enabled": true, "recipients": ["alerts@example.com"], "includeDiffs": true }
    }
  }'
```

```javascript
const res = await fetch("https://fastcrw.com/api/v1/monitor", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${process.env.CRW_API_KEY}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({
    name: "Pricing monitor",
    schedule: { text: "every 30 minutes", timezone: "UTC" },
    goal: "Alert when a pricing tier, price, or headline feature changes.",
    targets: [{ type: "scrape", urls: ["https://example.com/pricing"] }],
    notification: {
      email: { enabled: true, recipients: ["alerts@example.com"], includeDiffs: true },
    },
  }),
});
const { data } = await res.json();
console.log(data.id, data.nextRunAt);
```

```python
import os, requests

res = requests.post(
    "https://fastcrw.com/api/v1/monitor",
    headers={"Authorization": f"Bearer {os.environ['CRW_API_KEY']}"},
    json={
        "name": "Pricing monitor",
        "schedule": {"text": "every 30 minutes", "timezone": "UTC"},
        "goal": "Alert when a pricing tier, price, or headline feature changes.",
        "targets": [{"type": "scrape", "urls": ["https://example.com/pricing"]}],
        "notification": {
            "email": {"enabled": True, "recipients": ["alerts@example.com"], "includeDiffs": True}
        },
    },
)
print(res.json()["data"]["id"])
```
:::

The response returns the monitor with its normalized cron, computed `nextRunAt`, and `estimatedCreditsPerMonth` (an upper bound when judging is enabled). When the monitor has a webhook, the signing secret is returned **once** here as `webhookSecret`.

```json
{
  "success": true,
  "data": {
    "id": "019df960-06e7-7383-9d89-82c0113dc31a",
    "name": "Pricing monitor",
    "status": "active",
    "schedule": { "cron": "*/30 * * * *", "timezone": "UTC", "text": "every 30 minutes" },
    "nextRunAt": "2026-05-30T16:00:00.000Z",
    "lastRunAt": null,
    "currentCheckId": null,
    "goal": "Alert when a pricing tier, price, or headline feature changes.",
    "judgeEnabled": true,
    "targets": [
      { "type": "scrape", "urls": ["https://example.com/pricing"], "changeMode": "markdown" }
    ],
    "webhook": null,
    "notification": { "emails": ["alerts@example.com"], "includeDiffs": true },
    "retentionDays": 30,
    "estimatedCreditsPerMonth": 2880,
    "lastCheckSummary": null,
    "createdAt": "2026-05-30T15:30:00.000Z",
    "updatedAt": "2026-05-30T15:30:00.000Z"
  }
}
```

## Schedules

Provide a schedule as cron **or** as natural-language `text`. The minimum interval is 15 minutes; responses always return the normalized cron. `timezone` is an IANA zone (DST-correct — "daily at 9am" tracks 9:00 wall-clock across transitions). Text schedules are spread by monitor id so many monitors don't all fire at the same instant.

Supported natural-language forms: `every 30 minutes`, `every 15 minutes starting at :07`, `hourly`, `every 2 hours`, `daily`, `daily at 9:00`, `daily at 9am`, `daily at 5:30 PM`, `weekly`.

## Targets

Each monitor takes 1–50 targets:

- **`scrape`** — runs one scrape per URL in `urls` (≤50 distinct URLs across all targets).
- **`crawl`** — runs a full crawl for `url` on each check, then diffs every discovered page. Use `maxPages` to bound cost.

`scrapeOptions` / `crawlOptions` pass through to the underlying jobs. Monitor scrapes always fetch fresh.

## Goals and judging

Add a plain-language `goal` to be alerted only on meaningful changes. When `goal` is set and `judgeEnabled` is omitted, judging is enabled automatically. The judge runs only on **changed** pages and returns a judgment with `meaningful`, `confidence` (`low` / `medium` / `high`), `reason`, and `meaningfulChanges`. Set `judgeEnabled: false` to store a goal without judging.

## Change tracking modes

By default each page's markdown is diffed and reported as `same` / `changed` / `new` / `removed` / `error`. To track specific structured fields, set a `changeMode` on the target:

- **`markdown`** (default) — a unified text diff plus a parse-diff-style AST.
- **`json`** — supply a `jsonSchema`; CRW extracts those fields each check and emits a per-field diff keyed by JSON path (`plans[0].price → {previous, current}`) plus a full `snapshot`.
- **`mixed`** — both surfaces; a page is `changed` if **either** the markdown or a tracked field changed.

## Notifications

### Webhooks

Configure a `webhook` to receive two events:

- **`monitor.page`** — sent as each monitored page finishes; includes `isMeaningful` + `judgment` when judging ran.
- **`monitor.check.completed`** — sent after the full check reconciles, with summary counts.

Deliveries are signed (`X-CRW-Signature: t=<unix>,v1=<hmac-sha256>` over `<t>.<body>`), support custom headers + metadata and per-event subscription, retry with backoff, and dead-letter after repeated failures (with a one-time failure email). Webhook URLs are SSRF-guarded (https-only, private/loopback/metadata ranges blocked).

```json
{
  "webhook": {
    "url": "https://example.com/webhooks/crw",
    "events": ["monitor.page", "monitor.check.completed"],
    "headers": { "Authorization": "Bearer your-secret" },
    "metadata": { "environment": "production" }
  }
}
```

### Email

Email summaries are sent only when a check has `changed` / `new` / `removed` / `error` pages. With a goal + judging, noise-only checks are suppressed. New recipients receive a confirmation link (double opt-in) before any alert; up to 25 confirmed recipients per monitor. Set `includeDiffs: true` to embed the diff in the message.

## Check results

List checks with `GET /v1/monitor/{id}/checks` (filter by `status`: `queued`, `running`, `completed`, `failed`, `partial`, `skipped_overlap`) and inspect one with `GET /v1/monitor/{id}/checks/{checkId}`. Both auto-paginate via an opaque `next` cursor. A check detail returns `estimatedCredits`, `actualCredits`, summary counts, and a paginated `pages[]` array; each changed page carries inline `diff` data and (json mode) a `snapshot`.

```bash
curl "https://fastcrw.com/api/v1/monitor/$MONITOR_ID/checks/$CHECK_ID?status=changed" \
  -H "Authorization: Bearer $CRW_API_KEY"
```

## Parameters

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | string | required | Human-readable monitor name |
| `schedule.cron` | string | -- | Cron expression (provide this or `schedule.text`) |
| `schedule.text` | string | -- | Natural-language schedule (e.g. `"every 30 minutes"`) |
| `schedule.timezone` | string | `"UTC"` | IANA timezone for text/cron evaluation |
| `goal` | string | -- | Plain-language alert intent; enables the judge (≤2 KB) |
| `judgeEnabled` | boolean | auto | Force judging on/off; auto-on when `goal` is set |
| `targets` | object[] | required | 1–50 targets (`scrape` or `crawl`) |
| `targets[].type` | string | required | `"scrape"` or `"crawl"` |
| `targets[].urls` | string[] | -- | Scrape target URLs (≤50 distinct across targets) |
| `targets[].url` | string | -- | Crawl target root URL |
| `targets[].changeMode` | string | `"markdown"` | `markdown`, `json`, or `mixed` |
| `targets[].jsonSchema` | object | -- | Fields to track in `json` / `mixed` mode |
| `targets[].maxPages` | number | `1000` | Crawl page cap |
| `webhook` | object | -- | Signed webhook config (see Notifications) |
| `notification.email` | object | -- | `{ enabled, recipients[], includeDiffs }` |
| `retentionDays` | number | `30` | Snapshot/check retention (1–365) |

## Pricing

Monitors have no per-monitor fee. Each check pays for the scrapes or crawl it performs, plus an optional judge credit per changed page.

| Component | Credits |
| --- | --- |
| Scrape monitor | 1 credit per URL per check |
| Crawl monitor | 1 credit per discovered page per check |
| Meaningful-change judging | +1 credit per changed page the judge validates |

Checks with no changed pages use no judge credits. When a monitor runs out of credits its checks pause (`paused_no_credits`) and resume automatically once the balance recovers.

## Self-hosting monitoring

The open-core engine gives self-hosters the building blocks:

- **`changeTracking` scrape format** — add it to `/v1/scrape` `formats` with the diff `modes` and a `previous` snapshot you persist between checks. opencore is stateless: it returns the diff + the new snapshot for you to store.
- **`POST /v1/change-tracking/diff`** — diff a page (or a batch) against a supplied `previous` snapshot. The workhorse for crawl-based monitoring.
- **Optional `monitor` mode** — build the engine with the `monitor` Cargo feature (default OFF) for a SQLite-backed scheduler, set-level `new`/`removed`, an LLM judge, and signed local webhooks, with no external database.

```bash
# diff the current scrape against your stored snapshot
curl -s -X POST "http://localhost:3000/v1/change-tracking/diff" \
  -H "Content-Type: application/json" \
  -d '{
    "modes": ["gitDiff"],
    "previous": { "markdown": "Starter $19", "contentHash": "abc" },
    "current":  { "markdown": "Starter $24" }
  }'
```

:::note
The `monitor` feature pulls in SQLite/HMAC dependencies only when enabled — the default engine build stays dependency-light. Self-host monitoring uses UTC schedules and your own LLM key (BYOK) for judging.
:::

## Change Tracking

Change tracking is a **stateless** primitive: you supply the current scraped content and the previous snapshot; the engine returns the diff and the new snapshot to persist as the next baseline. The engine stores nothing.

Two entry points exist depending on whether you are running a single-page scrape or driving a custom orchestration loop:

| Entry point | When to use |
| --- | --- |
| `"changeTracking"` in `formats` on `/v1/scrape` | Single-URL scrape + diff in one call |
| `POST /v1/change-tracking/diff` (engine) | Batch diffs, custom crawl loops, separate fetch and diff steps |

### `changeTracking` format on `/v1/scrape`

Add `"changeTracking"` to `formats` and pass a sibling `changeTracking` object with the options. The alias `"change-tracking"` is also accepted on the wire.

```json
{
  "url": "https://example.com/pricing",
  "formats": ["markdown", "changeTracking"],
  "changeTracking": {
    "modes": ["gitDiff"],
    "previous": {
      "markdown": "Starter $19\n\nPro $49",
      "contentHash": "a3f8..."
    }
  }
}
```

The `changeTracking` field is **not** valid as an object inside the `formats` array — it must be the plain string `"changeTracking"` there; options ride on the top-level sibling field.

#### `changeTracking` options

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `modes` | string[] | `["gitDiff"]` | Diff surfaces to compute. `"gitDiff"` = markdown unified diff + AST; `"json"` = per-field structured diff; `["gitDiff","json"]` = mixed (both). Alias `"git-diff"` accepted on input; canonical serialization is `"gitDiff"`. |
| `previous` | object | — | Prior `ChangeTrackingSnapshot` to diff against. `null` or omitted = first observation. |
| `schema` | object | — | JSON Schema describing fields to track (`json` / `mixed` mode). |
| `prompt` | string | — | Natural-language extraction prompt (alternative to `schema`; `json` / `mixed` mode). |
| `tag` | string | — | Opaque caller label echoed back on the result (e.g. a target or monitor ID). |
| `contentType` | string | — | MIME type of the fetched resource. Binary types (`application/pdf`, `image/*`, `application/octet-stream`) are hashed by extracted text only — no text diff is produced. Unset = treat as text. |

> **Judge fields are top-level — not inside `changeTracking`.**
> To use the meaningful-change judge from `/v1/scrape`, set `goal` and `judgeEnabled` as top-level fields of the scrape request body — siblings of `url`, `formats`, and `changeTracking` — not inside the `changeTracking` object:
>
> ```json
> {
>   "url": "https://example.com",
>   "formats": ["changeTracking"],
>   "changeTracking": { "modes": ["gitDiff"], "previous": { "..." : "..." } },
>   "goal": "Alert when pricing changes",
>   "judgeEnabled": true
> }
> ```

### `POST /v1/change-tracking/diff`

A dedicated stateless endpoint served at the **engine** base URL: `https://api.fastcrw.com` (hosted) or your own origin when self-hosting.

```
POST https://api.fastcrw.com/v1/change-tracking/diff
Content-Type: application/json
Authorization: Bearer $CRW_API_KEY
```

The endpoint accepts two wire shapes on one route, discriminated by the presence of the `batch` key.

#### Single item

```json
{
  "modes": ["gitDiff"],
  "previous": { "markdown": "Starter $19", "contentHash": "a3f8..." },
  "current":  { "markdown": "Starter $24" }
}
```

Response: `{ "success": true, "data": <ChangeTrackingResult> }`

#### Batch

Supply a `batch` array. Top-level `modes`, `schema`, `prompt`, and `contentType` act as shared defaults; individual items may override them.

```json
{
  "modes": ["gitDiff"],
  "batch": [
    {
      "url": "https://example.com/pricing",
      "previous": { "markdown": "Starter $19", "contentHash": "a3f8..." },
      "current":  { "markdown": "Starter $24" }
    },
    {
      "url": "https://example.com/docs",
      "previous": { "markdown": "# Intro", "contentHash": "b9c1..." },
      "current":  { "markdown": "# Intro" }
    }
  ]
}
```

Response: `{ "success": true, "data": [<ChangeTrackingResult>, ...] }` — an array in the same order as `batch`.

The `batch` array must contain at least one item; an empty array returns `400 Bad Request`.

#### Request fields

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `current` | object | required | Current scrape content: `{ markdown?, json? }`. At least one sub-field is expected. |
| `current.markdown` | string | — | Current markdown text (used by `gitDiff` and `mixed` modes). |
| `current.json` | object | — | Current extracted JSON (used by `json` and `mixed` modes; caller pre-extracts it). |
| `previous` | object | — | Prior snapshot (see `ChangeTrackingSnapshot` shape below). Omit for first observation. |
| `modes` | string[] | `["gitDiff"]` | Same as the `changeTracking.modes` option above. |
| `schema` | object | — | JSON Schema for `json` / `mixed` mode field tracking. |
| `prompt` | string | — | Natural-language extraction prompt for `json` / `mixed` mode. |
| `contentType` | string | — | MIME type; binary content is hashed only, no diff emitted. |
| `tag` | string | — | Opaque label echoed on the result. |
| `url` | string | — | Informational URL label (batch items only; not used in diff computation). |
| `batch` | object[] | — | Array of items; presence selects batch mode. |
| `goal` | string | — | Plain-language goal for the meaningful-change judge. Accepted on both `DiffRequest` (shared default) and individual `DiffItem` entries for forward-compatibility. Not yet applied at the engine layer — judging is wired in M2. |
| `judgeEnabled` | boolean | — | Force judging on or off. Accepted for forward-compatibility alongside `goal`; not yet applied at the engine layer — judging is wired in M2. |

### `ChangeTrackingSnapshot`

The snapshot is the baseline the engine diffs the current scrape against, and also the value you must persist between checks as the next baseline. The engine returns the current snapshot on every result.

```json
{
  "markdown": "Starter $19\n\nPro $49",
  "json": { "price": "$19" },
  "contentHash": "a3f8c2d...",
  "capturedAt": "2026-06-15T12:00:00Z"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `markdown` | string | Normalized markdown stored for `gitDiff` / `mixed` mode. Omitted in `json`-only snapshots. |
| `json` | object | Extracted JSON stored for `json` / `mixed` mode. Omitted in `gitDiff`-only snapshots. |
| `contentHash` | string | Hex SHA-256 of the content — normalized markdown in `gitDiff`/`mixed` mode; canonicalized JSON in `json`-only mode. The SaaS store-skip short-circuit keys off this hash. Always supply the hash returned from the previous result; the engine does not re-derive it from a string you provide. |
| `capturedAt` | string | Optional ISO 8601 timestamp. Caller-stamped; echoed back untouched. |

### `ChangeTrackingResult`

Returned in `ScrapeData.changeTracking` (when `changeTracking` is in `formats`) and in `data` of `POST /v1/change-tracking/diff`.

```json
{
  "status": "changed",
  "firstObservation": false,
  "contentHash": "b7e2a1f...",
  "snapshot": { "markdown": "Starter $24", "contentHash": "b7e2a1f..." },
  "diff": {
    "text": "--- previous\n+++ current\n@@ -1 +1 @@\n-Starter $19\n+Starter $24\n",
    "json": {
      "files": [
        {
          "from": "previous",
          "to": "current",
          "additions": 1,
          "deletions": 1,
          "chunks": [
            {
              "content": "@@ -1,1 +1,1 @@",
              "oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1,
              "changes": [
                { "type": "del", "content": "Starter $19", "ln": 1 },
                { "type": "add", "content": "Starter $24", "ln": 1 }
              ]
            }
          ]
        }
      ],
      "additions": 1,
      "deletions": 1
    }
  }
  // tag and truncated are omitted when absent/false
}
```

| Field | Type | Description |
| --- | --- | --- |
| `status` | string | `"same"` or `"changed"`. First observations always return `"changed"`. |
| `firstObservation` | boolean | `true` when no `previous` was supplied. The caller maps this to `new` at the set level. |
| `contentHash` | string | Mode-aware SHA-256 of the current content (see `ChangeTrackingSnapshot.contentHash`). |
| `snapshot` | object | The current snapshot to persist as the next baseline. Always present. |
| `diff` | object | The diff envelope; `null` when `status == "same"` or for binary content. |
| `diff.text` | string | Unified text diff (present in `gitDiff` and `mixed` modes). |
| `diff.json` | object | Mode-polymorphic: the parse-diff AST (`gitDiff`-only) or the per-field path map (`json` / `mixed`). |
| `judgment` | object | LLM meaningful-change judgment; populated by the orchestration layer, not the engine directly. `null` when judging is not enabled or has not run. |
| `tag` | string | Echoed caller label from the request. |
| `truncated` | boolean | `true` when the diff AST was capped at the `max_diff_changes` limit (5 000 change-lines). The full snapshot is retained; only the AST is trimmed. |

#### `diff.json` — `gitDiff`-only mode (parse-diff AST)

When `modes` is `["gitDiff"]` (or omitted), `diff.json` is a parse-diff-style AST:

```json
{
  "files": [ { "from": "previous", "to": "current", "additions": 1, "deletions": 1, "chunks": [...] } ],
  "additions": 1,
  "deletions": 1
  // truncated only appears when true (AST was capped at max_diff_changes)
}
```

Each chunk entry in `changes[]` carries `type` (`"add"` / `"del"` / `"normal"`), `content` (the line without newline), and line numbers (`ln` for `add` (new-file line) and `del` (old-file line); `ln1` (old-file) + `ln2` (new-file) for `normal` context lines).

#### `diff.json` — `json` and `mixed` modes (per-field map)

When `modes` includes `"json"`, `diff.json` is a flat object keyed by dot-notation / bracket-notation JSON path, with each entry carrying `{ "previous": <value>, "current": <value> }`. Added fields have `"previous": null`; removed fields have `"current": null`.

```json
{
  "plans[0].price": { "previous": "$19", "current": "$24" },
  "plans[1].name":  { "previous": "Pro",  "current": null }
}
```

In `mixed` mode, both `diff.text` and `diff.json` (per-field map) are present.

### Normalization and hashing

The engine normalizes markdown before hashing and diffing so cosmetic churn never flips a page from `same` to `changed`:

- CRLF and bare CR normalized to LF
- Trailing whitespace stripped from every line
- Runs of two or more blank lines collapsed to one
- Leading and trailing blank lines trimmed

The `contentHash` is the hex SHA-256 of the normalized markdown (`gitDiff` / `mixed`) or the canonicalized JSON string with object keys sorted recursively (`json`-only). Whitespace-only edits and JSON key-order changes do not produce a `"changed"` result.

### Binary and non-text content

When `contentType` indicates a binary resource (e.g. `application/pdf`, `image/png`, `application/octet-stream`), the engine hashes the extracted text and returns `"same"` or `"changed"` with no `diff` object. Text types (anything matching `text/*`, `*json*`, `*xml*`, `*html*`, `*markdown*`, `*javascript*`, `*csv*`, `*yaml*`) are always diffed. Omitting `contentType` defaults to text behavior.

## Common mistakes

- **Passing `changeTracking` as an object in `formats`** — on the engine it is the plain string `"changeTracking"`; the options ride on the sibling `changeTracking` field.
- **Expecting `removed` from a scrape target** — `new` / `removed` are set-level states for **crawl** targets; a fixed `urls[]` entry that fails is `error`, never `removed`.
- **Intervals under 15 minutes** — rejected. Use `every 15 minutes` or longer.
- **Forgetting the webhook secret** — it is shown once on create; store it to verify the `X-CRW-Signature` header.

## What to read next

- [Scrape](#scraping) — the single-page primitive monitors run under the hood.
- [Crawl](#crawling) — multi-page discovery for crawl targets.
- [Credit Costs](#credit-costs) — how checks are metered.
- [Self-Hosting](#self-hosting) — run the engine + optional `monitor` mode yourself.
