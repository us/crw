# Examples

Runnable snippets showing how to use CRW from popular frameworks. These are
**not published packages** — copy what you need.

| Dir | Framework | How it talks to CRW | Replaces |
|---|---|---|---|
| `langchain/` | LangChain | `crw[langchain]` extra (`crw.integrations.langchain`) | the old `langchain-crw` package |
| `crewai/` | CrewAI | `crw[crewai]` extra (`crw.integrations.crewai`) | the old `crewai-crw` package |
| `openclaw/` | OpenClaw | the `crw-mcp` MCP server (MCP-native) | the old `openclaw-plugin-crw` package |
| `pi/` | Pi | the `crw-mcp` MCP server (MCP-native) | the old `pi-crw` package |

LangChain/CrewAI integrations ship inside the `crw` SDK as optional extras.
Agent frameworks that speak MCP (OpenClaw, Pi, Claude, Cursor, …) just point at
`crw-mcp` — no bespoke package needed.
