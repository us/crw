# TLS-fingerprint walls

Some sites block non-browser HTTP clients by fingerprinting the connection itself: the TLS ClientHello (JA3/JA4), the HTTP/2 SETTINGS frame, and the header order and case are evaluated before the body is read. A wrong-looking fingerprint gets a wall: an HTTP 200 carrying a useless interstitial page, a short obfuscated JavaScript stub, or a bare 400/403. Changing the User-Agent does nothing, because the verdict was made at the handshake.

The `impersonated-http` renderer tier exists for these sites: a plain HTTP fetch presenting a real Chrome TLS/JA3/HTTP2 fingerprint, no browser and no JavaScript. In the auto chain it runs between the plain HTTP tier and the browser ladder, and only when the plain tier hit a wall-shaped response. See [JS rendering](/docs/js-rendering) for the tier's mechanics and pin semantics.

This page is about *which sites* are in scope. The pattern matters more than any specific domain, because walls rotate.

## Three gate shapes

**TLS-only gate.** The fingerprint is the whole check. Impersonation clears it and no browser is needed. Verified examples (September 2026, residential IP, plain client vs `impersonated-http` pin):

- `amazon.it` product pages: the plain tier receives a 200 interstitial ("Fai clic sul pulsante qui sotto per continuare a fare acquisti"), the impersonated tier receives the real product page. The wall is per-request intermittent: the same plain client sometimes gets the real page, which is why the auto chain re-decides on every request.
- Temu catalog pages: the plain tier receives a ~3KB obfuscated anti-bot stub, the impersonated tier the real page.
- Public Facebook pages: the plain tier a 400 error page, the impersonated tier the real page.

**JavaScript gate.** The site serves the same shell or challenge to every client that does not execute JavaScript. Impersonation cannot help; the browser ladder is the right tier. Observed with identical responses on both clients: X, Booking.com (a script-based AWS WAF challenge), Zara, LinkedIn, Glassdoor, `www.reddit.com` (the old-reddit host serves plain HTML and needs neither).

**Combined gate.** A fingerprint vendor (DataDome, Akamai, PerimeterX) layers a JavaScript or cookie challenge on top of the TLS check. From a residential IP these often let both clients through; from datacenter IPs the wall engages and clearing the TLS half is necessary but not sufficient. Observed in this state: Vinted, Subito.it, AliExpress, Airbnb. Expect to pair the impersonated tier with a browser or a residential egress here.

**Not a fingerprint problem.** A block that returns the identical error to two different TLS stacks is an IP-rate or reputation block, not a fingerprint wall. eBay demonstrated this: fine in the morning, 403 for every client after heavy probing from the same address, self-healing with time. Impersonation is wasted budget on these; the fix is pacing, or a different egress.

## Diagnosing a blocked site on your network

The wall landscape depends on your egress IP, so test from wherever crw runs:

1. Scrape the URL with `renderJs:false` and read `metadata.renderedWith` and the body. A 200 with a tiny body whose visible text is one "verify / continue shopping" sentence is a wall.
2. Scrape again with `renderer:"impersonated-http"`.
3. Compare. Real content from the pin means a TLS-only gate and the auto chain will handle it unattended. The same wall from the pin means the site wants JavaScript or cookies; use a browser tier. The same error from both means rate limiting; slow down.

`GET /v1/capabilities` lists `impersonated-http` when the build and config enable it.

## Detector scope and how to extend it

Escalation into the tier is driven by the wall detectors, and the phrase list is deliberately narrow: only interstitial sentences verified against a live wall are listed, because a false wall escalates to browsers and can latch the host to proxy egress. As of v0.35.1 the list carries the verified it-IT Amazon sentence; the English "click the button below to continue shopping" is deliberately absent, since it is ordinary retail copy (empty-cart and order-confirmation pages carry it verbatim) and was never verified against a live wall.

To cover another marketplace: capture the interstitial its wall actually serves in your locale (the plain-tier body), confirm the sentence is unique to the wall and not normal copy on real pages, then open a PR adding that exact sentence plus a positive test with the captured fixture and a negative test with a real page that quotes it.

## Preset maintenance

The tier impersonates a pinned Chrome version. When sites start rejecting that fingerprint, the failure mode is a graceful fall-through to the browser ladder, not an error. The `#[ignore]` live tests in `crates/crw-renderer/src/impersonated.rs` are the sentinel: they fail when the pinned preset stops clearing a known wall, which is the signal to bump the preset.
