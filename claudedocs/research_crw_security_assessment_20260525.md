# CRW / fastCRW Security Assessment

**Date:** 2026-05-25
**Scope:** Full codebase at `/home/pantinor/data/repo/apps/crw` (v0.10.0)
**Perspective:** Self-hosted operator concerned about local machine exposure
**Deployment context:** Backends in rootless Podman; interest in client-side security

---

## Executive Summary

**Overall verdict: Good security posture with a few notable gaps.**

CRW demonstrates deliberate, above-average security engineering for an open-source web scraping tool. The Rust language choice eliminates entire vulnerability classes (buffer overflows, use-after-free). SSRF protections are comprehensive at the API level. There is zero telemetry or phone-home behavior. The dependency surface is clean (rustls-only TLS, no openssl, all crates.io sources). Docker container hardening for SearXNG is exemplary.

The gaps that matter most for your deployment are:

1. **The browse/MCP mode has weaker SSRF protection** than the server API -- it can navigate to internal IPs
2. **Default configuration is too open** for local-only use (binds 0.0.0.0, no auth, permissive CORS)
3. **Two Docker images use `:latest` tags** creating supply chain drift risk
4. **No binary checksum verification** in install.sh or Python SDK

None of these are showstoppers. All are configurable or patchable. Your choice to run backends in rootless Podman significantly reduces container-escape risk.

---

## Findings by Severity

### CRITICAL: None

No critical vulnerabilities were found. The SSRF fundamentals are covered, there's no RCE path from the API, no command injection, and no data exfiltration from the host.

---

### HIGH (3 findings)

#### H1. Browse mode missing SSRF protection on `goto`

**Files:** `crates/crw-browse/src/tools/goto.rs`
**What:** The browse MCP server (used when AI agents need interactive browsing) validates URL schemes (blocks `file://`, `javascript:`, etc.) but does NOT call `validate_safe_url()` to block private/loopback IPs. An AI agent could navigate the browser to `http://169.254.169.254/latest/meta-data/` (cloud metadata), `http://localhost:9222` (other services), or any internal network address.
**Combined with:** The `evaluate` tool allows arbitrary JS execution on the navigated page, and the `storage` tool can read cookies/localStorage including HttpOnly cookies via CDP.
**Your risk:** If you run browse mode, a prompt-injected AI agent could scan your local network through the browser. If you don't use browse mode (`crw browse`), this doesn't apply.
**Mitigation:** You can avoid using the `crw browse` command. The core scraping API (`/v1/scrape`, `/v1/crawl`, `/v1/map`) has proper SSRF protection.

#### H2. Browser-mediated SSRF via JS redirects

**Files:** `crates/crw-renderer/src/cdp.rs`
**What:** The server-side API validates URLs before fetching, but pages rendered in Chrome/LightPanda can execute JavaScript that redirects to internal addresses (`window.location = 'http://169.254.169.254/...'`). The resulting HTML is returned in the API response. Network capture can also leak responses from XHR requests the page makes to internal endpoints.
**Your risk:** Moderate for self-hosted. An attacker would need to control a page you scrape and have that page contain a JS redirect to an internal service. Cloud metadata (AWS/GCP) is the main concern; in a Podman deployment without cloud metadata, the risk is lower.
**Mitigation:** Your rootless Podman containers limit what internal endpoints are reachable from the browser container. Network namespace isolation helps here.

#### H3. GitHub Actions pinned by version tag, not SHA

**Files:** `.github/workflows/release.yml` (14 distinct actions)
**What:** All CI/CD actions use mutable version tags (`@v4`, `@v6`). The release workflow has access to Cargo, NPM, and PyPI registry tokens.
**Your risk:** This is a supply chain concern for the project maintainers, not directly for you as a user. However, a compromised release action could publish malicious binaries that you would download.

---

### MEDIUM (11 findings)

