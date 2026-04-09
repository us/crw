# CRW Docs Style Guide

Use this file as the required template for future docs work.

## Core rules

- Every endpoint page starts with a 1-2 sentence purpose statement.
- Every endpoint page uses this order:
  1. What this endpoint is for
  2. Endpoint path and auth note
  3. Good default request
  4. Good default response
  5. Parameters
  6. Response shape
  7. Minimal cURL example
  8. SDK examples
  9. Common production patterns
  10. Common mistakes
  11. What to read next
- Every example must be copy-pasteable.
- Use `https://fastcrw.com/api` as the canonical hosted base URL.
- Use `http://localhost:3000` as the canonical self-hosted default.
- Shared concepts live on one page and are linked, not re-explained differently across multiple pages.
- Use `tabs` only for language-switched code examples.
- Use internal hash links such as `#scraping`, never raw `.md` links.

## Copy guidance

- Lead with the first successful request, not with platform theory.
- Prefer “Start here”, “Use this when”, and “Common mistakes”.
- Keep marketing claims short and concrete.
- Explain tradeoffs directly: when to use `scrape` instead of `crawl`, when to avoid `search`, when to delay JS rendering.
