import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import {
  CLOUD_API_URL,
  CrwClient,
  CrwError,
  CrwExtractCancelledError,
  CrwTimeoutError,
} from "../dist/esm/index.js";

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
  const c = new CrwClient({ apiKey: "crw_live_test" });
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
  const c = new CrwClient({ apiKey: "crw_live_test" });
  await c.scrape("https://example.com", { formats: ["markdown"], renderJs: true, waitFor: 1500, jsonSchema: { type: "object" } });
  const body = JSON.parse((calls[0].init!.body as string));
  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/scrape`);
  assert.equal(body.renderJs, true);
  assert.equal(body.waitFor, 1500);
  assert.deepEqual(body.jsonSchema, { type: "object" });
  assert.ok(body.formats.includes("json") && body.formats.includes("markdown"));
});

test("scrape result exposes typed document fields", async () => {
  mockFetch({
    success: true,
    data: {
      markdown: "# Hi",
      links: ["https://example.com/a"],
      images: [{ url: "https://example.com/i.png", alt: "i" }],
      metadata: { title: "Ex", sourceURL: "https://example.com", statusCode: 200, elapsedMs: 12 },
    },
  });
  const c = new CrwClient({ apiKey: "crw_live_test" });
  const result = await c.scrape("https://example.com");
  // These property accesses only compile because ScrapeResult is now typed.
  const md: string | undefined = result.markdown;
  const src: string = result.metadata.sourceURL;
  const code: number = result.metadata.statusCode;
  const firstLink: string | undefined = result.links?.[0];
  const alt: string | undefined = result.images?.[0].alt;
  assert.equal(md, "# Hi");
  assert.equal(src, "https://example.com");
  assert.equal(code, 200);
  assert.equal(firstLink, "https://example.com/a");
  assert.equal(alt, "i");
});

test("search result carries typed items and answer siblings", async () => {
  mockFetch({
    success: true,
    data: [{ url: "https://example.com", title: "Ex", description: "d", position: 1 }],
    answer: "42",
    citations: [{ url: "https://example.com", title: "Ex", position: 1 }],
  });
  const c = new CrwClient({ apiKey: "crw_live_test" });
  const results = await c.search("q");
  assert.ok(Array.isArray(results));
  if (Array.isArray(results)) {
    const title: string = results[0].title;
    assert.equal(title, "Ex");
  }
  // `answer` rides alongside the array (attached non-enumerably by the client).
  assert.equal((results as { answer?: string }).answer, "42");
});

test("capabilities result is a typed feature payload", async () => {
  mockFetch({
    version: "0.27.1",
    llm: { providers: ["managed"], supportsBaseUrl: false, serverKeyConfigured: true, maxConcurrency: 4 },
    formats: { supported: ["markdown"], llmRequired: [], changeTrackingModes: [], changeTrackingModesLlmRequired: [] },
    search: { supported: true, answer: true, summarizeResults: true },
    screenshot: { supported: false, fullPage: false },
    renderers: { available: ["chrome"], mode: "auto" },
    extract: { supported: true, maxUrls: 10, perFieldAttribution: true, maxOutputTokens: 4096 },
    documents: { parsers: ["pdf"], fileUpload: { supported: true, endpoint: "/v2/parse", maxBytes: 1, types: ["pdf"], ocr: false } },
    limits: { maxBatchUrls: 1, maxExtractUrls: 1, searchDefaultLimit: 5, searchMaxLimit: 20, maxUploadBytes: 1 },
  });
  const c = new CrwClient({ apiKey: "crw_live_test" });
  const caps = await c.capabilities();
  const version: string = caps.version;
  const providers: string[] = caps.llm.providers;
  assert.equal(version, "0.27.1");
  assert.deepEqual(providers, ["managed"]);
});

test("HTTP-only methods throw in local mode", async () => {
  process.env.CRW_LOCAL = "1";
  const c = new CrwClient();
  await assert.rejects(() => c.extract({ urls: ["https://example.com"] }), /requires HTTP mode/);
  await assert.rejects(() => c.startExtract({ urls: ["https://example.com"] }), /requires HTTP mode/);
  await assert.rejects(() => c.getExtract("job-1"), /requires HTTP mode/);
  await assert.rejects(() => c.cancelExtract("job-1"), /requires HTTP mode/);
  await assert.rejects(() => c.batchScrape(["https://example.com"]), /requires HTTP mode/);
  await assert.rejects(() => c.capabilities(), /requires HTTP mode/);
  await assert.rejects(() => c.changeTrackingDiff({ markdown: "a" }), /requires HTTP mode/);
});

test("non-2xx body surfaces engine error as CrwApiError", async () => {
  mockFetch({ error: "boom" }, false, 400);
  const c = new CrwClient({ apiKey: "crw_live_test" });
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

  const c = new CrwClient({ apiKey: "crw_live_test" });
  const results = await c.extract({ urls: ["https://example.com"], schema: { type: "object" }, llmApiKey: "sk" });

  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/extract`);
  const postBody = JSON.parse(calls[0].init!.body as string);
  assert.deepEqual(postBody.urls, ["https://example.com"]);
  assert.equal(postBody.llmApiKey, "sk");
  assert.equal((calls[0].init!.headers as Record<string, string>).Prefer, "respond-async");
  assert.ok(calls[1].url.startsWith(`${CLOUD_API_URL}/v1/extract/job-1`));
  assert.equal(results.length, 1);
  const r0 = results[0] as { url: string; data: { title: string } };
  assert.equal(r0.url, "https://example.com");
  assert.equal(r0.data.title, "Hi");
});

