import { searchEngine } from "./search.js";
import config from "../site.config.js";

const features = Object.assign(
  { scrollReveal: true, codeCopyButtons: true, readingProgress: true, skeletonLoading: true },
  config.features || {}
);

// ========== SVG Icons ==========
const icons = {
  github: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"/></svg>',
  list: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>',
  rocket: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="M12 15l-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/></svg>',
  code: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>',
  globe: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
  map: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="1 6 1 22 8 18 16 22 23 18 23 2 16 6 8 2 1 6"/><line x1="8" y1="2" x2="8" y2="18"/><line x1="16" y1="6" x2="16" y2="22"/></svg>',
  server: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="2" width="20" height="8" rx="2" ry="2"/><rect x="2" y="14" width="20" height="8" rx="2" ry="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg>',
  box: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/><polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/></svg>',
  plug: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 22v-5"/><path d="M9 8V2"/><path d="M15 8V2"/><path d="M18 8v5a6 6 0 0 1-6 6h0a6 6 0 0 1-6-6V8z"/></svg>',
  settings: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
  layers: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/></svg>',
  key: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>',
  play: '<svg viewBox="0 0 24 24" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3"/></svg>',
  "file-text": '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>',
  search: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>',
  zap: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
  book: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>',
  info: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>',
  alert: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>',
  check: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>',
  external: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
};

function getIcon(name) {
  return icons[name] || icons.external;
}

// ========== Render Navbar ==========
function renderNavbar() {
  const logoEl = document.querySelector(".logo");
  if (config.logo) {
    logoEl.innerHTML = `<img src="${config.logo}" alt="${config.name}" height="24" class="logo-img logo-light" /><img src="${config.logoDark || config.logo}" alt="${config.name}" height="24" class="logo-img logo-dark" /><span class="logo-text">fast<span style="color:#16A34A">crw</span></span>`;
  } else {
    logoEl.textContent = config.name;
  }

  document.title = config.description
    ? `${config.name} — ${config.description}`
    : config.name;

  // Navbar tabs (second row)
  const tabsEl = document.getElementById("navbar-tabs");
  if (tabsEl && config.navTabs) {
    tabsEl.innerHTML = config.navTabs
      .map((tab) => {
        const external = tab.external ? ' target="_blank" rel="noopener"' : "";
        return `<a href="${tab.href}" class="navbar-tab"${external}>${tab.label}</a>`;
      })
      .join("");
  }

  // GitHub link
  const githubEl = document.getElementById("navbar-github");
  const githubLink = config.navLinks?.find(l => l.href?.includes("github.com"));
  if (githubEl && githubLink) {
    githubEl.href = githubLink.href;
  }

  // Footer
  renderFooter();
}

