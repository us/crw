const STORAGE_KEY = "theme-preference";

function getPreference() {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored) return stored;
  // Default to dark mode
  return "dark";
}

function setTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem(STORAGE_KEY, theme);

  // Swap highlight.js theme
  const hljsLink = document.getElementById("hljs-theme");
  if (hljsLink) {
    const base = "https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.11.1/styles";
    hljsLink.href = theme === "dark" ? `${base}/github-dark.min.css` : `${base}/github.min.css`;
  }
}

function toggleTheme() {
  const current = document.documentElement.getAttribute("data-theme");
  setTheme(current === "dark" ? "light" : "dark");
}

// Apply theme immediately
setTheme(getPreference());

document.getElementById("theme-toggle").addEventListener("click", toggleTheme);

// Listen for OS theme changes
window
  .matchMedia("(prefers-color-scheme: dark)")
  .addEventListener("change", (e) => {
    if (!localStorage.getItem(STORAGE_KEY)) {
      setTheme(e.matches ? "dark" : "light");
    }
  });
