# Contributing to fastCRW

Contributions are welcome — issues and PRs both.

## Contributor License Agreement

On your first pull request a bot will ask you to sign our
[Contributor License Agreement](CLA.md). It takes one comment and covers all of
your future contributions. We can't merge PRs until it's signed.

## Development setup

1. Fork the repository
2. Install pre-commit hooks: `make hooks`
3. Create a feature branch: `git checkout -b feat/my-feature`
4. Commit your changes: `git commit -m 'feat: add my feature'`
5. Push and open a Pull Request

The pre-commit hook runs the same checks as CI. Run them manually with:

```bash
make check    # cargo fmt + cargo clippy + cargo test
```

## Architecture

`crw-server` (Axum API + auth + MCP) sits on top of:

| Crate | Responsibility |
|-------|----------------|
| [`crw-core`](crates/crw-core) | Core types, config, error handling |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP rendering (LightPanda/Chrome auto-detect) |
| [`crw-extract`](crates/crw-extract) | HTML → markdown / plaintext / JSON extraction |
| [`crw-crawl`](crates/crw-crawl) | Async BFS crawler with robots.txt & sitemap |
| [`crw-server`](crates/crw-server) | Axum API server (native `/v1` + Firecrawl `/firecrawl/v2` compat) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio server (embedded + proxy mode) |
| [`crw-cli`](crates/crw-cli) | Standalone CLI (`crw` binary, no server) |

Full architecture docs: [docs.fastcrw.com/architecture/](https://docs.fastcrw.com/architecture/)

## Contributors

<p>
  <a href="https://github.com/us"><img src="https://github.com/us.png?size=64" width="64" height="64" alt="us"/></a>
  <a href="https://github.com/adambenhassen"><img src="https://github.com/adambenhassen.png?size=64" width="64" height="64" alt="adambenhassen"/></a>
  <a href="https://github.com/mj520"><img src="https://github.com/mj520.png?size=64" width="64" height="64" alt="mj520"/></a>
</p>
