/**
 * Theme management for the id web interface.
 * All themes use #000 black background with different accent colors:
 * - sneak: blue (default)
 * - arch: green
 * - mech: orange
 */

export type Theme = "sneak" | "arch" | "mech";

const THEME_STORAGE_KEY = "id-theme";

/**
 * Get the currently active theme from localStorage.
 */
export function getTheme(): Theme {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  if (stored && isValidTheme(stored)) {
    return stored;
  }
  return "sneak";
}

/**
 * Get the current theme from the DOM (what this tab is actually showing).
 */
export function getCurrentTabTheme(): Theme {
  const domTheme = document.documentElement.getAttribute("data-theme");
  if (domTheme && isValidTheme(domTheme)) {
    return domTheme;
  }
  return getTheme();
}

/**
 * Set the active theme.
 */
export function setTheme(theme: Theme): void {
  if (!isValidTheme(theme)) {
    console.error("Invalid theme:", theme);
    return;
  }

  // Update localStorage
  localStorage.setItem(THEME_STORAGE_KEY, theme);

  // Update document attribute
  document.documentElement.setAttribute("data-theme", theme);

  // Dispatch event for any listeners
  const event = new CustomEvent("theme:change", { detail: { theme } });
  document.dispatchEvent(event);

  console.log("[theme] Switched to", theme);
}

/**
 * Initialize theme system - apply stored theme on load.
 */
export function initTheme(): void {
  const theme = getTheme();
  document.documentElement.setAttribute("data-theme", theme);

  // Add keyboard shortcut for theme cycling (Alt+T)
  document.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.altKey && event.key === "t") {
      event.preventDefault();
      cycleTheme();
    }
  });

  console.log("[theme] Initialized with", theme);
}

/**
 * Cycle through available themes based on what this tab is currently showing.
 */
export function cycleTheme(): void {
  const themes: Theme[] = ["sneak", "arch", "mech"];
  const current = getCurrentTabTheme();
  const currentIndex = themes.indexOf(current);
  const nextIndex = (currentIndex + 1) % themes.length;
  setTheme(themes[nextIndex]);
}

/**
 * Check if a string is a valid theme name.
 */
function isValidTheme(theme: string): theme is Theme {
  return ["sneak", "arch", "mech"].includes(theme);
}

/**
 * Get CSS custom properties for a theme.
 * These are defined in CSS but this function documents them for reference.
 * All themes use #000 black background with different accent colors.
 */
export function getThemeColors(theme: Theme): Record<string, string> {
  switch (theme) {
    case "arch":
      return {
        "--bg-primary": "#000000",
        "--bg-secondary": "#0a0a0a",
        "--text-primary": "#00ff41",
        "--text-secondary": "#00cc33",
        "--accent": "#00ff41",
        "--accent-dim": "#003b00",
        "--border": "#003b00",
        "--glow-color": "rgba(0, 255, 65, 0.3)",
      };
    case "mech":
      return {
        "--bg-primary": "#000000",
        "--bg-secondary": "#0a0a0a",
        "--text-primary": "#ff6600",
        "--text-secondary": "#cc5200",
        "--accent": "#ff6600",
        "--accent-dim": "#4d2600",
        "--border": "#4d2600",
        "--glow-color": "rgba(255, 102, 0, 0.3)",
      };
    default:
      return {
        "--bg-primary": "#000000",
        "--bg-secondary": "#0a0a0a",
        "--text-primary": "#4a9eff",
        "--text-secondary": "#3a7ecc",
        "--accent": "#4a9eff",
        "--accent-dim": "#1a3a66",
        "--border": "#1a3a66",
        "--glow-color": "rgba(74, 158, 255, 0.3)",
      };
  }
}
