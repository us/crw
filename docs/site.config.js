export default {
  name: "CRW",
  description: "Open-source Firecrawl alternative — self-hosted web scraper & crawler in Rust with MCP server for AI agents",
  logo: "logo-icon.svg",
  logoDark: "logo-dark.svg",
  author: {
    name: "us",
    url: "https://usxperiments.com",
  },

  navLinks: [
    { label: "Docs", href: "/introduction" },
    { label: "API", href: "/rest-api" },
    { label: "MCP", href: "/mcp" },
    { label: "GitHub", href: "https://github.com/us/crw", external: true },
  ],

  navTabs: [
    {
      label: "Start",
      href: "/quick-start",
      match: ["introduction", "quick-start", "authentication", "playground", "rest-api", "installation"],
    },
    {
      label: "Endpoints",
      href: "/scraping",
      match: ["scraping", "search", "map", "crawling", "extract"],
    },
    {
      label: "MCP",
      href: "/mcp",
      match: ["mcp", "mcp-clients", "sdk-examples", "integrations", "agent-onboarding"],
    },
    {
      label: "Self-Host",
      href: "/self-hosting",
      match: ["self-hosting", "docker", "configuration", "self-hosting-hardening", "js-rendering"],
    },
    {
      label: "Reference",
      href: "/response-shapes",
      match: ["response-shapes", "output-formats", "rate-limits", "error-codes", "credit-costs", "compatibility", "changelog", "architecture", "crates"],
    },
  ],

  quickLinks: [
    { title: "Playground", href: "https://fastcrw.com/playground", icon: "play", external: true },
    { title: "Get API Key", href: "https://fastcrw.com/register", icon: "key", external: true },
    { title: "GitHub", href: "https://github.com/us/crw", icon: "github", external: true },
    { title: "Changelog", href: "/changelog", icon: "list" },
  ],

  sidebar: [
    {
      title: "Get Started",
      children: [
        { title: "Introduction", slug: "introduction", icon: "rocket" },
        { title: "Quick Start", slug: "quick-start", icon: "play" },
        { title: "Authentication", slug: "authentication", icon: "key" },
        { title: "API Playground", slug: "playground", icon: "play" },
        { title: "API Overview", slug: "rest-api", icon: "server" },
        { title: "Installation", slug: "installation", icon: "box" },
      ],
    },
    {
      title: "Core Endpoints",
      children: [
        { title: "Scrape", slug: "scraping", icon: "code" },
        { title: "Search", slug: "search", icon: "search" },
      ],
    },
    {
      title: "More APIs",
      children: [
        { title: "Map", slug: "map", icon: "map" },
        { title: "Crawl", slug: "crawling", icon: "globe" },
        { title: "Extract", slug: "extract", icon: "zap" },
      ],
    },
    {
      title: "Integrations",
      children: [
        { title: "MCP Server", slug: "mcp", icon: "plug" },
        { title: "MCP Client Setup", slug: "mcp-clients", icon: "settings" },
        { title: "SDK Examples", slug: "sdk-examples", icon: "code" },
        { title: "Framework Integrations", slug: "integrations", icon: "layers" },
        { title: "Agent Onboarding", slug: "agent-onboarding", icon: "book" },
      ],
    },
    {
      title: "Deploy",
      children: [
        { title: "Self-Hosting", slug: "self-hosting", icon: "server" },
        { title: "Docker", slug: "docker", icon: "box" },
        { title: "Configuration", slug: "configuration", icon: "settings" },
        { title: "Hardening", slug: "self-hosting-hardening", icon: "alert" },
        { title: "JS Rendering", slug: "js-rendering", icon: "zap" },
      ],
    },
    {
      title: "Reference",
      children: [
        { title: "Response Shapes", slug: "response-shapes", icon: "layers" },
        { title: "Output Formats", slug: "output-formats", icon: "file-text" },
        { title: "Rate Limits", slug: "rate-limits", icon: "alert" },
        { title: "Error Codes", slug: "error-codes", icon: "info" },
        { title: "Credit Costs", slug: "credit-costs", icon: "list" },
        { title: "Compatibility", slug: "compatibility", icon: "check" },
        { title: "Changelog", slug: "changelog", icon: "list" },
        { title: "Architecture", slug: "architecture", icon: "layers" },
        { title: "Crates", slug: "crates", icon: "box" },
      ],
    },
  ],

  defaultPage: "introduction",

  footer: {
    tagline: "The base layer for agentic web intelligence.",
    columns: [
      { title: "Product", links: [
        { label: "Quick Start", href: "/quick-start" },
        { label: "REST API", href: "/rest-api" },
        { label: "MCP Server", href: "/mcp" },
        { label: "Changelog", href: "/changelog" },
      ]},
      { title: "Community", links: [
        { label: "GitHub", href: "https://github.com/us/crw", external: true },
        { label: "Issues", href: "https://github.com/us/crw/issues", external: true },
      ]},
      { title: "Legal", links: [
        { label: "License (AGPL-3.0)", href: "https://github.com/us/crw/blob/main/LICENSE", external: true },
      ]},
    ],
    socials: [
      { icon: "github", href: "https://github.com/us/crw" },
    ],
  },
};
