class SearchEngine {
  constructor() {
    this.index = [];
    this.modal = document.getElementById("search-modal");
    this.input = document.getElementById("search-input");
    this.results = document.getElementById("search-results");
    this.trigger = document.getElementById("search-trigger");
    this.selectedIndex = -1;

    this.bindEvents();
  }

  bindEvents() {
    this.trigger.addEventListener("click", () => this.open());

    document.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "k") {
        e.preventDefault();
        this.open();
      }
      if (e.key === "Escape" && this.modal.classList.contains("active")) {
        this.close();
      }
    });

    this.modal.addEventListener("click", (e) => {
      if (e.target === this.modal) this.close();
    });

    this.input.addEventListener("input", () => {
      this.selectedIndex = -1;
      this.search(this.input.value);
    });

    this.input.addEventListener("keydown", (e) => {
      const items = this.results.querySelectorAll(".search-result-item");
      if (e.key === "ArrowDown") {
        e.preventDefault();
        this.selectedIndex = Math.min(
          this.selectedIndex + 1,
          items.length - 1
        );
        this.updateSelection(items);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        this.selectedIndex = Math.max(this.selectedIndex - 1, 0);
        this.updateSelection(items);
      } else if (e.key === "Enter" && this.selectedIndex >= 0) {
        e.preventDefault();
        items[this.selectedIndex]?.click();
      }
    });
  }

  updateSelection(items) {
    items.forEach((item, i) => {
      item.classList.toggle("selected", i === this.selectedIndex);
    });
    items[this.selectedIndex]?.scrollIntoView({ block: "nearest" });
  }

  open() {
    this.modal.classList.add("active");
    this.input.value = "";
    this.results.innerHTML = "";
    this.selectedIndex = -1;
    setTimeout(() => this.input.focus(), 50);
  }

  close() {
    this.modal.classList.remove("active");
    this.input.value = "";
    this.results.innerHTML = "";
  }

  buildIndex(pages) {
    this.index = pages.map((page) => ({
      title: page.title,
      slug: page.slug,
      content: page.content.toLowerCase(),
      rawContent: page.content,
    }));
  }

  search(query) {
    if (!query.trim()) {
      this.results.innerHTML = "";
      return;
    }

    const terms = query.toLowerCase().trim().split(/\s+/);
    const matches = [];

    for (const page of this.index) {
      const allMatch = terms.every((term) => page.content.includes(term));
      if (!allMatch) continue;

      const snippet = this.getSnippet(page.rawContent, terms[0]);
      matches.push({
        title: page.title,
        slug: page.slug,
        snippet,
      });
    }

    this.renderResults(matches, terms);
  }

  getSnippet(content, term) {
    const lower = content.toLowerCase();
    const idx = lower.indexOf(term.toLowerCase());
    if (idx === -1) return content.slice(0, 120) + "...";

    const start = Math.max(0, idx - 50);
    const end = Math.min(content.length, idx + 80);
    let snippet = content.slice(start, end);

    if (start > 0) snippet = "..." + snippet;
    if (end < content.length) snippet += "...";

    return snippet;
  }

  highlightTerms(text, terms) {
    let result = text;
    for (const term of terms) {
      const regex = new RegExp(
        `(${term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`,
        "gi"
      );
      result = result.replace(regex, "<mark>$1</mark>");
    }
    return result;
  }

  renderResults(matches, terms) {
    if (matches.length === 0) {
      this.results.innerHTML =
        '<div class="search-empty">No results found.</div>';
      return;
    }

    this.results.innerHTML = matches
      .map(
        (match) => `
      <div class="search-result-item" data-slug="${match.slug}">
        <div class="search-result-title">${this.highlightTerms(match.title, terms)}</div>
        <div class="search-result-snippet">${this.highlightTerms(match.snippet, terms)}</div>
      </div>
    `
      )
      .join("");

    this.results.querySelectorAll(".search-result-item").forEach((item) => {
      item.addEventListener("click", () => {
        window.location.hash = item.dataset.slug;
        this.close();
      });
    });
  }
}

const searchEngine = new SearchEngine();
export { searchEngine };
