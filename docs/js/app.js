import { searchEngine } from "./search.js";
import config from "../site.config.js";

const features = Object.assign(
  { scrollReveal: true, codeCopyButtons: true, readingProgress: true, skeletonLoading: true },
  config.features || {}
);

// ========== Render Navbar ==========
function renderNavbar() {
  // Logo: image or text
  const logoEl = document.querySelector(".logo");
  if (config.logo) {
    logoEl.innerHTML = `<img src="${config.logo}" alt="${config.name}" class="logo-img logo-light" /><img src="${config.logoDark || config.logo}" alt="${config.name}" class="logo-img logo-dark" /><span class="logo-text">fast<span style="color:#16A34A">crw</span></span>`;
  } else {
    logoEl.textContent = config.name;
  }

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

  // Profile link (author branding)
  const profileLink = document.getElementById("profile-link");
  if (profileLink && config.author?.url) {
    profileLink.href = config.author.url;
    profileLink.textContent = config.author.name || "us";
    profileLink.title = config.author.url;
  }

  // Footer
  const footer = document.querySelector(".footer");
  const authorName = config.author?.name || "us";
  const authorUrl = config.author?.url || "https://github.com/us";
  const githubLink = config.navLinks?.find(l => l.href?.includes("github.com"))?.href || "#";

  let footerLeft = config.footer?.left || "";
  let footerRight = config.footer?.right || "";

  footer.innerHTML = `
    <div class="footer-col">
      <span class="footer-project">${config.name}</span>
      <span class="footer-license">${footerLeft}</span>
    </div>
    <div class="footer-col footer-col-right">
      ${githubLink !== "#" ? `<a href="${githubLink}" target="_blank" rel="noopener">GitHub</a>` : ""}
      <a href="${authorUrl}" target="_blank" rel="noopener">Created by ${authorName}</a>
      <span class="footer-license">${footerRight}</span>
    </div>
  `;
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
  if (/^\s*</.test(md)) {
    return md;
  }

  let html = md;

  // Extract code blocks and inline code FIRST to protect from further parsing
  const codeBlocks = [];
  const inlineCodes = [];

  // Code blocks (fenced) — extract and replace with placeholders
  html = html.replace(
    /```(\w*)\n([\s\S]*?)```/g,
    (_, lang, code) => {
      const escaped = code.trim()
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;");
      const langAttr = lang ? ` class="language-${lang}"` : "";
      const dataLang = lang ? ` data-lang="${lang}"` : "";
      const placeholder = `\x00CODEBLOCK${codeBlocks.length}\x00`;
      codeBlocks.push(`<pre${dataLang}><code${langAttr}>${escaped}</code></pre>`);
      return placeholder;
    }
  );

  // Inline code — extract and replace with placeholders
  html = html.replace(/`([^`]+)`/g, (_, code) => {
    const placeholder = `\x00INLINECODE${inlineCodes.length}\x00`;
    inlineCodes.push(`<code>${code}</code>`);
    return placeholder;
  });

  // Now safe to parse markdown — code content is protected

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
    '<img src="$2" alt="$1" loading="lazy">'
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
      if (/^\x00CODEBLOCK/.test(trimmed)) return trimmed;
      return `<p>${trimmed.replace(/\n/g, "<br>")}</p>`;
    })
    .join("\n");

  // Restore code blocks and inline code
  codeBlocks.forEach((block, i) => {
    html = html.replace(`\x00CODEBLOCK${i}\x00`, block);
  });
  inlineCodes.forEach((code, i) => {
    html = html.replace(`\x00INLINECODE${i}\x00`, code);
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
// SVG icons for quick links
const quickLinkIcons = {
  github: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"/></svg>',
  star: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>',
  users: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>',
  list: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>',
  book: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>',
  external: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
};

function renderSidebar() {
  const nav = document.getElementById("sidebar-nav");
  const currentSlug = getCurrentSlug();

  // Quick links
  const quickLinksHTML = (config.quickLinks || []).length > 0
    ? `<div class="sidebar-quick-links">${config.quickLinks.map((link) => {
        const icon = quickLinkIcons[link.icon] || quickLinkIcons.external;
        const ext = link.external ? ' target="_blank" rel="noopener"' : '';
        return `<a href="${link.href}" class="sidebar-quick-link"${ext}><span class="sidebar-quick-icon">${icon}</span>${link.title}</a>`;
      }).join('')}</div>`
    : '';

  nav.innerHTML = quickLinksHTML + config.sidebar
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

// ========== Skeleton Loading ==========
function showContentSkeleton() {
  if (!features.skeletonLoading) return;
  const article = document.getElementById("article");
  article.innerHTML = `
    <div class="skeleton" style="width:45%;height:32px;margin-bottom:20px"></div>
    <div class="skeleton" style="width:100%;height:14px;margin-bottom:10px"></div>
    <div class="skeleton" style="width:92%;height:14px;margin-bottom:10px"></div>
    <div class="skeleton" style="width:78%;height:14px;margin-bottom:28px"></div>
    <div class="skeleton" style="width:55%;height:24px;margin-bottom:16px"></div>
    <div class="skeleton" style="width:100%;height:14px;margin-bottom:10px"></div>
    <div class="skeleton" style="width:85%;height:14px;margin-bottom:10px"></div>
    <div class="skeleton" style="width:96%;height:14px;margin-bottom:28px"></div>
    <div class="skeleton" style="width:100%;height:120px;margin-bottom:20px"></div>
  `;
}

// ========== Code Copy Buttons ==========
function addCodeCopyButtons(container) {
  if (!features.codeCopyButtons) return;
  container.querySelectorAll("pre").forEach((pre) => {
    pre.style.position = "relative";
    const btn = document.createElement("button");
    btn.className = "code-copy-btn";
    btn.textContent = "COPY";
    btn.setAttribute("aria-label", "Copy code");
    btn.addEventListener("click", () => {
      const code = pre.querySelector("code");
      navigator.clipboard.writeText(code ? code.textContent : pre.textContent);
      btn.textContent = "COPIED";
      btn.classList.add("copied");
      setTimeout(() => {
        btn.textContent = "COPY";
        btn.classList.remove("copied");
      }, 2000);
    });
    pre.appendChild(btn);
  });
}

// ========== Scroll Reveal ==========
function initScrollReveal() {
  if (!features.scrollReveal) return;
  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          entry.target.classList.add("revealed");
          observer.unobserve(entry.target);
        }
      });
    },
    { threshold: 0.15 }
  );

  document.querySelectorAll(".reveal").forEach((el) => observer.observe(el));
}

function applyRevealToContent(container) {
  if (!features.scrollReveal) return;
  container.querySelectorAll("h1, h2, h3, pre, blockquote, table, img").forEach((el, i) => {
    el.classList.add("reveal");
    el.style.transitionDelay = `${Math.min(i * 40, 200)}ms`;
  });
  initScrollReveal();
}

// ========== Reading Progress ==========
function initReadingProgress() {
  if (!features.readingProgress) return;
  const bar = document.getElementById("reading-progress");
  if (!bar) return;

  const content = document.getElementById("content");
  const update = () => {
    const scrollTop = window.scrollY;
    const docHeight = content.scrollHeight - window.innerHeight;
    const progress = docHeight > 0 ? Math.min(scrollTop / docHeight, 1) : 0;
    bar.style.width = `${progress * 100}%`;
    bar.style.opacity = progress > 0.01 ? "1" : "0";
  };

  window.addEventListener("scroll", update, { passive: true });
  update();
}

// ========== Table of Contents ==========
let tocObserver = null;

function renderTOC() {
  const tocNav = document.getElementById('toc-nav');
  const tocEl = document.getElementById('toc');
  const article = document.getElementById('article');

  if (!tocNav || !tocEl) return;

  // Clean up previous observer
  if (tocObserver) {
    tocObserver.disconnect();
    tocObserver = null;
  }

  const headings = article.querySelectorAll('h2, h3');

  if (headings.length === 0) {
    tocEl.style.display = 'none';
    return;
  }
  tocEl.style.display = '';

  tocNav.innerHTML = Array.from(headings).map((h) => {
    if (!h.id) {
      h.id = 'toc-' + h.textContent.trim().toLowerCase()
        .replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');
    }
    const level = h.tagName === 'H3' ? ' toc-h3' : '';
    return `<a class="toc-link${level}" data-target="${h.id}">${h.textContent}</a>`;
  }).join('');

  // Click handler — scroll into view without hash change
  tocNav.querySelectorAll('.toc-link').forEach((link) => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      const target = document.getElementById(link.dataset.target);
      if (target) {
        target.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    });
  });

  // Scroll spy via IntersectionObserver
  tocObserver = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        tocNav.querySelectorAll('.toc-link').forEach((l) => l.classList.remove('active'));
        const link = tocNav.querySelector(`[data-target="${entry.target.id}"]`);
        if (link) link.classList.add('active');
      }
    });
  }, { rootMargin: '-80px 0px -70% 0px' });

  headings.forEach((h) => tocObserver.observe(h));
}

// ========== Prev/Next Navigation ==========
function renderPrevNext(currentSlug) {
  const allPages = config.sidebar.flatMap((s) => s.children);
  const idx = allPages.findIndex((p) => p.slug === currentSlug);
  if (idx === -1) return;

  const prev = idx > 0 ? allPages[idx - 1] : null;
  const next = idx < allPages.length - 1 ? allPages[idx + 1] : null;

  if (!prev && !next) return;

  const article = document.getElementById('article');
  const nav = document.createElement('nav');
  nav.className = 'prev-next-nav';
  nav.innerHTML = `
    ${prev ? `<a href="#${prev.slug}" class="prev-next prev"><span class="prev-next-label">Previous</span><span class="prev-next-title">${prev.title}</span></a>` : '<div></div>'}
    ${next ? `<a href="#${next.slug}" class="prev-next next"><span class="prev-next-label">Next</span><span class="prev-next-title">${next.title}</span></a>` : '<div></div>'}
  `;
  article.appendChild(nav);
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

  showContentSkeleton();

  try {
    const response = await fetch(`docs/${slug}.md`);
    if (!response.ok) throw new Error("Not found");
    const md = await response.text();
    article.innerHTML = parseMarkdown(md);
    addCodeCopyButtons(article);
    applyRevealToContent(article);
    renderTOC();
    renderPrevNext(slug);
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
  initReadingProgress();

  window.addEventListener("hashchange", () => {
    loadPage(getCurrentSlug());
  });

  buildSearchIndex();
}

init();
