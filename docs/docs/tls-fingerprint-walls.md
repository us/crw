<div class="page-intro">
  <div class="page-kicker">Deploy</div>
  <h1>TLS-fingerprint walls</h1>
  <p class="page-subtitle">Some sites block non-browser clients by fingerprinting the connection itself. This page tells you which site shapes the <code>impersonated-http</code> tier clears, which ones need a browser, and how to tell the difference from your own egress.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Covers:</strong> scrape · renderer tiers · self-hosting</div>
    <div class="page-capability"><strong>Verified against:</strong> crates/crw-renderer/src/impersonated.rs · crates/crw-renderer/src/detector.rs · live wall probes (September 2026)</div>
  </div>
</div>

Some sites reject non-browser HTTP clients before reading a single byte of the request: the TLS ClientHello (JA3/JA4), the HTTP/2 SETTINGS frame, and the header order and case are fingerprinted at the handshake, and a wrong-looking fingerprint gets a wall. Changing the `User-Agent` does nothing, because the verdict was already made.

The [`impersonated-http`](/docs/js-rendering) renderer tier exists for these sites: a plain HTTP fetch presenting a real Chrome TLS/JA3/HTTP2 fingerprint, with no browser and no JavaScript. In the auto chain it runs between the plain HTTP tier and the browser ladder, and only when the plain tier hit a wall-shaped response. This page is about *which sites* are in scope, because the pattern matters more than any specific domain: walls rotate.

## What the wall looks like

**Symptom:** the scrape succeeds (`success: true`, HTTP 200) but the markdown is a useless stub: an interstitial page asking the reader to click a button, a few kilobytes of obfuscated JavaScript, or an error shell. The same URL renders fine in a browser, and `metadata.renderedWith` says `http`.

That last field is the tell. A wall is not an error: the origin answers `200` and returns a page, so everything downstream of the fetcher sees a normal response. The auto chain detects the shape and escalates; `metadata.renderedWith` and `renderDecision` in the response tell you which tier ended up serving the page.

## Which sites are in scope

Sites split into three shapes when they block non-browser clients. Only the first is what `impersonated-http` fixes.

**TLS-only gate.** The fingerprint is the whole check; impersonation clears it and no browser is needed. Verified against the live walls in September 2026:

- `amazon.it` product pages: the plain tier receives a 200 interstitial, the impersonated tier the real product page. The wall is per-request intermittent, so the same plain client sometimes gets the real page, which is why the auto chain re-decides on every request.
- Temu catalog pages: a ~3KB obfuscated anti-bot stub versus the real page.
- Public Facebook pages: a 400 error page versus the real page.

**JavaScript gate.** The site serves the same shell or challenge to every client that does not execute JavaScript, fingerprint notwithstanding. Impersonation cannot help; a browser tier is the right answer. Sites observed answering both clients identically: X, Booking.com (a script-based AWS WAF challenge), Zara, LinkedIn, Glassdoor, `www.reddit.com` (the old-reddit host serves plain HTML and needs neither).

**Combined gate.** A fingerprint vendor (DataDome, Akamai, PerimeterX) layers a JavaScript or cookie challenge on top of the TLS check. From a residential IP these often let both clients through; from datacenter egress the wall engages, and clearing the TLS half is necessary but not sufficient. Observed in this state: Vinted, Subito.it, AliExpress, Airbnb. Pair the impersonated tier with a browser or a residential egress here.

> **Not a fingerprint problem.** A block that returns the identical error to two
> different TLS stacks is an IP-rate or reputation block, not a fingerprint wall.
> eBay behaves this way under heavy probing from one address: the same 403 for
> every client, self-healing with time. Impersonation is wasted budget on these;
> the fix is pacing, or a different egress.

## Diagnose from your own egress

The wall landscape depends on the IP crw egresses from, so test from wherever your instance runs:

1. Scrape the URL with `renderJs:false` and read `metadata.renderedWith` and the body. A 200 with a tiny body whose visible text is one "verify" or "continue shopping" sentence is a wall.
2. Scrape again with `renderer:"impersonated-http"`:
3. Compare. Real content from the pin means a TLS-only gate, and the auto chain will handle it unattended. The same wall from the pin means the site wants JavaScript or cookies; use a browser tier. The same error from both means rate limiting; slow down.

```bash
# cURL — step 2: the impersonated pin (managed cloud)
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://www.example-walled.com/product/123",
    "renderer": "impersonated-http"
  }'
```

```bash
# cURL — self-hosted: same call against your own instance
curl -X POST http://localhost:3030/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://www.example-walled.com/product/123",
    "renderer": "impersonated-http"
  }'
```

`GET /v1/capabilities` lists `impersonated-http` when the build and config enable it, so a client can gate the pin before sending traffic.

## Extending the wall detector

Escalation into the tier is driven by the wall detectors, and the phrase list is deliberately narrow: only interstitial sentences verified against a live wall are listed, because a false wall escalates to browsers and can latch the host to proxy egress. As of v0.35.1 the list carries the verified it-IT Amazon sentence; the English "click the button below to continue shopping" is deliberately absent, since it is ordinary retail copy that empty-cart and order-confirmation pages carry verbatim.

To cover another marketplace: capture the interstitial its wall actually serves in your locale (the plain-tier body), confirm the sentence is unique to the wall and not normal copy on real pages, then open a PR adding that exact sentence plus a positive test with the captured fixture and a negative test with a real page that quotes it.

## When the fingerprint ages

The tier impersonates a pinned Chrome version, and sites eventually start rejecting old fingerprints. The failure mode is graceful: the hop's result fails the accept gate and the chain falls through to the browser ladder, so pages keep working, slower. The `#[ignore]` live tests in `crates/crw-renderer/src/impersonated.rs` are the sentinel: they fail when the pinned preset stops clearing a known wall, which is the signal to bump the preset.
