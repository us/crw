// Coverage for the hand-rolled SHA256SUMS parser. It exists because Windows
// ships no sha256sum, and Windows is the only platform with no prebuilt
// package, so every Windows user depends on this code path.
const { test } = require("node:test");
const assert = require("node:assert");

const { digestFor } = require("../bin/crw-mcp.js");

const H = "a".repeat(64);
const ASSET = "crw-mcp-win32-x64.zip";

test("two-space coreutils form", () => {
  assert.equal(digestFor(`${H}  ${ASSET}\n`, ASSET), H);
});

test("binary-mode marker and CRLF", () => {
  assert.equal(digestFor(`${H} *${ASSET}\r\n`, ASSET), H);
});

test("finds the entry among many", () => {
  const body = `${H}  other.tar.gz\r\n${H}  ${ASSET}\r\n${H}  third.zip\n`;
  assert.equal(digestFor(body, ASSET), H);
});

test("unlisted asset returns null", () => {
  assert.equal(digestFor(`${H}  other.tar.gz\n`, ASSET), null);
});

test("a name that merely contains the asset is not a match", () => {
  assert.equal(digestFor(`${H}  prefix-${ASSET}\n`, ASSET), null);
});

test("an HTML error page served with 200 yields nothing", () => {
  const html = "<html><head><title>404</title></head><body>Not Found</body></html>";
  assert.equal(digestFor(html, ASSET), null);
});

test("empty body yields nothing", () => {
  assert.equal(digestFor("", ASSET), null);
});

test("an uppercase digest is normalised", () => {
  // Keeps the two parsers in step: Python lowercases too, so a SHA256SUMS
  // written by a tool emitting uppercase hex cannot pass on one and fail on
  // the other.
  assert.equal(digestFor(`${H.toUpperCase()}  ${ASSET}\n`, ASSET), H);
});
