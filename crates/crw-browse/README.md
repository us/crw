# crw-browse

MCP server for interactive browser automation over the Chrome DevTools Protocol.

Part of the [CRW workspace](../..). Built on top of `crw-renderer`'s persistent
`CdpConnection` primitive, so it shares the same CDP machinery the scrape
pipeline uses.

## Status: Phase 1 — walking skeleton

Phase 1 ships two tools over stdio:

| Tool   | Purpose |
|--------|---------|
| `goto` | Navigate the default session to a URL and wait for load |
| `tree` | Fetch the page's accessibility tree as a compact listing |

Phase 2+ (session lifecycle tools, click/type/fill\_form, HTTP transport, policy
guards, structured data, screenshot, etc.) is in `ROADMAP.md`.

## Running

Build the binary (or `cargo install --path crates/crw-browse` from the workspace root to put it on `$PATH`):

```bash
cargo build -p crw-browse --release
# Binary at target/release/crw-browse
```

Start a CDP-enabled browser first, then point `crw-browse` at it:

```bash
# Chrome (macOS)
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
  --remote-debugging-port=9222 --headless=new --user-data-dir=/tmp/crw-chrome

# Chrome (Linux)
google-chrome --remote-debugging-port=9222 --headless=new --user-data-dir=/tmp/crw-chrome

# Or Lightpanda
lightpanda serve --host 127.0.0.1 --port 9222
```

Then:

```bash
crw-browse --ws-url ws://localhost:9222
```

> Phase 1 uses a single shared default session. Running two MCP clients
> against the same `crw-browse` process means they share browser state
> (`goto` from one client will affect `tree` from the other). Multi-session
> isolation lands in Phase 2 via `session.new` / `session.close`.

Environment variables (all optional):

| Var | Default | Meaning |
|-----|---------|---------|
| `CRW_BROWSE_WS_URL` | `ws://localhost:9222` | CDP WebSocket endpoint |
| `CRW_BROWSE_PAGE_TIMEOUT_MS` | `30000` | Per-page load timeout |
| `RUST_LOG` | unset | `tracing_subscriber` filter, e.g. `crw_browse=debug` |

### `--ws-url` vs `--chrome-ws-url`: when to use which

`crw-browse` accepts two CDP endpoint flags. They serve different roles:

- **`--ws-url`** is the **primary** CDP endpoint that backs every interactive
  tool (`goto`, `tree`, `click`, `type_text`, `fill`, `wait`, …). It can point
  at either Chrome **or** Lightpanda. Lightpanda is faster and lighter, so
  it's a good default when you don't need screenshots.
- **`--chrome-ws-url`** is a **screenshot-only fallback** that *must* point at
  real Chrome/Chromium. Lightpanda's `Page.captureScreenshot` returns a
  30-byte fake stub, so we route screenshot calls through a separate Chrome
  connection if one is configured.

Three common configurations:

| Setup | `--ws-url` | `--chrome-ws-url` | Tradeoff |
|-------|-----------|-------------------|----------|
| Lightpanda + Chrome screenshot | `ws://lightpanda:9222` | `ws://chrome:9223` | Fast interactive, working screenshots |
| Chrome only (simplest) | `ws://chrome:9222` | `ws://chrome:9222` (same!) | Single browser, screenshots work, slower |
| Lightpanda only (no screenshots) | `ws://lightpanda:9222` | _(unset)_ | Lightest; `screenshot` returns `NOT_IMPLEMENTED` |

Both flags can point at the **same** Chrome instance — there's no harm in
configuring `--chrome-ws-url` when `--ws-url` is also Chrome.

## Claude Code configuration

```json
{
  "mcpServers": {
    "crw-browse": {
      "command": "crw-browse",
      "args": ["--ws-url", "ws://localhost:9222"]
    }
  }
}
```

## Example JSON-RPC flow

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize",...}
← {"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"crw-browse",...}}}

→ {"jsonrpc":"2.0","id":2,"method":"tools/call",
   "params":{"name":"goto","arguments":{"url":"https://example.com"}}}
← {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text",
   "text":"{\"ok\":true,\"session\":\"a3f1\",\"url\":\"https://example.com\",
   \"navigated\":true,\"elapsed_ms\":412,\"data\":{\"status\":200}}"}]}}

→ {"jsonrpc":"2.0","id":3,"method":"tools/call",
   "params":{"name":"tree","arguments":{"max_nodes":50}}}
← {... data.tree = "[1] WebArea: Example Domain\n  [2] heading: Example Domain\n  [3] link: More information\n..." }
```

## Development

- Unit + integration tests: `cargo test -p crw-browse`
- Live walking-skeleton test: `CRW_BROWSE_WS_URL=ws://... cargo test -p crw-browse walking_skeleton`
- Compile + lint: `cargo check -p crw-browse && cargo clippy -p crw-browse`

## Architecture summary

- **`CdpConnection`** (in `crw-renderer`) owns the WebSocket. A single-reader
  event loop routes responses back to the calling task via a pending
  `oneshot::Sender` map keyed by CDP `id`, and broadcasts unmatched messages
  as events.
- **`SessionRegistry`** holds `BrowserSession`s keyed by UUID, with a secondary
  index on 4-char base62 `short_id` tokens (the only thing the LLM sees).
- **Tools** are `#[tool]`-annotated async methods on `CrwBrowse`; the rmcp
  macros generate schemas from `schemars::JsonSchema` derives on the input
  structs.
- **Responses** are `ToolResponse<T>` JSON envelopes wrapped in MCP text
  content. Errors become `ErrorResponse` with `is_error: true`.

License: AGPL-3.0 (same as the rest of the workspace).
