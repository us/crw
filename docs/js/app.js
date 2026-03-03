import { searchEngine } from "./search.js";
import config from "../site.config.js";

// ========== Render Navbar ==========
function renderNavbar() {
  document.querySelector(".logo").textContent = config.name;
  document.title = config.description
    ? `${config.name} — ${config.description}`
    : config.name;

  const navLinks = document.querySelector(".navbar-links");
  navLinks.innerHTML = config.navLinks
    .map((link) => {
      const external = link.external
        ? ' target="_blank" rel="noopener"'
        : "";
      return `<a href="${link.href}"${external}>${link.label}</a>`;
    })
    .join("");

  // Footer
  const footer = document.querySelector(".footer");
  footer.innerHTML = `<span>${config.footer.left}</span><span>${config.footer.right}</span>`;
}

// ========== Apply Custom Theme ==========
function applyThemeOverrides() {
  if (!config.theme) return;

  const style = document.createElement("style");
  let css = "";

  if (config.theme.light) {
    css += '[data-theme="light"] {\n';
    for (const [prop, val] of Object.entries(config.theme.light)) {
      css += `  ${prop}: ${val};\n`;
    }
    css += "}\n";
  }

  if (config.theme.dark) {
    css += '[data-theme="dark"] {\n';
    for (const [prop, val] of Object.entries(config.theme.dark)) {
      css += `  ${prop}: ${val};\n`;
    }
    css += "}\n";
  }

  style.textContent = css;
  document.head.appendChild(style);
}

