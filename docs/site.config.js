export default {
  name: "crw",
  description: "Lightweight Firecrawl alternative — web scraper & crawler + MCP server",

  navLinks: [
    { label: "Docs", href: "#introduction" },
    { label: "API", href: "#rest-api" },
    { label: "MCP", href: "#mcp" },
    { label: "GitHub", href: "https://github.com/us/crw", external: true },
  ],

  sidebar: [
    {
      title: "Getting Started",
      children: [
        { title: "Introduction", slug: "introduction" },
        { title: "Installation", slug: "installation" },
        { title: "Quick Start", slug: "quick-start" },
      ],
    },
    {
      title: "Features",
      children: [
        { title: "Scraping", slug: "scraping" },
        { title: "Crawling", slug: "crawling" },
        { title: "Output Formats", slug: "output-formats" },
        { title: "JS Rendering", slug: "js-rendering" },
      ],
    },
    {
      title: "API & Integration",
      children: [
        { title: "REST API", slug: "rest-api" },
        { title: "MCP Server", slug: "mcp" },
      ],
    },
    {
      title: "Deployment",
      children: [
        { title: "Docker", slug: "docker" },
        { title: "Configuration", slug: "configuration" },
      ],
    },
    {
      title: "Reference",
      children: [
        { title: "Crates", slug: "crates" },
        { title: "Architecture", slug: "architecture" },
      ],
    },
  ],

  defaultPage: "introduction",

  footer: {
    left: "Released under the AGPL-3.0 License",
    right: "crw — Lightweight Firecrawl alternative in Rust",
  },
};
