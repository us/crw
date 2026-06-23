#!/usr/bin/env node

// Handle install/init subcommands before delegating to the Rust binary.
// `init` = skill only; `install` = skill + MCP server.
if (process.argv[2] === "init") {
  require("./init.js");
  process.exit(0);
}
if (process.argv[2] === "install") {
  require("./install.js");
  process.exit(0);
}

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const os = require("os");
const https = require("https");

const VERSION = require("../package.json").version;
const REPO = "us/crw";

// Each platform has a prebuilt npm package (fast path, installed via
// optionalDependencies) AND a matching GitHub release asset (fallback path,
// downloaded on first run). The fallback makes the launcher robust to a
// missing/unpublishable platform package (e.g. npm security-held names) — no
// single package can freeze the channel, mirroring the PyPI launcher.
const PLATFORMS = {
  "darwin-x64": { pkg: "crw-mcp-darwin-x64", asset: "crw-mcp-darwin-x64.tar.gz" },
  "darwin-arm64": { pkg: "crw-mcp-darwin-arm64", asset: "crw-mcp-darwin-arm64.tar.gz" },
  "linux-x64": { pkg: "crw-mcp-linux-x64", asset: "crw-mcp-linux-x64.tar.gz" },
  "linux-arm64": { pkg: "crw-mcp-linux-arm64", asset: "crw-mcp-linux-arm64.tar.gz" },
  "win32-x64": { pkg: "crw-mcp-win32-x64", asset: "crw-mcp-win32-x64.zip" },
  "win32-arm64": { pkg: "crw-mcp-win32-arm64", asset: "crw-mcp-win32-arm64.zip" },
};

const key = `${process.platform}-${process.arch}`;
const plat = PLATFORMS[key];

if (!plat) {
  console.error(
    `crw-mcp: unsupported platform ${key}. Supported: ${Object.keys(PLATFORMS).join(", ")}`
  );
  process.exit(1);
}

const binName = `crw-mcp${process.platform === "win32" ? ".exe" : ""}`;

// 1. Explicit override.
function fromEnv() {
  const p = process.env.CRW_MCP_BINARY || process.env.CRW_BINARY;
  return p && fs.existsSync(p) ? p : null;
}

// 2. Prebuilt platform package (fast path — no download).
function fromPackage() {
  try {
    const bin = path.join(
      path.dirname(require.resolve(`${plat.pkg}/package.json`)),
      binName
    );
    return fs.existsSync(bin) ? bin : null;
  } catch {
    return null;
  }
}

function cacheDir() {
  const base =
    process.platform === "win32"
      ? process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local")
      : process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
  return path.join(base, "crw-mcp", `v${VERSION}`);
}

function httpsGet(url, dest, redirects = 0) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": `crw-mcp/${VERSION}` } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          res.resume();
          if (redirects > 5) return reject(new Error("too many redirects"));
          return resolve(httpsGet(res.headers.location, dest, redirects + 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const file = fs.createWriteStream(dest);
        res.pipe(file);
        file.on("finish", () => file.close(() => resolve()));
        file.on("error", reject);
      })
      .on("error", reject);
  });
}

// 3. Download the matching release asset and extract it (cached per version).
async function fromDownload() {
  const dir = cacheDir();
  const bin = path.join(dir, binName);
  if (fs.existsSync(bin)) return bin;

  fs.mkdirSync(dir, { recursive: true });
  const archive = path.join(dir, plat.asset);
  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${plat.asset}`;

  // stderr only: stdout is the MCP (JSON-RPC) channel.
  console.error(`crw-mcp: downloading ${plat.asset} (v${VERSION})...`);
  await httpsGet(url, archive);

  const tarArgs = plat.asset.endsWith(".zip")
    ? ["-xf", archive, "-C", dir]
    : ["-xzf", archive, "-C", dir];
  const x = spawnSync("tar", tarArgs, { stdio: "inherit" });
  if (x.status !== 0) {
    throw new Error(`failed to extract ${plat.asset} (tar exit ${x.status})`);
  }
  fs.rmSync(archive, { force: true });

  if (!fs.existsSync(bin)) {
    throw new Error(`binary ${binName} not found in ${plat.asset}`);
  }
  if (process.platform !== "win32") fs.chmodSync(bin, 0o755);
  return bin;
}

async function resolveBinary() {
  return fromEnv() || fromPackage() || (await fromDownload());
}

resolveBinary()
  .then((bin) => {
    const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
    process.exit(result.status ?? 1);
  })
  .catch((err) => {
    console.error(
      `crw-mcp: could not locate or download the ${key} binary.\n  ${err.message}\n` +
        `  Set CRW_MCP_BINARY=/path/to/crw-mcp to use a local build, or install\n` +
        `  from https://github.com/${REPO}/releases.`
    );
    process.exit(1);
  });