| # | Finding | Your Risk | Mitigation |
|---|---------|-----------|------------|
| M1 | **Default bind 0.0.0.0** -- server listens on all interfaces | Any device on your LAN can use the scraper | Set `host = "127.0.0.1"` in your config |
| M2 | **No auth by default** -- API keys commented out in default config | Combined with M1, anyone on your LAN has full access | Set `api_keys = ["your-key"]` in `[auth]` |
| M3 | **Permissive CORS** -- `Access-Control-Allow-Origin: *` | Any website you visit could make cross-origin requests to your CRW instance | Only matters if M1+M2 are not addressed |
| M4 | **Unauthenticated admin endpoints** -- `/admin/breakers/reset`, `/metrics` bypass auth | Operational state can be manipulated without auth | Behind localhost bind + auth, this is low risk |
| M5 | **DNS rebinding gap** in SSRF protection | Theoretical bypass of URL validation via DNS TOCTOU | Very hard to exploit; rootless Podman helps |
| M6 | **`lightpanda/browser:latest`** unpinned Docker image | Silent supply chain drift | Pin to a specific version/digest |
| M7 | **`chromedp/headless-shell:latest`** unpinned Docker image | Same as M6 | Pin to a specific Chromium version tag |
| M8 | **Dockerfile runs as root** -- no USER directive | Container processes run as root inside the container | Rootless Podman maps this to your UID -- **already mitigated** |
| M9 | **Browser containers lack hardening** -- no `cap_drop`, `security_opt` | Browser processes have unnecessary capabilities | Add `cap_drop: [ALL]`, `security_opt: [no-new-privileges:true]` |
| M10 | **Chrome `--ignore-certificate-errors`** | Chrome accepts MITM'd certificates for scraped sites | Only affects scraping targets, not your machine |
| M11 | **Prompt injection via scraped content** to LLM | Attacker-controlled pages could influence extraction/summary output | Only if you use LLM features; validate LLM outputs |

---

### LOW (10 findings)

| # | Finding | Notes |
|---|---------|-------|
| L1 | Test-only SSRF bypass env var (`CRW_ALLOW_LOOPBACK_FOR_TESTS`) is runtime, not compile-time | Don't set this in production |
| L2 | `--llm-key` CLI flag exposes key in process list and shell history | Use env var `CRW_EXTRACTION__LLM__API_KEY` instead |
| L3 | Proxy credentials logged on parse failure | Only triggers on malformed proxy URLs |
| L4 | Default secrets in docker-compose (`SEARXNG_SECRET_KEY`, `BROWSERLESS_TOKEN`) | Set proper values in `.env` for non-local deployments |
| L5 | Regex DoS in chunking -- user-supplied regex not size-limited | Rust's `regex` crate guarantees linear time, so CPU DoS only |
| L6 | `Debug` derive on structs containing API keys | Only a risk if debug logging is added in the future |
| L7 | WebSocket connections to renderers are plaintext (`ws://`) | Contained within Docker bridge network |
| L8 | No `cargo audit` in CI | 496 transitive deps not scanned for advisories |
| L9 | `cross` installed from git HEAD in release CI | Unpinned build tool |
| L10 | Screenshot tool in browse mode writes to arbitrary file paths | Only relevant if using browse mode |

---

### Positive Findings (INFO)

These are things the project does **well**:

| Area | Detail |
|------|--------|
| **Memory safety** | Rust; near-zero `unsafe` (only 2 justified `killpg` calls and edition-2024 `set_var`) |
| **TLS** | Exclusive `rustls` -- no openssl/native-tls in entire dependency tree |
| **No telemetry** | Zero phone-home behavior. Comprehensive grep confirmed no analytics, tracking, or update-check calls |
| **SSRF at API level** | `validate_safe_url()` applied on all 5 route handlers + every redirect hop |
| **Constant-time auth** | API key comparison resists timing attacks; all configured keys checked without short-circuit |
| **Body size limits** | 1 MB max request body across all endpoints |
| **Input clamping** | URL length (2048), search query (2000 chars), map params (64 max), network capture (30 bodies, 2MB) |
| **Stateless** | No database, no persistent storage, no disk-based caches |
| **Config file security** | Written with `0o600` permissions, symlink-safe, atomic writes |
| **Container hardening (SearXNG)** | `read_only`, `cap_drop: ALL`, `no-new-privileges`, `tmpfs`, resource limits, healthcheck |
| **Security headers** | `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` |
| **Security test suite** | Dedicated `tests/security.rs` covering SSRF, auth, information disclosure |
| **Dependency hygiene** | All 496 deps from crates.io (zero git deps), minimal Python/NPM surfaces |
| **No build scripts** | No `build.rs` or proc-macro crates in the workspace |

