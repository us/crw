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
    { label: "Docs", href: "#introduction" },
    { label: "API", href: "#rest-api" },
    { label: "MCP", href: "#mcp" },
    { label: "GitHub", href: "https://github.com/us/crw", external: true },
  ],

  navTabs: [
    { label: "Documentation", href: "#introduction" },
    { label: "API Reference", href: "#rest-api" },
    { label: "MCP Server", href: "#mcp" },
    { label: "Integrations", href: "#integrations" },
  ],

  quickLinks: [
    { title: "GitHub", href: "https://github.com/us/crw", icon: "github", external: true },
    { title: "Changelog", href: "#changelog", icon: "list" },
  ],

  sidebar: [
    {
      title: "Getting Started",
      children: [
        { title: "Introduction", slug: "introduction", icon: "rocket" },
        { title: "Installation", slug: "installation" },
        { title: "Quick Start", slug: "quick-start" },
        { title: "SDK Examples", slug: "sdk-examples" },
      ],
    },
    {
      title: "Core Endpoints",
      children: [
        { title: "Scrape", slug: "scraping", icon: "code" },
        { title: "Crawl", slug: "crawling", icon: "globe" },
        { title: "Search", slug: "search", icon: "search" },
        { title: "Map", slug: "map", icon: "map" },
        { title: "Extract", slug: "extract", icon: "zap" },
      ],
    },
    {
      title: "Features",
      children: [
        { title: "Output Formats", slug: "output-formats", icon: "layers" },
        { title: "JS Rendering", slug: "js-rendering" },
        { title: "Agent Onboarding", slug: "agent-onboarding" },
      ],
    },
    {
      title: "API & Integration",
      children: [
        { title: "REST API", slug: "rest-api", icon: "server" },
        { title: "MCP Server", slug: "mcp", icon: "plug" },
        { title: "Integrations", slug: "integrations" },
        { title: "Compatibility", slug: "compatibility" },
      ],
    },
    {
      title: "Deployment",
      children: [
        { title: "Docker", slug: "docker", icon: "box" },
        { title: "Self-Hosting", slug: "self-hosting" },
        { title: "Hardening", slug: "self-hosting-hardening" },
        { title: "Configuration", slug: "configuration", icon: "settings" },
      ],
    },
    {
      title: "Reference",
      children: [
        { title: "Rate Limits", slug: "rate-limits" },
        { title: "Error Codes", slug: "error-codes" },
        { title: "Credit Costs", slug: "credit-costs" },
        { title: "Crates", slug: "crates" },
        { title: "Architecture", slug: "architecture" },
        { title: "Changelog", slug: "changelog" },
      ],
    },
  ],

  defaultPage: "introduction",

  footer: {
    left: "Released under the AGPL-3.0 License",
    right: "CRW — Open-source Firecrawl alternative | Self-hosted web scraper in Rust",
    socials: [
      { icon: "github", href: "https://github.com/us/crw" },
    ],
  },
};
