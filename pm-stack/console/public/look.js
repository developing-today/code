// How the app looks on this device: its colours, its typeface, the size
// of its text, the room around things, how round the corners are, and
// whether anything moves.
//
// Each choice is one attribute on the page's root element (data-theme,
// data-font, data-size, data-space, data-corners, data-motion) and the
// style sheets do the rest: themes.css has a block of colours for each
// theme, look.css a rule for each of the others. Nothing here writes a
// style, so the page's content policy stays as strict as it is.
//
// The choices are kept on the device they were made on (a phone with an
// OLED screen and a desk monitor want different things), and they are
// put on the page before it is first drawn: this file is loaded in the
// page's head, after the style sheets and before anything is shown.
//
// One file for the page and the tests alike. It is a plain script, not
// a module, so the page has it before the app's own script runs; both
// find it as PMLook.
((root) => {
  /** Every theme: its name in themes.css, what the chooser calls it,
   * the group it is shown in, and whether it is a dark or a light one. */
  const THEMES = [
    ["slate", "Slate", "standard", "dark"],
    ["paper", "Paper", "standard", "light"],
    ["black", "Black", "black", "dark"],
    ["black-amber", "Black amber", "black", "dark"],
    ["black-green", "Black terminal", "black", "dark"],
    ["black-rose", "Black rose", "black", "dark"],
    ["black-ice", "Black ice", "black", "dark"],
    ["black-violet", "Black violet", "black", "dark"],
    ["black-ember", "Black ember", "black", "dark"],
    ["black-dim", "Black, dimmed", "black", "dark"],
    ["contrast-dark", "High contrast dark", "contrast", "dark"],
    ["contrast-aqua", "High contrast aqua", "contrast", "dark"],
    ["contrast-light", "High contrast light", "contrast", "light"],
    ["nord", "Nord", "dark", "dark"],
    ["dracula", "Dracula", "dark", "dark"],
    ["solarized-dark", "Solarized dark", "dark", "dark"],
    ["gruvbox-dark", "Gruvbox dark", "dark", "dark"],
    ["catppuccin-mocha", "Catppuccin Mocha", "dark", "dark"],
    ["tokyo-night", "Tokyo Night", "dark", "dark"],
    ["rose-pine", "Rosé Pine", "dark", "dark"],
    ["everforest", "Everforest", "dark", "dark"],
    ["one-dark", "One Dark", "dark", "dark"],
    ["ocean", "Ocean", "dark", "dark"],
    ["dusk", "Dusk", "dark", "dark"],
    ["graphite", "Graphite", "dark", "dark"],
    ["blueprint", "Blueprint", "dark", "dark"],
    ["solarized-light", "Solarized light", "light", "light"],
    ["gruvbox-light", "Gruvbox light", "light", "light"],
    ["catppuccin-latte", "Catppuccin Latte", "light", "light"],
    ["rose-pine-dawn", "Rosé Pine Dawn", "light", "light"],
    ["sepia", "Sepia", "light", "light"],
    ["mint", "Mint", "light", "light"],
    ["ink", "Ink", "light", "light"],
  ];
  /** The groups, in the order the chooser shows them. */
  const THEME_GROUPS = [
    ["standard", "Standard", "What the app has always looked like."],
    ["black", "True black", "The page is pure black: an OLED screen leaves those pixels off."],
    ["contrast", "High contrast", "Text at 15 to 1 or more, and edges that can be seen."],
    ["dark", "Dark", ""],
    ["light", "Light", ""],
  ];
  /** Every typeface: its name in look.css, what the chooser calls it,
   * its kind, and what is worth knowing about it. The first three are
   * the device's own and cost nothing to fetch; the rest are files the
   * app carries (public/fonts, each with its licence) and are fetched
   * when first chosen, then kept for when there is no connection. */
  const FONTS = [
    ["system", "System", "sans", "This device's own interface typeface."],
    ["system-serif", "System serif", "serif", "This device's own serif."],
    ["system-mono", "System mono", "mono", "This device's own fixed-width typeface."],
    ["inter", "Inter", "sans", "Made for screens; tall and even."],
    ["ibm-plex-sans", "IBM Plex Sans", "sans", "Engineered, a little square."],
    ["source-sans-3", "Source Sans", "sans", "Calm and narrow: more on a line."],
    ["public-sans", "Public Sans", "sans", "Plain and strong, made for public services."],
    ["figtree", "Figtree", "sans", "Friendly and clean."],
    ["manrope", "Manrope", "sans", "Modern, open shapes."],
    ["space-grotesk", "Space Grotesk", "sans", "Geometric, with character."],
    ["nunito", "Nunito", "sans", "Rounded ends; soft."],
    ["atkinson-hyperlegible-next", "Atkinson Hyperlegible", "reading", "Letters that cannot be mistaken for each other; made for low vision."],
    ["lexend", "Lexend", "reading", "Wide and open, made to be read fast."],
    ["opendyslexic", "OpenDyslexic", "reading", "Weighted at the bottom, made for readers with dyslexia."],
    ["comic-neue", "Comic Neue", "reading", "Casual, hand-drawn shapes."],
    ["source-serif-4", "Source Serif", "serif", "A reading serif."],
    ["literata", "Literata", "serif", "A book face, made for long reading."],
    ["bitter", "Bitter", "serif", "A slab serif, sturdy on a screen."],
    ["jetbrains-mono", "JetBrains Mono", "mono", "Fixed width, made for code."],
    ["fira-code", "Fira Code", "mono", "Fixed width, with joined-up arrows."],
    ["ibm-plex-mono", "IBM Plex Mono", "mono", "Fixed width, typewriter in feel."],
    ["atkinson-hyperlegible-mono", "Atkinson Hyperlegible Mono", "mono", "Fixed width, and hard to misread."],
  ];
  const FONT_GROUPS = [
    ["sans", "Sans"],
    ["reading", "Made to be easy to read"],
    ["serif", "Serif"],
    ["mono", "Fixed width"],
  ];
  /** The steps of the other choices: the name in look.css, what the
   * chooser calls it, and the number look.css gives it (for the tests,
   * which hold the two together). */
  const SIZES = [
    ["xs", "Smallest", 0.85],
    ["s", "Smaller", 0.925],
    ["m", "Regular", 1],
    ["l", "Larger", 1.1],
    ["xl", "Large", 1.2],
    ["xxl", "Very large", 1.35],
    ["xxxl", "Largest", 1.5],
  ];
  const SPACES = [
    ["tight", "Tight", 0.7],
    ["snug", "Snug", 0.85],
    ["regular", "Regular", 1],
    ["roomy", "Roomy", 1.2],
    ["airy", "Airy", 1.45],
  ];
  const CORNERS = [
    ["square", "Square", 0],
    ["soft", "Soft", 0.5],
    ["regular", "Regular", 1],
    ["round", "Round", 1.7],
  ];
  const MOTIONS = [
    ["system", "As this device asks"],
    ["full", "Everything moves"],
    ["less", "Nothing moves"],
  ];

  /** What the app looks like until something else is chosen: it
   * follows the system between Paper by day and Slate by night, which
   * is what it did before there was a choice. */
  const DEFAULTS = Object.freeze({
    follow: true,
    theme: "slate",
    day: "paper",
    night: "slate",
    font: "system",
    size: "m",
    space: "regular",
    corners: "regular",
    motion: "system",
  });
  const KEY = "pm.look";

  const known = (list, id) => typeof id === "string" && list.some((entry) => entry[0] === id);
  const schemeOf = (id) => THEMES.find((t) => t[0] === id)?.[3];

  /** The choices as they will be used: anything missing, of the wrong
   * kind or no longer on offer (a theme since removed, a file edited by
   * hand) is the default instead, so a bad value can never leave the
   * page without colours. */
  function normal(raw) {
    const from = raw && typeof raw === "object" ? raw : {};
    const pick = (list, name) => (known(list, from[name]) ? from[name] : DEFAULTS[name]);
    return {
      follow: typeof from.follow === "boolean" ? from.follow : DEFAULTS.follow,
      theme: pick(THEMES, "theme"),
      day: pick(THEMES, "day"),
      night: pick(THEMES, "night"),
      font: pick(FONTS, "font"),
      size: pick(SIZES, "size"),
      space: pick(SPACES, "space"),
      corners: pick(CORNERS, "corners"),
      motion: pick(MOTIONS, "motion"),
    };
  }

  /** The theme to show: the one chosen, or, following the system, the
   * one chosen for the day or for the night. */
  function themeFor(settings, systemDark) {
    const s = normal(settings);
    if (!s.follow) return s.theme;
    return systemDark ? s.night : s.day;
  }

  /** The attributes the root element is given. Motion left to the
   * device has no attribute at all: the style sheet then asks the
   * device. */
  function attrs(settings, systemDark) {
    const s = normal(settings);
    return {
      "data-theme": themeFor(s, systemDark),
      "data-font": s.font,
      "data-size": s.size,
      "data-space": s.space,
      "data-corners": s.corners,
      "data-motion": s.motion === "system" ? null : s.motion,
    };
  }

  /** The choices kept on this device. A device that keeps nothing (a
   * private window, storage refused) gets the defaults every time. */
  function read(storage) {
    try {
      const text = storage?.getItem(KEY);
      return normal(text ? JSON.parse(text) : null);
    } catch {
      return normal(null);
    }
  }

  /** Keep the choices. Says whether they were kept. */
  function write(storage, settings) {
    try {
      storage.setItem(KEY, JSON.stringify(normal(settings)));
      return true;
    } catch {
      return false;
    }
  }

  /** Put the choices on a page. The bar the browser draws around the
   * page (and the phone's status bar, in the installed app) is given
   * the theme's own page colour. */
  function apply(doc, settings, systemDark, view = root) {
    const el = doc.documentElement;
    for (const [name, value] of Object.entries(attrs(settings, systemDark))) {
      if (value === null) el.removeAttribute(name);
      else el.setAttribute(name, value);
    }
    try {
      const bg = view.getComputedStyle?.(el).getPropertyValue("--bg").trim();
      const meta = doc.querySelector?.('meta[name="theme-color"]');
      if (bg && meta) meta.setAttribute("content", bg);
    } catch {
      // The bar keeps the colour it had.
    }
  }

  const storageOf = (view) => {
    try {
      return view.localStorage;
    } catch {
      return null;
    }
  };
  const darkOf = (view) => {
    try {
      return view.matchMedia("(prefers-color-scheme: dark)").matches;
    } catch {
      return true;
    }
  };

  /** Put this device's choices on its page now, and again whenever the
   * system goes from day to night or another tab changes them. */
  function start(view = root) {
    const again = () => apply(view.document, read(storageOf(view)), darkOf(view), view);
    again();
    try {
      view.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", again);
      view.addEventListener("storage", (event) => {
        if (event.key === null || event.key === KEY) again();
      });
    } catch {
      // A browser without these shows what was chosen when it opened.
    }
    return again;
  }

  /** Change the choices on this device, and show them at once. A value
   * that is not on offer changes nothing: what was chosen before stays. */
  function choose(change, view = root) {
    const storage = storageOf(view);
    const before = read(storage);
    const offered = Object.entries(change || {}).filter(([name, value]) => Object.hasOwn(DEFAULTS, name) && normal({ [name]: value })[name] === value);
    const next = normal({ ...before, ...Object.fromEntries(offered) });
    const kept = storage ? write(storage, next) : false;
    apply(view.document, next, darkOf(view), view);
    return { settings: next, kept };
  }

  root.PMLook = {
    THEMES,
    THEME_GROUPS,
    FONTS,
    FONT_GROUPS,
    SIZES,
    SPACES,
    CORNERS,
    MOTIONS,
    DEFAULTS,
    KEY,
    schemeOf,
    normal,
    themeFor,
    attrs,
    read,
    write,
    apply,
    start,
    choose,
    current: (view = root) => read(storageOf(view)),
    systemDark: (view = root) => darkOf(view),
  };
  // On a page, at once: this is what keeps the first thing drawn from
  // being the wrong colours.
  if (root.document?.documentElement && typeof root.matchMedia === "function") start(root);
})(typeof self !== "undefined" ? self : globalThis);
