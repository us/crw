#!/usr/bin/env node

const { spawnSync } = require("child_process");
const path = require("path");

const PLATFORMS = {
  "darwin-x64": "crw-mcp-darwin-x64",
  "darwin-arm64": "crw-mcp-darwin-arm64",
  "linux-x64": "crw-mcp-linux-x64",
  "linux-arm64": "crw-mcp-linux-arm64",
  "win32-x64": "crw-mcp-win32-x64",
  "win32-arm64": "crw-mcp-win32-arm64",
};

const key = `${process.platform}-${process.arch}`;
const pkg = PLATFORMS[key];

if (!pkg) {
  console.error(
    `crw-mcp: unsupported platform ${key}. Supported: ${Object.keys(PLATFORMS).join(", ")}`
  );
  process.exit(1);
}

const ext = process.platform === "win32" ? ".exe" : "";

let bin;
try {
  bin = path.join(
    path.dirname(require.resolve(`${pkg}/package.json`)),
    `crw-mcp${ext}`
  );
} catch {
  console.error(
    `crw-mcp: platform package "${pkg}" not found. Try reinstalling:\n  npm install crw-mcp`
  );
  process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
process.exit(result.status ?? 1);
