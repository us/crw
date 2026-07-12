# Recipe: Monitor a Competitor Pricing Page → Slack Alert

**What you build:** A monitor that checks a competitor pricing page every 30 minutes, uses an LLM judge to filter noise, and posts a diff to Slack the moment a meaningful price change lands. A Python webhook handler verifies the signed payload and formats the Slack message.

**Time to wire up:** ~15 minutes.

**Credits used:** 1 per check (scrape) + 1 per changed page the judge validates.

---

## How it works

```
fastcrw.com/api (SaaS scheduler)
    ↓  every 30 min
api.fastcrw.com  ← scrapes competitor.com/pricing
    ↓  diff against last snapshot
LLM judge        ← is this a meaningful price change?
    ↓  yes → signed webhook
Your server      ← verifies X-CRW-Signature, posts to Slack
```

The monitor control plane (`/v1/monitor`) lives at `https://fastcrw.com/api` — separate from the scrape engine at `https://api.fastcrw.com`. You only call the control plane to create/update the monitor; the scrapes and webhooks happen automatically on schedule.

> **SaaS-only field.** `targets[].changeMode` (`"markdown"` | `"json"` | `"mixed"`) belongs to the managed monitor control plane. The open-core engine has no monitor resource at all: it exposes the stateless `changeTracking` scrape format and `/v1/change-tracking/diff`, which take a top-level `modes[]` array instead. Use `changeMode` only when calling `https://fastcrw.com/api`.

---

## Step 1 — Create the monitor

:::tabs
::tab{title="cURL"}
```bash
curl -s -X POST "https://fastcrw.com/api/v1/monitor" \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Competitor pricing watcher",
    "schedule": { "text": "every 30 minutes", "timezone": "UTC" },
    "goal": "Alert only when a pricing tier name, price, or headline feature changes. Ignore navigation, footer, or cookie banner changes.",
    "targets": [
      {
        "type": "scrape",
        "urls": ["https://competitor.com/pricing"],
        "changeMode": "markdown"
      }
    ],
    "webhook": {
      "url": "https://yourserver.com/webhooks/crw",
      "events": ["monitor.page", "monitor.check.completed"],
      "metadata": { "slackChannel": "#pricing-alerts" }
    }
  }'
```
::
::tab{title="Python"}
```python
import os
import requests

res = requests.post(
    "https://fastcrw.com/api/v1/monitor",
    headers={"Authorization": f"Bearer {os.environ['CRW_API_KEY']}"},
    json={
        "name": "Competitor pricing watcher",
        "schedule": {"text": "every 30 minutes", "timezone": "UTC"},
        "goal": (
            "Alert only when a pricing tier name, price, or headline feature "
            "changes. Ignore navigation, footer, or cookie banner changes."
        ),
        "targets": [
            {
                "type": "scrape",
                "urls": ["https://competitor.com/pricing"],
                "changeMode": "markdown",
            }
        ],
        "webhook": {
            "url": "https://yourserver.com/webhooks/crw",
            "events": ["monitor.page", "monitor.check.completed"],
            "metadata": {"slackChannel": "#pricing-alerts"},
        },
    },
)
data = res.json()["data"]
print("monitor id :", data["id"])
print("next run   :", data["nextRunAt"])
print("webhook secret (save this — shown once):", data.get("webhookSecret"))
```
::
:::

**Store the `webhookSecret` now.** It is returned once in the create response and never again. You need it to verify signatures.

### Expected response

```json
{
  "success": true,
  "data": {
    "id": "019df960-06e7-7383-9d89-82c0113dc31a",
    "name": "Competitor pricing watcher",
    "status": "active",
    "schedule": { "cron": "*/30 * * * *", "timezone": "UTC", "text": "every 30 minutes" },
    "nextRunAt": "2026-06-15T12:30:00.000Z",
    "lastRunAt": null,
    "goal": "Alert only when a pricing tier name, price, or headline feature changes...",
    "judgeEnabled": true,
    "targets": [
      {
        "type": "scrape",
        "urls": ["https://competitor.com/pricing"],
        "changeMode": "markdown"
      }
    ],
    "webhook": {
      "url": "https://yourserver.com/webhooks/crw",
      "events": ["monitor.page", "monitor.check.completed"]
    },
    "webhookSecret": "a3f8b2c1d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1",
    "estimatedCreditsPerMonth": 2880,
    "createdAt": "2026-06-15T12:00:00.000Z"
  }
}
```

---

## Step 2 — Webhook handler (Python)

Save your `webhookSecret` as an environment variable (`CRW_WEBHOOK_SECRET`) and run this Flask app. It verifies the HMAC-SHA256 signature, then posts to Slack when the LLM judge finds a meaningful change.