---

## Your Specific Deployment: Rootless Podman

Your choice of rootless Podman is a strong mitigation for several findings:

| Finding | Podman Mitigation |
|---------|------------------|
| M8 (runs as root) | **Fully mitigated.** Rootless Podman maps container root to your UID via user namespaces. Even PID 1 inside the container runs as your unprivileged user on the host. |
| H2 (browser SSRF) | **Partially mitigated.** Network namespace isolation limits what internal endpoints the browser can reach. Podman's `slirp4netns` or `pasta` networking adds a layer of separation. |
| M9 (browser capabilities) | **Partially mitigated.** Rootless Podman drops many capabilities by default. `CAP_SYS_ADMIN` (needed for Chrome sandbox) is typically not available, which is why `--no-sandbox` is used. |
| M10 (cert errors) | **Not mitigated.** This is a Chrome flag that affects TLS validation for scraped targets regardless of container runtime. |

---

## Recommended Hardening for Your Setup

### Immediate (config changes only, no code changes):

1. **Bind to localhost**: Set `host = "127.0.0.1"` in your config TOML
2. **Set API keys**: Uncomment and set `api_keys = ["your-key"]` in `[auth]`
3. **Pin Docker images**: Change `:latest` to specific versions for `lightpanda/browser` and `chromedp/headless-shell`
4. **Harden browser containers**: Add to your docker-compose override:
   ```yaml
   lightpanda:
     cap_drop: [ALL]
     security_opt: [no-new-privileges:true]
     pids_limit: 128
   chrome:
     cap_drop: [ALL]
     security_opt: [no-new-privileges:true]
     pids_limit: 256
   ```

### If using browse mode:

5. **Be aware** that browse mode has weaker SSRF protection than the server API. AI agents connected via MCP can navigate the browser to internal addresses. If this concerns you, avoid using `crw browse` or restrict which AI agents can access it.

### If using LLM features:

6. **Use env vars for API keys**: `CRW_EXTRACTION__LLM__API_KEY=sk-...` instead of config file
7. **Validate LLM outputs**: Scraped content may contain prompt injection payloads

---

## Fair Overall Assessment

**For a self-hosted web scraper, CRW is significantly more secure than most alternatives in its class.** The Rust foundation, memory-safe TLS, comprehensive SSRF protections, zero telemetry, and stateless architecture are all strong positives. The security test suite shows intentional security thinking, not afterthought.

**The main concerns for a local deployment are configuration defaults** (0.0.0.0 bind, no auth, permissive CORS), which are easily fixed. The browse mode SSRF gap is the most substantive code-level issue, but it only applies if you use that specific feature.

**Your rootless Podman deployment is a good defense-in-depth choice** that mitigates several container-level concerns. Combined with the localhost bind and API key config changes, your exposure surface becomes quite small: you'd be running a localhost-only, authenticated, stateless Rust binary that fetches web pages with SSRF protection and returns markdown.

**Confidence level for daily use:** HIGH, provided you apply the config hardening above.

---

## Methodology

This assessment was conducted through static analysis of the full codebase (10 crates, ~15,000 lines of Rust), configuration files, Docker infrastructure, CI/CD pipelines, install scripts, and NPM/Python distribution packages. Four parallel analysis streams covered: server-side security, client/MCP security, supply chain/dependencies, and network exposure. No dynamic testing or fuzzing was performed.

---

*Report generated by security analysis of CRW v0.10.0 codebase.*