// ========== Render Footer ==========
function renderFooter() {
  const footer = document.getElementById("footer");
  if (!footer) return;

  const socials = config.footer?.socials || [];
  const columns = config.footer?.columns || [];
  const tagline = config.footer?.tagline || "";

  const socialHTML = socials.length > 0
    ? `<div class="footer-socials">${socials.map(s =>
        `<a href="${s.href}" target="_blank" rel="noopener" class="footer-social" aria-label="${s.icon}">${getIcon(s.icon)}</a>`
      ).join('')}</div>`
    : '';

  const columnsHTML = columns.map(col => `
    <div class="footer-col">
      <div class="footer-col-title">${col.title}</div>
      <div class="footer-col-links">
        ${col.links.map(l =>
          `<a href="${l.href}"${l.external ? ' target="_blank" rel="noopener"' : ''}>${l.label}</a>`
        ).join('')}
      </div>
    </div>
  `).join('');

  footer.innerHTML = `
    <div class="footer-inner">
      <div class="footer-grid">
        <div class="footer-brand">
          <div class="footer-brand-logo">
            <img src="logo-icon.svg" alt="CRW" class="logo-img logo-light" />
            <img src="logo-dark.svg" alt="CRW" class="logo-img logo-dark" />
            <span>CRW</span>
          </div>
          <div class="footer-tagline">${tagline}</div>
          ${socialHTML}
        </div>
        ${columnsHTML}
      </div>
      <div class="footer-bottom">
        <div class="footer-copyright">&copy; ${new Date().getFullYear()} CRW. All rights reserved.</div>
        <div class="footer-status"><span class="footer-status-dot"></span> All systems operational</div>
      </div>
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
    for (const [prop, val] of Object.entries(config.theme.light)) css += `  ${prop}: ${val};\n`;
    css += "}\n";
  }
  if (config.theme.dark) {
    css += '[data-theme="dark"] {\n';
    for (const [prop, val] of Object.entries(config.theme.dark)) css += `  ${prop}: ${val};\n`;
    css += "}\n";
  }
  style.textContent = css;
  document.head.appendChild(style);
}

// ========== Component Parser ==========
function parseComponents(md) {
  // Cards: :::cards ... :::
  md = md.replace(/:::cards\n([\s\S]*?):::/g, (_, content) => {
    const cards = [];
    content.replace(/::card\{([^}]*)\}/g, (__, attrs) => {
      const props = {};
      attrs.replace(/(\w+)="([^"]*)"/g, (___, k, v) => { props[k] = v; });
      cards.push(props);
    });
    return `<div class="card-grid">${cards.map(c =>
      `<a href="${c.href || '#'}" class="doc-card"${c.href?.startsWith('http') ? ' target="_blank" rel="noopener"' : ''}>
        ${c.icon ? `<div class="doc-card-icon">${getIcon(c.icon)}</div>` : ''}
        <div class="doc-card-title">${c.title || ''}</div>
        <div class="doc-card-desc">${c.description || ''}</div>
      </a>`
    ).join('')}</div>`;
  });

  // Features: :::features ... :::
  md = md.replace(/:::features\n([\s\S]*?):::/g, (_, content) => {
    const items = [];
    content.replace(/::feature\{([^}]*)\}/g, (__, attrs) => {
      const props = {};
      attrs.replace(/(\w+)="([^"]*)"/g, (___, k, v) => { props[k] = v; });
      items.push(props);
    });
    return `<div class="feature-grid">${items.map(f =>
      `<div class="feature-card">
        ${f.icon ? `<div class="feature-card-icon">${getIcon(f.icon)}</div>` : ''}
        <div class="feature-card-title">${f.title || ''}</div>
        <div class="feature-card-desc">${f.description || ''}</div>
      </div>`
    ).join('')}</div>`;
  });

  // Callouts: :::note/warning/tip ... :::
  md = md.replace(/:::(note|warning|tip|info)\n([\s\S]*?):::/g, (_, type, content) => {
    const iconName = type === 'warning' ? 'alert' : type === 'tip' ? 'check' : 'info';
    return `<div class="callout callout-${type}"><div class="callout-icon">${getIcon(iconName)}</div><div class="callout-content">${content.trim()}</div></div>`;
  });

  // Collapsible: :::details{title="..."} ... :::
  md = md.replace(/:::details\{title="([^"]*)"\}\n([\s\S]*?):::/g, (_, title, content) => {
    return `<div class="details-block"><div class="details-summary" onclick="this.parentElement.classList.toggle('open')">${title}<span class="details-chevron">&#9654;</span></div><div class="details-content">${content.trim()}</div></div>`;
  });

  // Code tabs: :::tabs ... :::
  md = md.replace(/:::tabs\n([\s\S]*?):::/g, (_, content) => {
    const tabs = [];
    const tabRegex = /::tab\{title="([^"]*)"\}\n([\s\S]*?)(?=::tab\{|$)/g;
    let match;
    while ((match = tabRegex.exec(content)) !== null) {
      tabs.push({ title: match[1], content: match[2].trim() });
    }
    const id = 'tabs-' + Math.random().toString(36).slice(2, 8);
    return `<div class="code-tabs" data-tabs-id="${id}">
      <div class="code-tabs-header">${tabs.map((t, i) =>
        `<button class="code-tab${i === 0 ? ' active' : ''}" data-tab="${i}">${t.title}</button>`
      ).join('')}</div>
      ${tabs.map((t, i) =>
        `<div class="code-tab-panel${i === 0 ? ' active' : ''}" data-panel="${i}">${t.content}</div>`
      ).join('')}
    </div>`;
  });

  return md;
}

