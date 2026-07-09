import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import { CLOUD_API_URL, CrwClient, CrwError } from "../dist/esm/index.js";

const origFetch = globalThis.fetch;
const origEnv = { ...process.env };

beforeEach(() => {
  delete process.env.CRW_LOCAL;
  delete process.env.CRW_API_KEY;
  delete process.env.CRW_API_URL;
});

afterEach(() => {
  globalThis.fetch = origFetch;
  process.env = { ...origEnv };
});

function mockFetch(body: unknown, ok = true, status = 200) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  globalThis.fetch = (async (url: string, init?: RequestInit) => {
    calls.push({ url: String(url), init });
    return {
      ok,
      status,
      statusText: "OK",
      text: async () => JSON.stringify(body),
    } as Response;
  }) as typeof fetch;
  return calls;
}

test("cloud-first: no key throws onboarding nudge", () => {
  assert.throws(() => new CrwClient(), /500 free credits/);
});

test("cloud-first: with key targets the cloud", () => {
  const c = new CrwClient({ apiKey: "fc-test" });
  assert.equal((c as unknown as { apiUrl: string }).apiUrl, CLOUD_API_URL);
});

test("explicit apiUrl does not require a key", () => {
  const c = new CrwClient({ apiUrl: "http://localhost:3000" });
  assert.equal((c as unknown as { apiUrl: string }).apiUrl, "http://localhost:3000");
});

test("CRW_LOCAL=1 enables local mode (apiUrl null)", () => {
  process.env.CRW_LOCAL = "1";
  const c = new CrwClient();
  assert.equal((c as unknown as { apiUrl: string | null }).apiUrl, null);
});

test("scrape maps first-class params + jsonSchema adds json format", async () => {
  const calls = mockFetch({ success: true, data: { markdown: "x" } });
  const c = new CrwClient({ apiKey: "fc-test" });
  await c.scrape("https://example.com", { formats: ["markdown"], renderJs: true, waitFor: 1500, jsonSchema: { type: "object" } });
  const body = JSON.parse((calls[0].init!.body as string));
  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/scrape`);
  assert.equal(body.renderJs, true);
  assert.equal(body.waitFor, 1500);
  assert.deepEqual(body.jsonSchema, { type: "object" });
  assert.ok(body.formats.includes("json") && body.formats.includes("markdown"));
});

test("HTTP-only methods throw in local mode", async () => {
  process.env.CRW_LOCAL = "1";
  const c = new CrwClient();
  await assert.rejects(() => c.extract({ urls: ["https://example.com"] }), /requires HTTP mode/);
  await assert.rejects(() => c.batchScrape(["https://example.com"]), /requires HTTP mode/);
  await assert.rejects(() => c.capabilities(), /requires HTTP mode/);
  await assert.rejects(() => c.changeTrackingDiff({ markdown: "a" }), /requires HTTP mode/);
});

test("non-2xx body surfaces engine error as CrwApiError", async () => {
  mockFetch({ error: "boom" }, false, 400);
  const c = new CrwClient({ apiKey: "fc-test" });
  await assert.rejects(() => c.scrape("https://example.com"), /boom/);
});

test("extract starts a /v1/extract job and returns per-URL results", async () => {
  let n = 0;
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  globalThis.fetch = (async (url: string, init?: RequestInit) => {
    calls.push({ url: String(url), init });
    const body =
      n++ === 0
        ? { success: true, id: "job-1" }
        : {
            success: true,
            status: "completed",
            results: [{ url: "https://example.com", status: "completed", data: { title: "Hi" } }],
          };
    return { ok: true, status: 200, statusText: "OK", text: async () => JSON.stringify(body) } as Response;
  }) as typeof fetch;

  const c = new CrwClient({ apiKey: "fc-test" });
  const results = await c.extract({ urls: ["https://example.com"], schema: { type: "object" }, llmApiKey: "sk" });

  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/extract`);
  const postBody = JSON.parse(calls[0].init!.body as string);
  assert.deepEqual(postBody.urls, ["https://example.com"]);
  assert.equal(postBody.llmApiKey, "sk");
  assert.ok(calls[1].url.startsWith(`${CLOUD_API_URL}/v1/extract/job-1`));
  assert.equal(results.length, 1);
  const r0 = results[0] as { url: string; data: { title: string } };
  assert.equal(r0.url, "https://example.com");
  assert.equal(r0.data.title, "Hi");
});

test("capabilities unwraps and uses GET /v1/capabilities", async () => {
  const calls = mockFetch({ version: "0.14.0" });
  const c = new CrwClient({ apiKey: "fc-test" });
  const caps = await c.capabilities();
  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/capabilities`);
  assert.equal((caps as { version: string }).version, "0.14.0");
});

// silence unused import lint in some configs
void CrwError;