**Signature scheme:** the `X-CRW-Signature` header has the form `t=<unix>,v1=<hex>`. The MAC is `HMAC-SHA256(secret, "<t>.<raw_body>")` over the raw request bytes. Verify before parsing.

```python
# webhook_handler.py
import hashlib
import hmac
import json
import os
import time

import requests
from flask import Flask, abort, jsonify, request

app = Flask(__name__)

WEBHOOK_SECRET = os.environ["CRW_WEBHOOK_SECRET"]   # from monitor create response
SLACK_WEBHOOK_URL = os.environ["SLACK_WEBHOOK_URL"]  # https://hooks.slack.com/...
TOLERANCE_SECONDS = 300  # reject replays older than 5 minutes


# ---------------------------------------------------------------------------
# Signature verification
# ---------------------------------------------------------------------------

def verify_signature(raw_body: bytes, sig_header: str) -> None:
    """Raise ValueError if the X-CRW-Signature header is invalid or stale."""
    parts = dict(p.split("=", 1) for p in sig_header.split(",") if "=" in p)
    t = parts.get("t")
    v1 = parts.get("v1")
    if not t or not v1:
        raise ValueError("malformed signature header")

    age = abs(time.time() - int(t))
    if age > TOLERANCE_SECONDS:
        raise ValueError(f"signature too old: {age:.0f}s")

    expected = hmac.new(
        WEBHOOK_SECRET.encode(),
        f"{t}.".encode() + raw_body,
        hashlib.sha256,
    ).hexdigest()

    if not hmac.compare_digest(expected, v1):
        raise ValueError("signature mismatch")


# ---------------------------------------------------------------------------
# Slack helper
# ---------------------------------------------------------------------------

def post_to_slack(channel: str, text: str) -> None:
    requests.post(SLACK_WEBHOOK_URL, json={"channel": channel, "text": text}, timeout=5)


# ---------------------------------------------------------------------------
# Webhook endpoint
# ---------------------------------------------------------------------------

@app.route("/webhooks/crw", methods=["POST"])
def crw_webhook():
    raw = request.get_data()
    sig = request.headers.get("X-CRW-Signature", "")

    try:
        verify_signature(raw, sig)
    except ValueError as exc:
        app.logger.warning("rejected webhook: %s", exc)
        abort(400, str(exc))

    envelope = json.loads(raw)
    event = envelope.get("type")
    payload = envelope.get("data", [{}])[0]
    metadata = envelope.get("metadata", {})
    slack_channel = metadata.get("slackChannel", "#pricing-alerts")

    if event == "monitor.page":
        handle_page_event(payload, slack_channel)
    elif event == "monitor.check.completed":
        handle_check_completed(payload, slack_channel)

    return jsonify({"ok": True})


def handle_page_event(payload: dict, channel: str) -> None:
    """
    Fired for each non-same page. Only alert when the judge says meaningful.

    payload shape:
      { monitorId, checkId, url, status, isMeaningful }
    status is lowercase: "new" | "changed" | "removed" | "error"
    """
    if payload.get("isMeaningful") is not True:
        return  # noise — judge said not meaningful, skip

    url = payload["url"]
    status = payload["status"]
    check_id = payload["checkId"]

    text = (
        f":rotating_light: *Pricing change detected*\n"
        f"URL: {url}\n"
        f"Status: `{status}`\n"
        f"Check: `{check_id}`\n"
        f"Diff: https://fastcrw.com/dashboard (monitor → latest check)"
    )
    post_to_slack(channel, text)


def handle_check_completed(payload: dict, channel: str) -> None:
    """
    Fired once per check after all pages are reconciled.

    payload shape:
      { monitorId, checkId, summary: { totalPages, same, new, changed, removed, error }, siteDown }
    """
    summary = payload.get("summary", {})
    changed = summary.get("changed", 0)
    new = summary.get("new", 0)
    removed = summary.get("removed", 0)
    site_down = payload.get("siteDown", False)

    if site_down:
        post_to_slack(channel, ":warning: Competitor pricing page appears to be down (site-down gate tripped).")
        return

    if changed + new + removed == 0:
        return  # nothing changed, no alert

    lines = [f":bar_chart: *Check complete* — {summary.get('totalPages', 0)} pages scanned"]
    if changed:
        lines.append(f"  • `{changed}` changed")
    if new:
        lines.append(f"  • `{new}` new")
    if removed:
        lines.append(f"  • `{removed}` removed")

    post_to_slack(channel, "\n".join(lines))


if __name__ == "__main__":
    app.run(port=8080)
```

