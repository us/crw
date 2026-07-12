/**
 * Request/response types for the CRW SDK.
 *
 * Hand-written against the engine's OpenAPI spec
 * (crates/crw-server/openapi/openapi.json). Results are returned as the engine's
 * raw JSON objects, so the result aliases are intentionally permissive.
 */

export type Json = Record<string, unknown>;

export interface ClientOptions {
  /** Explicit server URL (self-hosted). Defaults to the managed cloud. */
  apiUrl?: string;
  /** API key for the cloud or an authenticated self-hosted server. */
  apiKey?: string;
}

export interface ScrapeOptions {
  formats?: string[];
  onlyMainContent?: boolean;
  includeTags?: string[];
  excludeTags?: string[];
  /** Force the JS renderer on/off (engine `renderJs`). */
  renderJs?: boolean;
  /** Pin a renderer tier (engine `renderer`). */
  renderer?: string;
  /** Milliseconds to wait after load before extracting (`waitFor`). */
  waitFor?: number;
  /** JSON Schema for structured LLM extraction (auto-adds the `json` format). */
  jsonSchema?: Json;
  /**
   * Return per-field evidence alongside `json`. Every top-level scalar property
   * of `jsonSchema` comes back as a {@link Basis}. Requires `jsonSchema`.
   */
  basis?: boolean;
  /** Any other engine scrape option, passed through verbatim. */
  [key: string]: unknown;
}

/** How well an extracted field is attributed to its source. */
export type FieldStatus = "supported" | "unverified" | "unsupported" | "notFound";

/** The citation backing an extracted value. Every field is server-produced. */
export interface EvidenceCitation {
  /** The document the engine fetched. Never a model-supplied url. */
  url: string;
  title?: string;
  /**
   * Verbatim span from the canonical source text. Absent when the span could
   * not be established (`status: "unverified"`).
   */
  excerpt?: string;
  /** `sha256:<hex>` of the exact text sent to the model. */
  sourceHash: string;
  /** Which text `sourceHash` covers. `"llmInput"` for extraction basis. */
  sourceTextKind: string;
}

/**
 * Per-field extraction evidence. Emitted only for top-level scalar schema
 * properties, and only when the request set `basis: true`.
 *
 * `status` is honest: a field whose attribution could not be verified is marked
 * `unverified`/`unsupported` rather than given a fabricated citation.
 * `citations` is empty exactly when `status` is `unsupported` or `notFound`;
 * `value` is `null` exactly when `status` is `notFound`.
 */
export interface Basis {
  basisVersion: number;
  field: string;
  value: unknown | null;
  status: FieldStatus;
  confidence?: "low" | "medium" | "high";
  citations: EvidenceCitation[];
}

/** A coded reason a field's attribution was downgraded. Never upstream text. */
export interface BasisWarning {
  field: string;
  code: string;
}

export interface CrawlOptions {
  maxDepth?: number;
  maxPages?: number;
  pollInterval?: number;
  timeout?: number;
  [key: string]: unknown;
}

export interface MapOptions {
  maxDepth?: number;
  useSitemap?: boolean;
  [key: string]: unknown;
}

export interface SearchOptions {
  limit?: number;
  lang?: string;
  tbs?: string;
  sources?: string[];
  categories?: string[];
  scrapeOptions?: Json;
  [key: string]: unknown;
}

export interface ParseFileOptions {
  filename?: string;
  formats?: string[];
  jsonSchema?: Json;
  parsers?: string[];
  [key: string]: unknown;
}

export interface ExtractOptions {
  urls: string[];
  prompt?: string;
  schema?: Json;
  /**
   * Return per-field evidence: each result carries a `basis` array, one
   * {@link Basis} per top-level scalar schema property. Requires `schema`.
   */
  basis?: boolean;
  /** BYOK: use your own LLM key/provider/model instead of the server's. */
  llmApiKey?: string;
  llmProvider?: string;
  llmModel?: string;
  pollInterval?: number;
  timeout?: number;
}

export interface BatchScrapeOptions {
  formats?: string[];
  pollInterval?: number;
  timeout?: number;
  [key: string]: unknown;
}

export interface ChangeTrackingOptions {
  modes?: string[];
  schema?: Json;
  prompt?: string;
  [key: string]: unknown;
}

export type ScrapeResult = Json;
export type CrawlResult = Json[];
export type SearchResult = Json | Json[];
export type ParseResult = Json;
/** Native `/v1/extract` returns one result object per URL, in request order. */
export type ExtractResult = Json[];
export type BatchResult = Json[];
export type Capabilities = Json;
export type DiffResult = Json;

/** Firecrawl-compatible Research API options (cloud only). */
export interface ResearchSearchOptions {
  k?: number;
  authors?: string;
  categories?: string;
  from?: string;
  to?: string;
}

export interface ResearchReadOptions {
  /** When set, returns top passages answering this question instead of metadata. */
  query?: string;
  k?: number;
}

export interface ResearchSimilarOptions {
  /** Required by Firecrawl: natural-language ranking intent. */
  intent: string;
  mode?: "similar" | "citers" | "references";
  k?: number;
  rerank?: boolean;
  anchor?: string[];
}
