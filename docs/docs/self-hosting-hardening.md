# Self-Hosting Hardening

## Minimum hardening baseline

- Terminate TLS in front of the API.
- Run the service as a non-root user.
- Restrict inbound access to required ports only.
- Isolate renderer sidecars from unnecessary network paths.

That baseline is the starting point, not the finish line. A self-hosted scraper talks to untrusted public pages and can sit close to valuable internal systems, so it deserves the same discipline as any other internet-facing API.

## Network and Access Control

- Put a reverse proxy or gateway in front of the service.
- Restrict who can reach the API by network, identity, or both.
- Avoid exposing internal health or admin surfaces to the public internet.
- If browser rendering is enabled, isolate the renderer from internal systems it does not need to reach.

## Runtime Isolation

Treat page fetching and browser rendering as higher-risk components than your application logic.

- run them with the least privilege possible,
- keep filesystem access narrow,
- and isolate sidecars so a renderer problem does not automatically become a broader platform problem.

## Secrets and Keys

- Keep API keys, proxy credentials, and LLM keys out of image builds.
- Inject secrets at runtime through your platform's secret store.
- Rotate keys during environment changes or incident response, not only on a fixed calendar.

## Operational guidance

- Rotate API keys during deployment cutovers.
- Keep browser-rendering dependencies on the smallest possible surface area.
- Expose `/health` only where your load balancer or monitoring needs it.
- Review warning-heavy targets separately; they often indicate anti-bot defenses rather than renderer bugs.

## Monitoring and Auditability

At minimum, watch:

- API error rate,
- warning frequency,
- crawl job duration,
- renderer availability,
- and resource spikes on the browser sidecar.

Keep enough logs to answer three questions after an incident:

1. what URL or workload triggered the issue,
2. whether it was an engine problem or a target-site problem,
3. and what data, if any, was still returned.

## Example Hardening Sequence

If you are moving from a dev VM to a real environment, the order should usually be:

1. put a reverse proxy and TLS in front,
2. add auth and external rate limiting,
3. move secrets into runtime injection,
4. restrict network access around the API and any renderer sidecar,
5. then enable monitoring and alerting on warnings, failures, and resource spikes.

That order keeps the riskiest exposure points under control early instead of treating hardening as a final cleanup step.

## When To Isolate the Renderer More Aggressively

Stronger isolation is worth it when:

- your targets are highly dynamic and require frequent JS rendering,
- the service runs close to internal systems with sensitive access,
- or many tenants or workloads share the same cluster.

In those cases, a renderer problem should not become an easy pivot into the rest of your infrastructure.

## Common Mistakes

- Leaving `/health` broadly exposed when only an internal load balancer needs it.
- Running the service with broader filesystem or network access than the scraping workload requires.
- Keeping incident logs too thin to separate target-site anti-bot issues from engine regressions.

Pair this page with [rate limits](/docs/rate-limits) and [error codes](/docs/error-codes) so operational hardening and runtime diagnostics are documented together.