test("startExtract sends Prefer for managed and self-hosted fixtures", async () => {
  const calls = mockFetch({ id: "job-1", status: "processing", urls: 1 });
  const managed = new CrwClient({ apiKey: "crw_live_test" });
  assert.equal((await managed.startExtract({ urls: ["https://example.com"], prompt: "x" })).id, "job-1");
  const selfHosted = new CrwClient({ apiUrl: "http://localhost:3000" });
  await selfHosted.startExtract({ urls: ["https://example.com"], schema: { type: "object" } });

  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/extract`);
  assert.equal(calls[1].url, "http://localhost:3000/v1/extract");
  for (const call of calls) {
    assert.equal((call.init!.headers as Record<string, string>).Prefer, "respond-async");
  }
});

test("extract preserves managed synchronous fixture while requesting async", async () => {
  const calls = mockFetch({
    success: true,
    results: [{ url: "https://example.com", status: "completed", data: { title: "Hi" } }],
  });
  const c = new CrwClient({ apiKey: "crw_live_test" });
  const results = await c.extract({ urls: ["https://example.com"], prompt: "title" });
  assert.equal(results[0].status, "completed");
  assert.equal((calls[0].init!.headers as Record<string, string>).Prefer, "respond-async");
});

test("getExtract and cancelExtract use canonical status route", async () => {
  const status = {
    id: "job/one",
    status: "cancelled",
    results: [{ url: "https://example.com", status: "cancelled" }],
    expiresAt: "2026-07-14T00:00:00Z",
    creditsUsed: 0,
    tokensUsed: 0,
  };
  const calls = mockFetch(status);
  const c = new CrwClient({ apiUrl: "http://localhost:3000" });
  assert.equal((await c.getExtract("job/one")).status, "cancelled");
  assert.equal((await c.cancelExtract("job/one")).results.length, 1);
  assert.equal(calls[0].url, "http://localhost:3000/v1/extract/job%2Fone");
  assert.equal(calls[0].init!.method, "GET");
  assert.equal(calls[1].init!.method, "DELETE");
});

test("extract treats cancelling as non-terminal then raises typed cancelled error", async () => {
  const responses = [
    { id: "job-1", status: "processing", urls: 2 },
    {
      id: "job-1",
      status: "cancelling",
      results: [
        { url: "https://a.example", status: "completed", data: { title: "A" } },
        { url: "https://b.example", status: "processing" },
      ],
      expiresAt: "2026-07-14T00:00:00Z",
      creditsUsed: 1,
      tokensUsed: 9,
    },
    {
      id: "job-1",
      status: "cancelled",
      results: [
        { url: "https://a.example", status: "completed", data: { title: "A" } },
        { url: "https://b.example", status: "cancelled" },
      ],
      expiresAt: "2026-07-14T00:00:00Z",
      creditsUsed: 1,
      tokensUsed: 9,
    },
  ];
  globalThis.fetch = (async () => {
    const body = responses.shift();
    return { ok: true, status: 200, statusText: "OK", text: async () => JSON.stringify(body) } as Response;
  }) as typeof fetch;
  const c = new CrwClient({ apiUrl: "http://localhost:3000" });
  await assert.rejects(
    () => c.extract({ urls: ["https://a.example", "https://b.example"], prompt: "x", pollInterval: 0 }),
    (error: unknown) => {
      assert.ok(error instanceof CrwExtractCancelledError);
      assert.equal(error.status.status, "cancelled");
      assert.equal(error.results[0].status, "completed");
      return true;
    },
  );
});

test("extract timeout performs best-effort DELETE", async () => {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  globalThis.fetch = (async (url: string, init?: RequestInit) => {
    calls.push({ url: String(url), init });
    const body = init?.method === "POST"
      ? { id: "job-1", status: "processing", urls: 1 }
      : {
          id: "job-1",
          status: "cancelled",
          results: [{ url: "https://example.com", status: "cancelled" }],
          expiresAt: "2026-07-14T00:00:00Z",
          creditsUsed: 0,
          tokensUsed: 0,
        };
    return { ok: true, status: 200, statusText: "OK", text: async () => JSON.stringify(body) } as Response;
  }) as typeof fetch;
  const c = new CrwClient({ apiUrl: "http://localhost:3000" });
  await assert.rejects(
    () => c.extract({ urls: ["https://example.com"], prompt: "x", timeout: -1 }),
    CrwTimeoutError,
  );
  assert.equal(calls.at(-1)?.init?.method, "DELETE");
});

test("capabilities unwraps and uses GET /v1/capabilities", async () => {
  const calls = mockFetch({ version: "0.14.0" });
  const c = new CrwClient({ apiKey: "crw_live_test" });
  const caps = await c.capabilities();
  assert.equal(calls[0].url, `${CLOUD_API_URL}/v1/capabilities`);
  assert.equal((caps as { version: string }).version, "0.14.0");
});

// silence unused import lint in some configs
void CrwError;
