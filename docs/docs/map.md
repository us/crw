# Map Endpoint Guide

## Overview

`map` is the lightweight discovery tool in the stack. Use it before `crawl` when you need to answer:

- what URLs are reachable from this starting point,
- which subsection of the site is actually worth scraping,
- and whether a full crawl is justified at all.

```bash
curl -X POST http://localhost:3000/v1/map \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{"url":"https://example.com"}'
```

## Why `map` Comes First

A lot of scraping waste comes from starting too broad. `map` helps you inspect the site structure first, which is especially useful when:

- the site has multiple product areas,
- navigation is noisy,
- or an agent needs to choose its next step instead of crawling everything.

## When To Use It

Choose `map` when you want to answer:

- what URLs are reachable from this starting point,
- what section of the site should we target next,
- and whether a full crawl is worth the cost.

## A Typical Workflow

One common pattern looks like this:

1. `map` a docs home page or product section.
2. Filter the returned URLs in your application.
3. Launch `scrape` for a handful of known-important pages.
4. Launch `crawl` only for the subset that deserves broader recursion.

That works well for AI agents, indexing systems, and human operators doing a first evaluation.

## Output Expectations

`map` is for discovery, not deep extraction. Treat it as a planning primitive:

- it helps you decide what to fetch,
- it reduces unnecessary crawl scope,
- and it makes later scrape or crawl requests more intentional.

If your end goal is page content, `map` should usually be the first step, not the last one.

## Example: Narrow a Noisy Site Before Crawl

Imagine a large docs domain with product pages, marketing pages, changelogs, and a blog mixed together. Running a broad crawl from the home page creates noise quickly.

Instead:

1. start with `map` on the docs root,
2. inspect the returned URLs,
3. keep only the section that matters,
4. then launch `crawl` on that smaller scope.

That pattern reduces wasted credits and keeps downstream systems cleaner.

## Common Mistakes

- Using `map` when you already know the exact page you need. In that case use [scrape](/docs/scraping) directly.
- Treating `map` output as if it were extracted content instead of URL discovery.
- Launching a full crawl from a noisy homepage before inspecting the reachable structure.

## What To Read Next

- Use [crawl](/docs/crawling) when you are ready to recurse after discovery.
- Use [getting started](/docs/quick-start) if you need the shortest path to a working first request.
- Use [rate limits](/docs/rate-limits) when map-based discovery is feeding many follow-up requests.
