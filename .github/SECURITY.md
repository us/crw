# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| Latest release (`main`) | Yes |
| Older releases | No — please upgrade |

We release frequently. The safest course is always to run the latest published
version from [crates.io](https://crates.io/crates/crw-server) or the latest
GitHub release.

---

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report privately through either channel:

- **GitHub Private Vulnerability Reporting** (preferred): open a report from the
  [Security tab](https://github.com/us/crw/security/advisories/new) of this
  repository. This routes straight to the maintainers and keeps the details private.
- **Email**: **security@fastcrw.com**

Include as much detail as you can:

- A description of the vulnerability and its potential impact.
- Steps to reproduce or a minimal proof-of-concept.
- The version(s) of fastCRW affected.
- Any suggested mitigations, if you have them.

We will acknowledge your report within **48 hours** and aim to ship a patch within
**14 days** for confirmed high-severity issues. We will keep you updated throughout
the process and credit you in the release notes (unless you prefer to stay
anonymous).

---

## Disclosure process

1. You email `security@fastcrw.com` with the details.
2. We confirm receipt and open a private tracking issue.
3. We reproduce the issue, assess severity, and develop a fix.
4. We coordinate a disclosure date with you.
5. We publish a patched release and a public advisory on the same day.

We follow a **90-day coordinated disclosure** window by default. If a vulnerability
is already being actively exploited, we may move faster.

---

## Scope

In scope for this policy:

- The `crw-server` binary and all workspace crates in this repository.
- The MCP server (`crw-mcp`).
- The TypeScript SDK (`sdks/typescript`).

Out of scope:

- The managed cloud platform at `fastcrw.com` and `api.fastcrw.com` — report
  cloud/SaaS issues to the same address; they are triaged separately.
- Third-party headless browsers (Chromium, Lightpanda) invoked by the renderer.
  Please report those upstream.

---

## License note

fastCRW is licensed under AGPL-3.0. The license does not limit your right to
report security vulnerabilities or receive fixes. If you are running fastCRW in a
context where the AGPL obligations are relevant to your deployment, please read
`LICENSE` and feel free to contact us at `security@fastcrw.com` with any questions.