// ========== Minimal Markdown Parser ==========
function parseMarkdown(md) {
  // If content is already HTML (TMLS sections), return as-is
  if (/^\s*</.test(md)) {
    return md;
  }

  let html = md;

  // Code blocks (fenced) — extract and replace with placeholders
  const codeBlocks = [];
  html = html.replace(
    /```(\w*)\n([\s\S]*?)```/g,
    (_, lang, code) => {
      const escaped = code.trim()
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;");
      const index = codeBlocks.length;
      codeBlocks.push(`<pre><code class="language-${lang}">${escaped}</code></pre>`);
      return `\n%%CODEBLOCK_${index}%%\n`;
    }
  );

  // Inline code — extract and replace with placeholders
  const inlineCode = [];
  html = html.replace(/`([^`]+)`/g, (_, code) => {
    const escaped = code
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
    const index = inlineCode.length;
    inlineCode.push(`<code>${escaped}</code>`);
    return `%%INLINECODE_${index}%%`;
  });

  // Headings
  html = html.replace(/^#### (.+)$/gm, "<h4>$1</h4>");
  html = html.replace(/^### (.+)$/gm, "<h3>$1</h3>");
  html = html.replace(/^## (.+)$/gm, "<h2>$1</h2>");
  html = html.replace(/^# (.+)$/gm, "<h1>$1</h1>");

  // Horizontal rules
  html = html.replace(/^---$/gm, "<hr>");

  // Bold and italic
  html = html.replace(/\*\*\*(.+?)\*\*\*/g, "<strong><em>$1</em></strong>");
  html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");

  // Images (before links)
  html = html.replace(
    /!\[([^\]]*)\]\(([^)]+)\)/g,
    '<img src="$2" alt="$1">'
  );

  // Links
  html = html.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    '<a href="$2">$1</a>'
  );

  // Blockquotes
  html = html.replace(/^&gt; (.+)$/gm, "<blockquote><p>$1</p></blockquote>");

  // Unordered lists
  html = html.replace(/^(\s*)[-*] (.+)$/gm, "$1<li>$2</li>");
  html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, "<ul>$1</ul>");

  // Ordered lists
  html = html.replace(/^\d+\. (.+)$/gm, "<li>$1</li>");

  // Tables
  html = html.replace(
    /^\|(.+)\|\s*\n\|[-| :]+\|\s*\n((?:\|.+\|\s*\n?)*)/gm,
    (_, header, body) => {
      const headers = header
        .split("|")
        .map((h) => h.trim())
        .filter(Boolean);
      const rows = body
        .trim()
        .split("\n")
        .map((row) =>
          row
            .split("|")
            .map((c) => c.trim())
            .filter(Boolean)
        );

      let table = "<table><thead><tr>";
      headers.forEach((h) => (table += `<th>${h}</th>`));
      table += "</tr></thead><tbody>";
      rows.forEach((row) => {
        table += "<tr>";
        row.forEach((cell) => (table += `<td>${cell}</td>`));
        table += "</tr>";
      });
      table += "</tbody></table>";
      return table;
    }
  );

  // Paragraphs
  html = html
    .split("\n\n")
    .map((block) => {
      const trimmed = block.trim();
      if (!trimmed) return "";
      if (/^</.test(trimmed)) return trimmed;
      if (/^%%CODEBLOCK_/.test(trimmed)) return trimmed;
      return `<p>${trimmed.replace(/\n/g, "<br>")}</p>`;
    })
    .join("\n");

  // Restore code blocks and inline code from placeholders
  codeBlocks.forEach((block, i) => {
    html = html.replace(`%%CODEBLOCK_${i}%%`, block);
  });
  inlineCode.forEach((code, i) => {
    html = html.replace(`%%INLINECODE_${i}%%`, code);
  });

  return html;
}

// ========== Strip markdown for search indexing ==========
function stripMarkdown(md) {
  return md
    .replace(/```[\s\S]*?```/g, "")
    .replace(/`[^`]+`/g, "")
    .replace(/[#*_\[\]()>|`-]/g, "")
    .replace(/\n+/g, " ")
    .trim();
}

// ========== Sidebar Rendering ==========
function renderSidebar() {
  const nav = document.getElementById("sidebar-nav");
  const currentSlug = getCurrentSlug();

  nav.innerHTML = config.sidebar
    .map((section) => {
      const hasActiveChild = section.children.some(
        (c) => c.slug === currentSlug
      );
      const isOpen = hasActiveChild;

      return `
        <div class="sidebar-section">
          <button class="sidebar-group-toggle ${isOpen ? "open" : ""}" data-section="${section.title}">
            ${section.title}
            <span class="chevron">&#9654;</span>
          </button>
          <div class="sidebar-group-children ${isOpen ? "open" : ""}">
            ${section.children
              .map(
                (child) => `
              <a href="#${child.slug}" class="sidebar-link ${child.slug === currentSlug ? "active" : ""}">${child.title}</a>
            `
              )
              .join("")}
          </div>
        </div>
      `;
    })
    .join("");

  // Toggle section collapse
  nav.querySelectorAll(".sidebar-group-toggle").forEach((btn) => {
    btn.addEventListener("click", () => {
      btn.classList.toggle("open");
      btn.nextElementSibling.classList.toggle("open");
    });
  });

  // Close sidebar on mobile when clicking a link
  nav.querySelectorAll(".sidebar-link").forEach((link) => {
    link.addEventListener("click", () => {
      if (window.innerWidth <= 768) {
        closeSidebar();
      }
    });
  });
}

// ========== Routing ==========
function getCurrentSlug() {
  return window.location.hash.slice(1) || config.defaultPage;
}

function getPageTitle(slug) {
  for (const section of config.sidebar) {
    const found = section.children.find((c) => c.slug === slug);
    if (found) return found.title;
  }
  return slug;
}

async function loadPage(slug) {
  const article = document.getElementById("article");

  try {
    const response = await fetch(`docs/${slug}.md`);
    if (!response.ok) throw new Error("Not found");
    const md = await response.text();
    article.innerHTML = parseMarkdown(md);
  } catch {
    article.innerHTML = `
      <h1>Page Not Found</h1>
      <p>The page <code>${slug}</code> could not be found.</p>
      <p><a href="#${config.defaultPage}">Go to ${getPageTitle(config.defaultPage)}</a></p>
    `;
  }

  document.title = `${getPageTitle(slug)} — ${config.name}`;
  renderSidebar();
  window.scrollTo(0, 0);
}

// ========== Mobile Sidebar ==========
const hamburger = document.getElementById("hamburger");
const sidebar = document.getElementById("sidebar");
const overlay = document.getElementById("overlay");

function openSidebar() {
  sidebar.classList.add("open");
  overlay.classList.add("active");
  hamburger.classList.add("active");
}

function closeSidebar() {
  sidebar.classList.remove("open");
  overlay.classList.remove("active");
  hamburger.classList.remove("active");
}

hamburger.addEventListener("click", () => {
  sidebar.classList.contains("open") ? closeSidebar() : openSidebar();
});

overlay.addEventListener("click", closeSidebar);

// ========== Search Indexing ==========
async function buildSearchIndex() {
  const pages = [];

  for (const section of config.sidebar) {
    for (const child of section.children) {
      try {
        const response = await fetch(`docs/${child.slug}.md`);
        if (!response.ok) continue;
        const md = await response.text();
        pages.push({
          title: child.title,
          slug: child.slug,
          content: stripMarkdown(md),
        });
      } catch {
        // Skip pages that can't be fetched
      }
    }
  }

  searchEngine.buildIndex(pages);
}

// ========== Init ==========
function init() {
  renderNavbar();
  applyThemeOverrides();
  loadPage(getCurrentSlug());

  window.addEventListener("hashchange", () => {
    loadPage(getCurrentSlug());
  });

  buildSearchIndex();
}

init();