// ========== Minimal Markdown Parser ==========
function parseMarkdown(md) {
  if (/^\s*</.test(md)) return md;

  // Parse components FIRST (before code block extraction)
  let html = parseComponents(md);

  const codeBlocks = [];
  const inlineCodes = [];

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

  html = html.replace(/`([^`]+)`/g, (_, code) => {
    const placeholder = `\x00INLINECODE${inlineCodes.length}\x00`;
    inlineCodes.push(`<code>${code}</code>`);
    return placeholder;
  });

  html = html.replace(/^#### (.+)$/gm, "<h4>$1</h4>");
  html = html.replace(/^### (.+)$/gm, "<h3>$1</h3>");
  html = html.replace(/^## (.+)$/gm, "<h2>$1</h2>");
  html = html.replace(/^# (.+)$/gm, "<h1>$1</h1>");
  html = html.replace(/^---$/gm, "<hr>");
  html = html.replace(/\*\*\*(.+?)\*\*\*/g, "<strong><em>$1</em></strong>");
  html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");
  html = html.replace(/!\[([^\]]*)\]\(([^)]+)\)/g, '<img src="$2" alt="$1" loading="lazy">');
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>');
  html = html.replace(/^&gt; (.+)$/gm, "<blockquote><p>$1</p></blockquote>");
  html = html.replace(/^(\s*)[-*] (.+)$/gm, "$1<li>$2</li>");
  html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, "<ul>$1</ul>");
  html = html.replace(/^\d+\. (.+)$/gm, "<li>$1</li>");

  html = html.replace(
    /^\|(.+)\|\s*\n\|[-| :]+\|\s*\n((?:\|.+\|\s*\n?)*)/gm,
    (_, header, body) => {
      const headers = header.split("|").map((h) => h.trim()).filter(Boolean);
      const rows = body.trim().split("\n").map((row) => row.split("|").map((c) => c.trim()).filter(Boolean));
      let table = "<table><thead><tr>";
      headers.forEach((h) => (table += `<th>${h}</th>`));
      table += "</tr></thead><tbody>";
      rows.forEach((row) => { table += "<tr>"; row.forEach((cell) => (table += `<td>${cell}</td>`)); table += "</tr>"; });
      table += "</tbody></table>";
      return table;
    }
  );

  html = html.split("\n\n").map((block) => {
    const trimmed = block.trim();
    if (!trimmed) return "";
    if (/^</.test(trimmed)) return trimmed;
    if (/^\x00CODEBLOCK/.test(trimmed)) return trimmed;
    return `<p>${trimmed.replace(/\n/g, "<br>")}</p>`;
  }).join("\n");

  codeBlocks.forEach((block, i) => { html = html.replace(`\x00CODEBLOCK${i}\x00`, block); });
  inlineCodes.forEach((code, i) => { html = html.replace(`\x00INLINECODE${i}\x00`, code); });

  return html;
}

function stripMarkdown(md) {
  return md.replace(/```[\s\S]*?```/g, "").replace(/`[^`]+`/g, "").replace(/:::[^:]*:::/g, "").replace(/::[\w]+\{[^}]*\}/g, "").replace(/[#*_\[\]()>|`-]/g, "").replace(/\n+/g, " ").trim();
}

// ========== Sidebar Rendering ==========
function renderSidebar() {
  const nav = document.getElementById("sidebar-nav");
  const currentSlug = getCurrentSlug();

  // Quick links
  const quickLinksHTML = (config.quickLinks || []).length > 0
    ? `<div class="sidebar-quick-links">${config.quickLinks.map((link) => {
        const icon = getIcon(link.icon);
        const ext = link.external ? ' target="_blank" rel="noopener"' : '';
        return `<a href="${link.href}" class="sidebar-quick-link"${ext}><span class="sidebar-quick-icon">${icon}</span>${link.title}</a>`;
      }).join('')}</div>`
    : '';

  nav.innerHTML = quickLinksHTML + config.sidebar
    .map((section) => {
      const hasActiveChild = section.children.some((c) => c.slug === currentSlug);

      // Sections are always open (flat, Mintlify-style)
      const childrenHTML = section.children.map((child) => {
        const iconHTML = child.icon ? `<span class="sidebar-item-icon">${getIcon(child.icon)}</span>` : '';
        return `<a href="#${child.slug}" class="sidebar-link ${child.slug === currentSlug ? "active" : ""}">${iconHTML}${child.title}</a>`;
      }).join("");

      return `
        <div class="sidebar-section${hasActiveChild ? ' has-active' : ''}">
          <div class="sidebar-section-title">${section.title}</div>
          <div class="sidebar-group-children open">
            ${childrenHTML}
          </div>
        </div>
      `;
    })
    .join("");

  // Close sidebar on mobile
  nav.querySelectorAll(".sidebar-link").forEach((link) => {
    link.addEventListener("click", () => {
      if (window.innerWidth <= 768) closeSidebar();
    });
  });
}

// ========== Init Code Tabs ==========
function initCodeTabs(container) {
  container.querySelectorAll(".code-tabs").forEach((tabsEl) => {
    const buttons = tabsEl.querySelectorAll(".code-tab");
    const panels = tabsEl.querySelectorAll(".code-tab-panel");

    buttons.forEach((btn) => {
      btn.addEventListener("click", () => {
        const idx = btn.dataset.tab;
        buttons.forEach((b) => b.classList.remove("active"));
        panels.forEach((p) => p.classList.remove("active"));
        btn.classList.add("active");
        tabsEl.querySelector(`[data-panel="${idx}"]`)?.classList.add("active");
      });
    });
  });
}

// ========== Skeleton Loading ==========
function showContentSkeleton() {
  if (!features.skeletonLoading) return;
  const article = document.getElementById("article");
  article.innerHTML = `
    <div class="skeleton" style="width:30%;height:14px;margin-bottom:8px"></div>
    <div class="skeleton" style="width:45%;height:32px;margin-bottom:12px"></div>
    <div class="skeleton" style="width:80%;height:14px;margin-bottom:28px"></div>
    <div class="skeleton" style="width:100%;height:14px;margin-bottom:10px"></div>
    <div class="skeleton" style="width:92%;height:14px;margin-bottom:10px"></div>
    <div class="skeleton" style="width:78%;height:14px;margin-bottom:28px"></div>
    <div class="skeleton" style="width:100%;height:120px;margin-bottom:20px"></div>
  `;
}

// ========== Code Copy Buttons ==========
function addCodeCopyButtons(container) {
  if (!features.codeCopyButtons) return;
  container.querySelectorAll("pre").forEach((pre) => {
    if (pre.querySelector(".code-copy-btn")) return;
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
      setTimeout(() => { btn.textContent = "COPY"; btn.classList.remove("copied"); }, 2000);
    });
    pre.appendChild(btn);
  });
}

