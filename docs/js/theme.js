const STORAGE_KEY = "theme-preference";

function getPreference() {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored) return stored;
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

function setTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem(STORAGE_KEY, theme);
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
