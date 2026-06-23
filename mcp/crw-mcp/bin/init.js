#!/usr/bin/env node

// `crw-mcp init [--<agent>]` — installs the crw SKILL (SKILL.md) only.
// For the skill AND the MCP server in one shot, use `crw-mcp install`.

const {
  AGENTS,
  detectAgents,
  getApiKey,
  readSkill,
  installSkill,
  home,
} = require("./agents.js");

const args = process.argv.slice(2);

if (args.includes("--help") || args.includes("-h")) {
  console.log(`
crw-mcp init — Install the CRW agent SKILL (skill only, no MCP server)

Usage:
  npx crw-mcp@latest init [options]      # skill only
  npx crw-mcp@latest install [options]   # skill + MCP server

Options:
  --all            Install to all detected agents
  --claude-code    Claude Code      --codex      Codex
  --cursor         Cursor           --opencode   OpenCode
  --gemini-cli     Gemini CLI       --windsurf   Windsurf
  --api-key <key>  Your fastcrw.com API key
  -h, --help       Show this help

Without flags, auto-detects installed agents.
`);
  process.exit(0);
}

const skill = readSkill();
const agents = detectAgents(args);

if (agents.length === 0) {
  console.log("No supported AI agents detected.");
  console.log("Supported: " + AGENTS.map((a) => a.name).join(", "));
  process.exit(1);
}

const installed = [];
for (const agent of agents) {
  try {
    installed.push({ name: agent.name, path: installSkill(agent, skill) });
  } catch (err) {
    console.error(`  Failed for ${agent.name}: ${err.message}`);
  }
}
if (installed.length === 0) {
  console.log("Failed to install to any agent.");
  process.exit(1);
}

console.log("crw SKILL installed (skill only — this does NOT add the MCP server):\n");
const maxName = Math.max(...installed.map((i) => i.name.length));
for (const i of installed) {
  console.log(`  ${i.name.padEnd(maxName + 2)} ${i.path.replace(home, "~")}`);
}

console.log("\nWant the MCP tools (crw_scrape / crawl / map / search) too?");
console.log("  npx crw-mcp@latest install        # same agents, skill + MCP server");

const apiKey = getApiKey(args);
if (apiKey) {
  console.log(`\nAPI key set: ${apiKey.slice(0, 10)}… — export it for cloud mode:`);
  console.log(`  export CRW_API_KEY=${apiKey}`);
  console.log("  export CRW_API_URL=https://api.fastcrw.com");
} else {
  console.log("\nThe skill works in local mode out of the box (free, embedded).");
  console.log("Cloud mode (managed, 500 free credits): https://fastcrw.com");
}
console.log("\nDocs: https://fastcrw.com/docs");