// ========== Scroll Reveal ==========
function initScrollReveal() {
  if (!features.scrollReveal) return;
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) { entry.target.classList.add("revealed"); observer.unobserve(entry.target); }
    });
  }, { threshold: 0.15 });
  document.querySelectorAll(".reveal").forEach((el) => observer.observe(el));
}

function applyRevealToContent(container) {
  if (!features.scrollReveal) return;
  container.querySelectorAll("h1, h2, h3, pre, blockquote, table, img, .card-grid, .feature-grid, .callout").forEach((el, i) => {
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

  if (tocObserver) { tocObserver.disconnect(); tocObserver = null; }

  const headings = article.querySelectorAll('h2, h3');
  if (headings.length === 0) { tocEl.style.display = 'none'; return; }
  tocEl.style.display = '';

  tocNav.innerHTML = Array.from(headings).map((h) => {
    if (!h.id) {
      h.id = 'toc-' + h.textContent.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');
    }
    const level = h.tagName === 'H3' ? ' toc-h3' : '';
    return `<a class="toc-link${level}" href="#${h.id}" data-target="${h.id}">${h.textContent}</a>`;
  }).join('');

  tocNav.querySelectorAll('.toc-link').forEach((link) => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      const target = document.getElementById(link.dataset.target);
      if (target) target.scrollIntoView({ behavior: 'smooth', block: 'start' });
    });
  });

  tocObserver = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        tocNav.querySelectorAll('.toc-link').forEach((l) => l.classList.remove('active'));
        const link = tocNav.querySelector(`[data-target="${entry.target.id}"]`);
        if (link) link.classList.add('active');
      }
    });
  }, { rootMargin: '-96px 0px -70% 0px' });

  headings.forEach((h) => tocObserver.observe(h));
}

