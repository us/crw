#!/usr/bin/env node

const fs = require("fs");
const path = require("path");
const os = require("os");

const AGENTS = [
  { name: "Claude Code", dir: ".claude", flag: "--claude-code" },
  { name: "Cursor", dir: ".cursor", flag: "--cursor" },
  { name: "Gemini CLI", dir: ".gemini", flag: "--gemini-cli" },
  { name: "Codex", dir: ".codex", flag: "--codex" },
  { name: "OpenCode", dir: ".opencode", flag: "--opencode" },
  { name: "Windsurf", dir: ".windsurf", flag: "--windsurf" },
];

const home = os.homedir();
const args = process.argv.slice(2);

function hasFlag(flag) {
  return args.includes(flag);
}

function getApiKey() {
  const idx = args.indexOf("--api-key");
  return idx !== -1 && args[idx + 1] ? args[idx + 1] : null;
}

function readSkillFile() {
  const skillPath = path.join(__dirname, "..", "skills", "SKILL.md");
  return fs.readFileSync(skillPath, "utf-8");
}

function detectAgents() {
  const all = hasFlag("--all");
  const specificFlags = AGENTS.filter((a) => hasFlag(a.flag));

  if (!all && specificFlags.length === 0) {
    // Auto-detect: install to all agents whose config dirs exist
    return AGENTS.filter((a) =>
      fs.existsSync(path.join(home, a.dir))
    );
  }

  if (all) {
    return AGENTS.filter((a) =>
      fs.existsSync(path.join(home, a.dir))
    );
  }

  return specificFlags;
}

function deploy(agents, skillContent) {
  const installed = [];

  for (const agent of agents) {
    const targetDir = path.join(home, agent.dir, "skills", "crw");
    const targetFile = path.join(targetDir, "SKILL.md");

    try {
      fs.mkdirSync(targetDir, { recursive: true });
      fs.writeFileSync(targetFile, skillContent, "utf-8");
      installed.push({ name: agent.name, path: targetFile });
    } catch (err) {
      console.error(`  Failed to install for ${agent.name}: ${err.message}`);
    }
  }

  return installed;
}

function main() {
  if (hasFlag("--help") || hasFlag("-h")) {
    console.log(`
crw-mcp init — Install CRW agent skill to your AI coding agents

Usage:
  npx crw-mcp@latest init [options]

Options:
  --all            Install to all detected agents
  --claude-code    Install to Claude Code only
  --cursor         Install to Cursor only
  --gemini-cli     Install to Gemini CLI only
  --codex          Install to Codex only
  --opencode       Install to OpenCode only
  --windsurf       Install to Windsurf only
  --api-key <key>  Set your fastcrw.com API key
  -h, --help       Show this help message

Without flags, auto-detects installed agents and installs to all of them.
`);
    process.exit(0);
  }

  const skillContent = readSkillFile();
  const agents = detectAgents();

  if (agents.length === 0) {
    console.log("No supported AI agents detected.");
    console.log("Supported agents: " + AGENTS.map((a) => a.name).join(", "));
    console.log("\nManually install by copying the skill file to your agent's skills directory.");
    process.exit(1);
  }

  const installed = deploy(agents, skillContent);

  if (installed.length === 0) {
    console.log("Failed to install to any agent.");
    process.exit(1);
  }

  console.log("crw skill installed:\n");
  const maxName = Math.max(...installed.map((i) => i.name.length));
  for (const i of installed) {
    console.log(`  ${i.name.padEnd(maxName + 2)} ${i.path}`);
  }

  const apiKey = getApiKey();
  if (apiKey) {
    console.log(`\nAPI key configured: ${apiKey.slice(0, 6)}...`);
    console.log("Set it in your environment:");
    console.log(`  export CRW_API_KEY=${apiKey}`);
  } else {
    console.log("\nCloud mode (fastcrw.com):");
    console.log("  export CRW_API_KEY=fc-your-key");
    console.log("  export CRW_API_URL=https://fastcrw.com/api");
  }

  console.log("\nLocal mode (no key needed):");
  console.log("  npx crw-mcp");
  console.log("\nDocs: https://fastcrw.com/docs");
}

main();
