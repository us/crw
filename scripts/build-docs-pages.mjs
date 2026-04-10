/**
 * Static docs page generator for docs.fastcrw.com
 *
 * Generates docs/{slug}/index.html for each sidebar entry so Google can
 * index each docs page as a separate URL instead of a single hash-based SPA.
 *
 * Usage:
 *   npm install marked
 *   node scripts/build-docs-pages.mjs
 */

import { readFile, writeFile, mkdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { marked } from "marked";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, "..");
const DOCS_DIR = path.join(ROOT, "docs");
const CONTENT_DIR = path.join(DOCS_DIR, "docs");
const BASE_URL = "https://docs.fastcrw.com";

// ── Load config ──────────────────────────────────────────────────────────────
const { default: config } = await import(
  path.join(DOCS_DIR, "site.config.js")
);

// Flat list of { slug, title, priority } from sidebar
const slugMeta = config.sidebar.flatMap((section) =>
  section.children.map((child) => ({
    slug: child.slug,
    title: child.title,
    sectionTitle: section.title,
  }))
);

// Slug → title lookup
const slugTitles = Object.fromEntries(slugMeta.map((m) => [m.slug, m.title]));

// Priority per section (higher for core endpoints)
const sectionPriority = {
  "Get Started": 0.9,
  "Core Endpoints": 0.85,
  "More APIs": 0.8,
  Integrations: 0.75,
  Deploy: 0.7,
  Reference: 0.6,
};

// ── Load index.html template ──────────────────────────────────────────────────
let template = await readFile(path.join(DOCS_DIR, "index.html"), "utf8");

// Make all local asset paths absolute (so subdir pages can load them)
template = template
  .replace(/href="css\//g, 'href="/css/')
  .replace(/href="js\//g, 'href="/js/')
  .replace(/href="site\.config\.js"/g, 'href="/site.config.js"')
  .replace(/src="js\//g, 'src="/js/')
  .replace(/src="logo/g, 'src="/logo')
  .replace(/src="favicon/g, 'src="/favicon')
  // Remove introduction.md preload (not needed for static pages)
  .replace(/\s*<link rel="preload" href="docs\/introduction\.md"[^>]*>\n?/g, "");

// ── Configure marked ──────────────────────────────────────────────────────────
marked.setOptions({ gfm: true, breaks: false });

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Strip HTML tags and return plain text */
function stripTags(html) {
  return html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
}

/** Extract description from page-subtitle or first <p> */
function extractDescription(mdContent) {
  const subtitleMatch = mdContent.match(
    /<p class="page-subtitle">([\s\S]*?)<\/p>/
  );
  if (subtitleMatch) return stripTags(subtitleMatch[1]).slice(0, 200);

  // Fallback: first paragraph of plain markdown
  const paraMatch = mdContent.match(/^(?!#|<|[-*]|\d\.)(.{40,})/m);
  if (paraMatch) return paraMatch[1].trim().slice(0, 200);

  return `${config.description || "CRW documentation"}.`;
}

/** Rewrite internal #slug links to /slug in rendered HTML */
function rewriteInternalLinks(html) {
  // href="#slug" → href="/slug" for known slugs
  return html.replace(/href="#([a-z][a-z0-9-]*)"/g, (match, slug) => {
    if (slugTitles[slug]) return `href="/${slug}"`;
    return match; // leave in-page anchors alone
  });
}

/** Build a page HTML from the template */
function buildPage(slug, title, description, content) {
  let page = template;

  // ── Head: per-page metadata ──
  page = page.replace(
    /<title>[^<]*<\/title>/,
    `<title>${escHtml(title)} — CRW Docs</title>`
  );
  page = page.replace(
    /<meta name="description" content="[^"]*">/,
    `<meta name="description" content="${escHtml(description)}">`
  );
  page = page.replace(
    /<link rel="canonical" href="[^"]*">/,
    `<link rel="canonical" href="${BASE_URL}/${slug}">`
  );

  // OG
  page = page.replace(
    /<meta property="og:url" content="[^"]*">/,
    `<meta property="og:url" content="${BASE_URL}/${slug}">`
  );
  page = page.replace(
    /<meta property="og:title" content="[^"]*">/,
    `<meta property="og:title" content="${escHtml(title)} — CRW | Web Scraper in Rust">`
  );
  page = page.replace(
    /<meta property="og:description" content="[^"]*">/,
    `<meta property="og:description" content="${escHtml(description)}">`
  );

  // Twitter
  page = page.replace(
    /<meta name="twitter:title" content="[^"]*">/,
    `<meta name="twitter:title" content="${escHtml(title)} — CRW Docs">`
  );
  page = page.replace(
    /<meta name="twitter:description" content="[^"]*">/,
    `<meta name="twitter:description" content="${escHtml(description)}">`
  );

  // ── Article: inject pre-rendered content ──
  page = page.replace(
    /<article id="article" role="main"><\/article>/,
    `<article id="article" role="main">${content}</article>`
  );

  // ── __INITIAL_SLUG__ before </body> ──
  page = page.replace(
    "</body>",
    `  <script>window.__INITIAL_SLUG__ = ${JSON.stringify(slug)};</script>\n</body>`
  );

  return page;
}

function escHtml(str) {
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// ── Generate pages ────────────────────────────────────────────────────────────
let generated = 0;
const errors = [];

for (const { slug, title } of slugMeta) {
  const mdPath = path.join(CONTENT_DIR, `${slug}.md`);

  if (!existsSync(mdPath)) {
    errors.push(`  MISSING: docs/docs/${slug}.md`);
    continue;
  }

  const mdContent = await readFile(mdPath, "utf8");
  const description = extractDescription(mdContent);

  // Convert markdown → HTML (marked passes raw HTML blocks through unchanged)
  const rawHtml = marked.parse(mdContent);

  // Rewrite #slug → /slug for internal cross-references
  const content = rewriteInternalLinks(rawHtml);

  const pageHtml = buildPage(slug, title, description, content);

  // Write to docs/{slug}/index.html
  const outDir = path.join(DOCS_DIR, slug);
  await mkdir(outDir, { recursive: true });
  await writeFile(path.join(outDir, "index.html"), pageHtml, "utf8");
  generated++;
}

// ── Regenerate sitemap.xml ────────────────────────────────────────────────────
const now = new Date().toISOString().split("T")[0];

const urlEntries = [
  `  <url><loc>${BASE_URL}/</loc><priority>1.0</priority><changefreq>weekly</changefreq><lastmod>${now}</lastmod></url>`,
  ...slugMeta.map(({ slug, sectionTitle }) => {
    const priority = sectionPriority[sectionTitle] ?? 0.6;
    return `  <url><loc>${BASE_URL}/${slug}</loc><priority>${priority}</priority><changefreq>monthly</changefreq><lastmod>${now}</lastmod></url>`;
  }),
];

const sitemap = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urlEntries.join("\n")}
</urlset>
`;

await writeFile(path.join(DOCS_DIR, "sitemap.xml"), sitemap, "utf8");

// ── Summary ───────────────────────────────────────────────────────────────────
console.log(`\n✓ Generated ${generated}/${slugMeta.length} static pages`);
console.log(`✓ Updated docs/sitemap.xml (${slugMeta.length + 1} URLs)`);
if (errors.length) {
  console.warn(`\nWarnings:`);
  errors.forEach((e) => console.warn(e));
}