Run it:

```bash
pip install flask requests
CRW_WEBHOOK_SECRET=<from create response> \
SLACK_WEBHOOK_URL=https://hooks.slack.com/services/... \
python webhook_handler.py
```

---

## Webhook envelope shape

Every delivery wraps the event payload in a common envelope:

```json
{
  "success": true,
  "type": "monitor.page",
  "id": "<checkId>",
  "data": [
    {
      "monitorId": "019df960-06e7-7383-9d89-82c0113dc31a",
      "checkId": "019e1234-abcd-7000-8000-000000000001",
      "url": "https://competitor.com/pricing",
      "status": "changed",
      "isMeaningful": true
    }
  ],
  "metadata": { "slackChannel": "#pricing-alerts" }
}
```

And for `monitor.check.completed`:

```json
{
  "success": true,
  "type": "monitor.check.completed",
  "id": "019e1234-abcd-7000-8000-000000000001",
  "data": [
    {
      "monitorId": "019df960-06e7-7383-9d89-82c0113dc31a",
      "checkId": "019e1234-abcd-7000-8000-000000000001",
      "summary": {
        "totalPages": 1,
        "same": 0,
        "new": 0,
        "changed": 1,
        "removed": 0,
        "error": 0
      },
      "siteDown": false
    }
  ]
}
```

---

## Step 3 — Inspect the diff (optional)

Fetch the check detail to read the full text diff before it appears in Slack, or to build a richer alert:

:::tabs
::tab{title="cURL"}
```bash
curl "https://fastcrw.com/api/v1/monitor/$MONITOR_ID/checks/$CHECK_ID?status=changed" \
  -H "Authorization: Bearer $CRW_API_KEY"
```
::
::tab{title="Python"}
```python
import os, requests

monitor_id = "019df960-06e7-7383-9d89-82c0113dc31a"
check_id   = "019e1234-abcd-7000-8000-000000000001"

res = requests.get(
    f"https://fastcrw.com/api/v1/monitor/{monitor_id}/checks/{check_id}",
    params={"status": "changed"},
    headers={"Authorization": f"Bearer {os.environ['CRW_API_KEY']}"},
)
check = res.json()["data"]

for page in check.get("pages", []):
    if page["status"] == "changed":
        print(page["url"])
        print(page.get("diffText", "(no text diff)"))
```
::
:::

---

## What the Slack alert looks like

When the judge marks a change meaningful your `#pricing-alerts` channel gets:

```
🚨 Pricing change detected
URL: https://competitor.com/pricing
Status: `changed`
Check: `019e1234-abcd-7000-8000-000000000001`
Diff: https://fastcrw.com/dashboard (monitor → latest check)
```

When a check completes with no meaningful changes, the `monitor.page` handler returns early (because `isMeaningful` is not `true`) and the `monitor.check.completed` handler returns early (because `changed + new + removed == 0`). Slack stays quiet.

---

## Tuning the judge

The `goal` field is plain English — be specific. Vague goals produce more false positives because the judge has less context to discard minor rephrasing.

| Too vague | Better |
|---|---|
| `"Alert on changes"` | `"Alert when a plan price, tier name, or feature list changes. Ignore wording, button text, layout, or footer changes."` |
| `"Watch pricing"` | `"Alert when any numeric price, currency symbol, or billing cycle (monthly/annual) changes on a paid plan."` |

---

## Common mistakes

| Mistake | Fix |
|---|---|
| Losing the `webhookSecret` | It is returned once on create. Store it immediately in your secrets manager. To rotate: `PATCH /v1/monitor/{id}` with a new `webhook` object to regenerate. |
| Verifying the signature after JSON parsing | Always verify against the **raw bytes** before parsing. `request.get_data()` in Flask, `req.rawBody` in Express. |
| Setting `schedule.text` to an interval under 15 minutes | Minimum is `"every 15 minutes"`. Shorter intervals are rejected. |
| Expecting `status: "removed"` from a scrape target | `removed` is a set-level state for **crawl** targets only. A fixed-URL scrape that errors returns `status: "error"`. |
| Skipping replay protection | Check `abs(time.time() - int(t)) > 300` and reject stale deliveries. |

---

## Next steps

- [Monitoring reference](/monitoring) — all parameters, schedule syntax, change modes, and check status codes.
- [Crawl monitors](/monitoring#targets) — watch entire site sections, get `new`/`removed` for discovered pages.
- [JSON change mode](/monitoring#change-tracking-modes) — track specific structured fields (e.g. `plans[0].price`) across checks.
- [Credit costs](/credit-costs) — how scrape + judge credits are metered per check.