// ========== Prev/Next Navigation ==========
function renderPrevNext(currentSlug) {
  const allPages = config.sidebar.flatMap((s) => s.children);
  const idx = allPages.findIndex((p) => p.slug === currentSlug);
  if (idx === -1) return;

  const next = idx < allPages.length - 1 ? allPages[idx + 1] : null;
  if (!next) return;

  const article = document.getElementById('article');
  const nav = document.createElement('nav');
  nav.className = 'prev-next-nav';
  nav.innerHTML = `
    <div></div>
    <a href="#${next.slug}" class="prev-next next">
      <span class="prev-next-title">${next.title}</span>
      <span class="prev-next-label">Next &rsaquo;</span>
    </a>
  `;
  article.appendChild(nav);
}

// ========== Breadcrumb ==========
function renderBreadcrumb(slug) {
  const section = config.sidebar.find(s => s.children?.some(c => c.slug === slug));
  if (!section) return '';
  return `<div class="page-breadcrumb">${section.title}</div>`;
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
    const breadcrumb = renderBreadcrumb(slug);
    article.innerHTML = breadcrumb + parseMarkdown(md);
    addCodeCopyButtons(article);
    initCodeTabs(article);
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
  updateActiveTab(slug);
  window.scrollTo(0, 0);
}

// ========== Active Tab ==========
function updateActiveTab(slug) {
  document.querySelectorAll('.navbar-tab').forEach((tab) => {
    tab.classList.remove('active');
    const tabSlug = tab.getAttribute('href')?.replace('#', '');
    if (tabSlug === slug) {
      tab.classList.add('active');
    }
  });
  // Default: first tab active if no match
  const tabs = document.querySelectorAll('.navbar-tab');
  if (tabs.length > 0 && !document.querySelector('.navbar-tab.active')) {
    tabs[0].classList.add('active');
  }
}

// ========== Mobile Sidebar ==========
const hamburger = document.getElementById("hamburger");
const sidebar = document.getElementById("sidebar");
const overlay = document.getElementById("overlay");

function openSidebar() { sidebar.classList.add("open"); overlay.classList.add("active"); hamburger.classList.add("active"); }
function closeSidebar() { sidebar.classList.remove("open"); overlay.classList.remove("active"); hamburger.classList.remove("active"); }
hamburger.addEventListener("click", () => { sidebar.classList.contains("open") ? closeSidebar() : openSidebar(); });
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
        pages.push({ title: child.title, slug: child.slug, content: stripMarkdown(md) });
      } catch { /* skip */ }
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
  window.addEventListener("hashchange", () => { loadPage(getCurrentSlug()); });
  // Defer search index build to after page is interactive
  if ('requestIdleCallback' in window) {
    requestIdleCallback(() => buildSearchIndex());
  } else {
    setTimeout(buildSearchIndex, 2000);
  }
}

init();
