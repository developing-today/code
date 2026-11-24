// How the app looks: its themes, typefaces, text size, spacing, corners
// and movement, and the choices of them kept on a device.
//
// Oracles:
//   - whether a theme can be read is the WCAG 2 contrast formula,
//     written out here and checked on two pairs everybody knows, held
//     against the rules themes.css states for itself;
//   - what a true-black theme is, is the colour #000000 and nothing near it;
//   - that the chooser and the style sheets agree is each file's own
//     text, read here (a name in one and not the other is a choice that
//     does nothing, or a look nobody can choose);
//   - a typeface file is what its first four bytes say it is ("wOF2"),
//     and its licence is the text of the licence;
//   - what the choices do to a page is a stand-in page that records the
//     attributes it is given, with the expected attributes written out
//     by hand for each case.
// That a real browser draws each theme, typeface and step as chosen was
// looked at in Chromium on the laptop (see the design record).
import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import "../public/look.js";
import { lookCounts, lookSaid, themeChange } from "../public/app.js";

type Row = [string, string, ...unknown[]];
interface Settings {
  follow: boolean;
  theme: string;
  day: string;
  night: string;
  font: string;
  size: string;
  space: string;
  corners: string;
  motion: string;
}
const L = (globalThis as unknown as { PMLook: Record<string, unknown> }).PMLook as {
  THEMES: [string, string, string, string][];
  THEME_GROUPS: [string, string, string][];
  FONTS: [string, string, string, string][];
  FONT_GROUPS: [string, string][];
  SIZES: [string, string, number][];
  SPACES: [string, string, number][];
  CORNERS: [string, string, number][];
  MOTIONS: [string, string][];
  DEFAULTS: Settings;
  KEY: string;
  schemeOf(id: string): string | undefined;
  normal(raw: unknown): Settings;
  themeFor(s: Partial<Settings>, dark: boolean): string;
  attrs(s: Partial<Settings>, dark: boolean): Record<string, string | null>;
  read(storage: unknown): Settings;
  write(storage: unknown, s: Partial<Settings>): boolean;
  apply(doc: unknown, s: Partial<Settings>, dark: boolean, view?: unknown): void;
  start(view: unknown): () => void;
  choose(change: Partial<Settings>, view: unknown): { settings: Settings; kept: boolean };
};

const PUBLIC = join(import.meta.dir, "../public");
const text = (name: string) => readFileSync(join(PUBLIC, name), "utf8");
const themesCss = text("themes.css");
const lookCss = text("look.css");
const appCss = text("app.css");

const luminance = (hex: string) => {
  const channel = (i: number) => {
    const c = Number.parseInt(hex.slice(1 + 2 * i, 3 + 2 * i), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2);
};
const contrast = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** Every rule of a style sheet whose selector is one attribute with one
 * value: the value, and what the rule declares. */
function rulesFor(css: string, attribute: string): Map<string, Record<string, string>> {
  const out = new Map<string, Record<string, string>>();
  for (const m of css.matchAll(new RegExp(`^\\[${attribute}="([^"]+)"\\] \\{([^}]*)\\}`, "gm"))) {
    const decls = Object.fromEntries(
      [...m[2].matchAll(/^\s*([a-z0-9-]+):\s*([^;]+);/gm)].map((d) => [d[1], d[2].trim()]),
    );
    expect([m[1], out.has(m[1])]).toEqual([m[1], false]); // named once
    out.set(m[1], decls);
  }
  return out;
}

const COLOURS = [
  "--bg",
  "--panel",
  "--panel-2",
  "--line",
  "--text",
  "--quiet",
  "--accent",
  "--accent-ink",
  "--good",
  "--good-ink",
  "--warn",
  "--bad",
  "--bad-ink",
];
const themes = rulesFor(themesCss, "data-theme");

test("the contrast formula is the one everybody uses", () => {
  expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
  expect(contrast("#777777", "#ffffff")).toBeCloseTo(4.48, 2);
});

test("the chooser offers exactly the themes the style sheet has, each whole", () => {
  expect([...themes.keys()]).toEqual(L.THEMES.map((t) => t[0]));
  for (const [id, name, group, scheme] of L.THEMES) {
    const block = themes.get(id) as Record<string, string>;
    // The thirteen colours, each a six-digit colour, and whether the
    // browser's own controls are to be dark or light.
    for (const colour of COLOURS)
      expect([id, colour, /^#[0-9a-f]{6}$/.test(block[colour] ?? "")]).toEqual([id, colour, true]);
    expect([id, block["color-scheme"]]).toEqual([id, scheme]);
    const extra = Object.keys(block).filter((k) => !COLOURS.includes(k) && k !== "color-scheme" && k !== "--edge");
    expect([id, extra]).toEqual([id, []]);
    expect([id, name.length > 0, L.THEME_GROUPS.some((g) => g[0] === group)]).toEqual([id, true, true]);
    // A dark theme is dark and a light one light, by the page's own colour.
    expect([id, luminance(block["--bg"]) < 0.2 ? "dark" : "light"]).toEqual([id, scheme]);
  }
  expect(new Set(L.THEMES.map((t) => t[1])).size).toBe(L.THEMES.length); // no two called the same
  // A wide variety, of every kind asked for.
  const count = (group: string) => L.THEMES.filter((t) => t[2] === group).length;
  expect(L.THEMES.length).toBeGreaterThanOrEqual(30);
  expect(count("black")).toBeGreaterThanOrEqual(5);
  expect(count("contrast")).toBeGreaterThanOrEqual(2);
  expect(count("light")).toBeGreaterThanOrEqual(5);
  expect(count("dark")).toBeGreaterThanOrEqual(10);
});

test("every theme can be read: text, small print, the status colours, and the ink on each", () => {
  for (const [id, , group] of L.THEMES) {
    const c = themes.get(id) as Record<string, string>;
    const high = group === "contrast";
    const surfaces = ["--bg", "--panel", "--panel-2"];
    const rules: [string, string[], number][] = [
      ["--text", surfaces, high ? 15 : 7],
      ["--quiet", surfaces, high ? 10 : 4.5],
      ["--accent", surfaces, high ? 7 : 4.5],
      ["--good", surfaces, high ? 7 : 4.5],
      ["--warn", surfaces, high ? 7 : 4.5],
      ["--bad", surfaces, high ? 7 : 4.5],
      ["--accent-ink", ["--accent"], high ? 7 : 4.5],
      ["--good-ink", ["--good"], high ? 7 : 4.5],
      ["--bad-ink", ["--bad"], high ? 7 : 4.5],
      // An edge has to be told from what it is drawn on.
      ["--line", ["--bg", "--panel"], high ? 3 : 1.25],
    ];
    for (const [ink, ons, least] of rules) {
      for (const on of ons) {
        const ratio = contrast(c[ink], c[on]);
        expect(`${id} ${ink} on ${on}: ${ratio >= least ? "readable" : `${ratio.toFixed(2)} of ${least}`}`).toBe(
          `${id} ${ink} on ${on}: readable`,
        );
      }
    }
    // A high-contrast theme's edges are heavier too.
    expect([id, c["--edge"] ?? "the app's own"]).toEqual([id, high ? "2px" : "the app's own"]);
  }
});

test("a true-black theme is black: the page and its panels are #000000", () => {
  const black = L.THEMES.filter((t) => t[2] === "black");
  for (const [id] of black) {
    const c = themes.get(id) as Record<string, string>;
    expect([id, c["--bg"], c["--panel"]]).toEqual([id, "#000000", "#000000"]);
    // What is raised from it is all but black, so the screen stays dark.
    expect([id, luminance(c["--panel-2"]) < 0.006]).toEqual([id, true]);
  }
  // And nothing outside that group claims to be (a near-black is not black).
  for (const [id, , group] of L.THEMES) {
    if (group === "black" || group === "contrast") continue;
    expect([id, (themes.get(id) as Record<string, string>)["--bg"] === "#000000"]).toEqual([id, false]);
  }
});

test("the two standard themes are what the app looks like before anything is chosen", () => {
  const block = (from: number) => appCss.slice(appCss.indexOf("{", from) + 1, appCss.indexOf("}", from));
  const vars = (body: string) =>
    Object.fromEntries([...body.matchAll(/(--[a-z0-9-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)].map((m) => [m[1], m[2]]));
  const dark = vars(block(appCss.indexOf(":root")));
  const lightAt = appCss.indexOf("@media (prefers-color-scheme: light)");
  const light = { ...dark, ...vars(block(appCss.indexOf(":root", lightAt))) };
  const colours = (id: string) =>
    Object.fromEntries(COLOURS.map((k) => [k, (themes.get(id) as Record<string, string>)[k]]));
  const only = (all: Record<string, string>) => Object.fromEntries(COLOURS.map((k) => [k, all[k]]));
  expect(colours("slate")).toEqual(only(dark));
  expect(colours("paper")).toEqual(only(light));
  // Until something is chosen, the system is followed between those two.
  expect([L.DEFAULTS.follow, L.DEFAULTS.day, L.DEFAULTS.night]).toEqual([true, "paper", "slate"]);
  // The themes come after the app's own colours, or a chosen theme
  // would lose to what the system prefers.
  const page = text("index.html");
  expect(page.indexOf("/console/app.css")).toBeLessThan(page.indexOf("/console/themes.css"));
});

test("the chooser offers exactly the typefaces the style sheet has, and every file is there with its licence", () => {
  const fonts = rulesFor(lookCss, "data-font");
  expect([...fonts.keys()].sort()).toEqual(L.FONTS.map((f) => f[0]).sort());
  const faces = [...lookCss.matchAll(/@font-face \{([^}]*)\}/g)].map((m) => ({
    family: /font-family: "([^"]+)"/.exec(m[1])?.[1] ?? "",
    file: /src: url\("\/console\/(fonts\/[^"]+)"\) format\("woff2"\)/.exec(m[1])?.[1] ?? "",
    weight: /font-weight: ([^;]+);/.exec(m[1])?.[1] ?? "",
    swap: m[1].includes("font-display: swap"),
  }));
  const listed = (JSON.parse(text("files.json")) as { files: { path: string }[] }).files.map((f) => f.path);
  const carried = new Set<string>();
  for (const [id, name, group, note] of L.FONTS) {
    const family = /^("pm-[a-z0-9-]+")?/.exec(fonts.get(id)?.["--font"] ?? "")?.[1];
    expect([id, name.length > 0, note.length > 0, L.FONT_GROUPS.some((g) => g[0] === group)]).toEqual([
      id,
      true,
      true,
      true,
    ]);
    if (id.startsWith("system")) {
      // The device's own: nothing to fetch.
      expect([id, family]).toEqual([id, undefined]);
      continue;
    }
    expect([id, family]).toEqual([id, `"pm-${id}"`]);
    const mine = faces.filter((f) => f.family === `pm-${id}`);
    expect([id, mine.length > 0]).toEqual([id, true]);
    // It falls back to one of the device's own while it is fetched, or if it cannot be.
    expect([id, /(sans-serif|serif|monospace)$/.test(fonts.get(id)?.["--font"] ?? "")]).toEqual([id, true]);
    for (const face of mine) {
      expect([id, face.file, listed.includes(face.file), face.swap, /^\d+( \d+)?$/.test(face.weight)]).toEqual([
        id,
        face.file,
        true,
        true,
        true,
      ]);
      const bytes = readFileSync(join(PUBLIC, face.file));
      expect([face.file, bytes.subarray(0, 4).toString("latin1")]).toEqual([face.file, "wOF2"]);
      carried.add(face.file);
    }
    const licence = text(`fonts/licences/${id}.txt`);
    expect([id, licence.includes("SIL OPEN FONT LICENSE Version 1.1"), /Copyright/.test(licence)]).toEqual([
      id,
      true,
      true,
    ]);
  }
  // No face without a choice, no file without a face, no licence without a typeface.
  expect(faces.filter((f) => !L.FONTS.some((font) => `pm-${font[0]}` === f.family))).toEqual([]);
  expect(listed.filter((p) => p.startsWith("fonts/")).sort()).toEqual([...carried].sort());
  const bundled = L.FONTS.filter((f) => !f[0].startsWith("system")).map((f) => `${f[0]}.txt`);
  expect(readdirSync(join(PUBLIC, "fonts/licences")).sort()).toEqual(bundled.sort());
  // A dozen or more, beyond the device's own.
  expect(bundled.length).toBeGreaterThanOrEqual(12);
  expect(new Set(L.FONTS.map((f) => f[1])).size).toBe(L.FONTS.length);
});

test("each step of size, spacing and corners is in the style sheet with its number, and the app is drawn with them", () => {
  for (const [attribute, variable, steps] of [
    ["data-size", "--scale", L.SIZES],
    ["data-space", "--space", L.SPACES],
    ["data-corners", "--round", L.CORNERS],
  ] as const) {
    const rules = rulesFor(lookCss, attribute);
    expect([...rules.keys()]).toEqual(steps.map((s) => s[0]));
    for (const [id, , value] of steps) expect([id, Number(rules.get(id)?.[variable])]).toEqual([id, value]);
    // In order, with the app's own look (1) among them, and it is the default.
    const values = steps.map((s) => s[2]);
    expect(values).toEqual([...values].sort((a, b) => a - b));
    expect(values).toContain(1);
  }
  expect([L.DEFAULTS.size, L.DEFAULTS.space, L.DEFAULTS.corners]).toEqual(["m", "regular", "regular"]);
  expect(L.SIZES.find((s) => s[0] === "m")?.[2]).toBe(1);
  // The root starts from 1 for each, and takes its typeface and size from them.
  const root = appCss.slice(appCss.indexOf(":root {"), appCss.indexOf("}", appCss.indexOf(":root {")));
  for (const start of [
    "--scale: 1;",
    "--space: 1;",
    "--round: 1;",
    "--edge: 1px;",
    "font-family: var(--font);",
    "font-size: calc(16px * var(--scale));",
  ]) {
    expect(root).toContain(start);
  }
  // Room: every padding, margin and gap of 2px or more goes by --space,
  // but for the frame's own (the fixed header and tab bar, and the
  // badge on a tab), which are written out here.
  const fixed = ["padding: 0 5px;", "padding-left: 96px;", "padding-bottom: 24px;", "padding-top: 12px;"];
  const raw: string[] = [];
  for (const m of appCss.matchAll(/^\s*((?:padding|margin|gap|row-gap|column-gap)[a-z-]*: [^;]*;)/gm)) {
    const decl = m[1];
    if (decl.includes("env(safe-area-inset-bottom) + 12px")) continue; // the page clearing the tab bar
    const bare = decl.replace(/calc\(\d+px \* var\(--space\)\)/g, "");
    if (/(?<![\w.-])([2-9]|\d{2,})px/.test(bare) && !fixed.includes(decl)) raw.push(decl);
  }
  expect(raw).toEqual([]);
  expect((appCss.match(/var\(--space\)/g) ?? []).length).toBeGreaterThan(70);
  // Corners: every radius goes by --round, but a full pill, which is a shape.
  const radii = [...appCss.matchAll(/border-radius: ([^;]+);/g)].map((m) => m[1]);
  expect(radii.filter((r) => !/^(0|999px|calc\(\d+px \* var\(--round\)\))$/.test(r))).toEqual([]);
  // Edges: a hairline is the theme's to make heavier.
  expect(appCss).not.toMatch(/border[a-z-]*: 1px solid var\(--line\)/);
  // No colour of its own: everything is drawn in the theme's.
  const after = appCss.slice(appCss.indexOf("* {"));
  expect(after.match(/#[0-9a-fA-F]{3,8}\b/g) ?? []).toEqual([]);
});

test("movement stops when the owner says so, and when the device asks unless the owner said otherwise", () => {
  expect(L.MOTIONS.map((m) => m[0])).toEqual(["system", "full", "less"]);
  expect(lookCss).toContain('[data-motion="less"] *,');
  const asked = lookCss.slice(lookCss.indexOf("@media (prefers-reduced-motion: reduce)"));
  expect(asked).toContain(':root:not([data-motion="full"]) *,');
  expect(asked).toContain("animation: none !important;");
  expect(asked).toContain("transition: none !important;");
});

test("choices that are missing, of the wrong kind or no longer on offer are the defaults", () => {
  expect(L.normal(null)).toEqual({ ...L.DEFAULTS });
  expect(L.normal("slate")).toEqual({ ...L.DEFAULTS });
  expect(L.normal([])).toEqual({ ...L.DEFAULTS });
  const mixed = L.normal({
    follow: "yes",
    theme: "a-theme-since-removed",
    day: "sepia",
    night: 7,
    font: "inter",
    size: "huge",
    space: "airy",
    corners: null,
    motion: "less",
    extra: "ignored",
  });
  expect(mixed).toEqual({
    follow: true,
    theme: "slate",
    day: "sepia",
    night: "slate",
    font: "inter",
    size: "m",
    space: "airy",
    corners: "regular",
    motion: "less",
  });
  // Every choice on offer is taken as it is.
  for (const [id] of L.THEMES) expect(L.normal({ theme: id }).theme).toBe(id);
  for (const [id] of L.FONTS) expect(L.normal({ font: id }).font).toBe(id);
  // A name that is one of an object's own workings is not a choice.
  expect(L.normal({ theme: "constructor", font: "__proto__" })).toEqual({ ...L.DEFAULTS });
  expect(L.schemeOf("ink")).toBe("light");
  expect(L.schemeOf("nope")).toBeUndefined();
});

test("following the system shows the day's theme when it is light and the night's when it is dark; otherwise the one chosen", () => {
  const s = { follow: true, theme: "nord", day: "sepia", night: "black" };
  expect(L.themeFor(s, false)).toBe("sepia");
  expect(L.themeFor(s, true)).toBe("black");
  expect(L.themeFor({ ...s, follow: false }, false)).toBe("nord");
  expect(L.themeFor({ ...s, follow: false }, true)).toBe("nord");
  expect(L.attrs({ ...s, font: "lexend", size: "xl", space: "tight", corners: "round", motion: "less" }, true)).toEqual(
    {
      "data-theme": "black",
      "data-scheme": "dark",
      "data-font": "lexend",
      "data-size": "xl",
      "data-space": "tight",
      "data-corners": "round",
      "data-motion": "less",
    },
  );
  // Movement left to the device is no attribute at all: the style sheet asks the device.
  expect(L.attrs({}, false)).toEqual({
    "data-theme": "paper",
    "data-scheme": "light",
    "data-font": "system",
    "data-size": "m",
    "data-space": "regular",
    "data-corners": "regular",
    "data-motion": null,
  });
  expect(L.attrs({ motion: "full" }, true)["data-motion"]).toBe("full");
});

/** A page, as far as the look touches it: a root element that records
 * its attributes, the bar-colour tag, the system's day or night, and
 * what the device keeps. */
function page(options: { dark?: boolean; stored?: string | null; refuses?: boolean; noStorage?: boolean } = {}) {
  const attributes = new Map<string, string>();
  const meta = { content: "#11161d" };
  const kept = new Map<string, string>();
  if (options.stored) kept.set(L.KEY, options.stored);
  const listeners: Record<string, ((event: unknown) => void)[]> = {};
  const media = {
    matches: options.dark ?? true,
    addEventListener: (_: string, fn: (event: unknown) => void) => {
      listeners.media = [...(listeners.media ?? []), fn];
    },
  };
  /** The page colour of each theme, as the style sheet has it. */
  const bgOf = (id: string | undefined) => (id ? ((themes.get(id) as Record<string, string>)["--bg"] ?? "") : "");
  const view = {
    document: {
      documentElement: {
        setAttribute: (name: string, value: string) => void attributes.set(name, value),
        removeAttribute: (name: string) => void attributes.delete(name),
      },
      querySelector: (selector: string) =>
        selector === 'meta[name="theme-color"]'
          ? { setAttribute: (_: string, value: string) => void (meta.content = value) }
          : null,
    },
    getComputedStyle: () => ({
      getPropertyValue: (name: string) => (name === "--bg" ? ` ${bgOf(attributes.get("data-theme"))} ` : ""),
    }),
    matchMedia: (query: string) => {
      expect(query).toBe("(prefers-color-scheme: dark)");
      return media;
    },
    addEventListener: (name: string, fn: (event: unknown) => void) => {
      listeners[name] = [...(listeners[name] ?? []), fn];
    },
    get localStorage() {
      if (options.noStorage) throw new DOMException("The operation is insecure.", "SecurityError");
      return {
        getItem: (key: string) => kept.get(key) ?? null,
        setItem: (key: string, value: string) => {
          if (options.refuses) throw new DOMException("Quota exceeded", "QuotaExceededError");
          kept.set(key, value);
        },
      };
    },
  };
  return {
    view,
    attributes,
    meta,
    kept,
    media,
    fire: (name: string, event: unknown = {}) => (listeners[name] ?? []).forEach((fn) => fn(event)),
  };
}
const shown = (p: ReturnType<typeof page>) => Object.fromEntries([...p.attributes.entries()].sort());

test("opened, the page wears what this device chose, and the bar around it takes the theme's colour", () => {
  const stored = JSON.stringify({
    follow: false,
    theme: "black-amber",
    font: "jetbrains-mono",
    size: "l",
    space: "snug",
    corners: "square",
    motion: "full",
  });
  const p = page({ stored });
  L.start(p.view);
  expect(shown(p)).toEqual({
    "data-corners": "square",
    "data-font": "jetbrains-mono",
    "data-motion": "full",
    "data-scheme": "dark",
    "data-size": "l",
    "data-space": "snug",
    "data-theme": "black-amber",
  });
  expect(p.meta.content).toBe("#000000");
  // Nothing chosen yet: the system is followed, as before there was a choice.
  const fresh = page({ dark: false });
  L.start(fresh.view);
  expect(shown(fresh)).toEqual({
    "data-corners": "regular",
    "data-font": "system",
    "data-scheme": "light",
    "data-size": "m",
    "data-space": "regular",
    "data-theme": "paper",
  });
  expect(fresh.meta.content).toBe("#f4f6f9");
  // What is kept is not what this file wrote (edited by hand, or cut short): the defaults.
  for (const bad of ["{not json", '"a string"', "null", '{"theme":42}']) {
    const odd = page({ stored: bad, dark: true });
    L.start(odd.view);
    expect([bad, shown(odd)["data-theme"]]).toEqual([bad, "slate"]);
  }
  // A browser that gives the site no storage at all still gets a page.
  const none = page({ noStorage: true, dark: true });
  L.start(none.view);
  expect(shown(none)["data-theme"]).toBe("slate");
});

test("the page follows the system from day to night, and another tab's change, without being reloaded", () => {
  const p = page({ dark: false, stored: JSON.stringify({ follow: true, day: "mint", night: "black-ice" }) });
  L.start(p.view);
  expect([shown(p)["data-theme"], p.meta.content]).toEqual(["mint", "#eef7f2"]);
  p.media.matches = true;
  p.fire("media");
  expect([shown(p)["data-theme"], p.meta.content]).toEqual(["black-ice", "#000000"]);
  // Another tab chose something else.
  p.kept.set(L.KEY, JSON.stringify({ follow: false, theme: "sepia", motion: "less" }));
  p.fire("storage", { key: "something.else" });
  expect(shown(p)["data-theme"]).toBe("black-ice"); // not ours: nothing changes
  p.fire("storage", { key: L.KEY });
  expect([shown(p)["data-theme"], shown(p)["data-motion"]]).toEqual(["sepia", "less"]);
  // Everything this site kept was cleared: back to the defaults.
  p.kept.clear();
  p.fire("storage", { key: null });
  expect([shown(p)["data-theme"], "data-motion" in shown(p)]).toEqual(["slate", false]);
});

test("a choice is kept on the device and shown at once; a device that will not keep it is told apart", () => {
  const p = page({ dark: true });
  L.start(p.view);
  const first = L.choose({ follow: false, theme: "dracula" }, p.view);
  expect([first.kept, first.settings.theme, shown(p)["data-theme"], p.meta.content]).toEqual([
    true,
    "dracula",
    "dracula",
    "#282a36",
  ]);
  // The next choice keeps the last one.
  const second = L.choose({ font: "literata", size: "xxl" }, p.view);
  expect(second.settings).toEqual({ ...L.DEFAULTS, follow: false, theme: "dracula", font: "literata", size: "xxl" });
  expect(JSON.parse(p.kept.get(L.KEY) as string)).toEqual(second.settings);
  expect(L.read(p.view.localStorage)).toEqual(second.settings);
  // Something not on offer changes nothing of that kind.
  expect(L.choose({ theme: "nope", space: "roomy" }, p.view).settings).toEqual({ ...second.settings, space: "roomy" });
  // Movement back to the device's own: the attribute goes.
  L.choose({ motion: "less" }, p.view);
  expect(shown(p)["data-motion"]).toBe("less");
  L.choose({ motion: "system" }, p.view);
  expect(shown(p)["data-motion"]).toBeUndefined();
  // Storage refused: shown all the same, and said not to be kept.
  const full = page({ dark: true, refuses: true });
  const tried = L.choose({ follow: false, theme: "ink" }, full.view);
  expect([tried.kept, shown(full)["data-theme"], full.kept.size]).toEqual([false, "ink", 0]);
  expect(L.write({ setItem: () => undefined }, {})).toBe(true);
  expect(L.write(null, {})).toBe(false);
});

test("the look is on the page before it is first drawn, and nothing writes a style the page's policy would refuse", () => {
  const html = text("index.html");
  const head = html.slice(0, html.indexOf("</head>"));
  // After the style sheets (it reads the page colour from them), in the
  // head (so before anything is shown), and not deferred.
  const at = head.indexOf('<script src="/console/look.js"></script>');
  expect(at).toBeGreaterThan(head.indexOf("/console/look.css"));
  expect(at).toBeGreaterThan(head.indexOf("/console/themes.css"));
  expect(html).not.toMatch(/\sstyle=/);
  expect(html).not.toMatch(/<style/);
  // Attributes only: the policy (default-src 'self') has no room for a written style.
  expect(text("look.js")).not.toMatch(/\.style\b|"style"|cssText/);
  expect(text("app.js")).not.toMatch(/setAttribute\("style"|\bstyle: /);
});

test("the screen says which theme is shown and why, and a choice goes to the right one of the two", () => {
  const s = { ...L.DEFAULTS, day: "sepia", night: "black", theme: "nord" };
  expect(lookSaid(s, true, L)).toBe(
    "Following the system: Sepia when it is light, Black when it is dark. It is dark now, so Black.",
  );
  expect(lookSaid(s, false, L)).toBe(
    "Following the system: Sepia when it is light, Black when it is dark. It is light now, so Sepia.",
  );
  expect(lookSaid({ ...s, follow: false }, true, L)).toBe("Nord, whatever the system is set to.");
  expect(themeChange(s, "day", "mint")).toEqual({ day: "mint" });
  expect(themeChange(s, "night", "ocean")).toEqual({ night: "ocean" });
  expect(themeChange({ ...s, follow: false }, null, "ink")).toEqual({ theme: "ink" });
  const black = L.THEMES.filter((t) => t[2] === "black").length;
  expect(lookCounts(L)).toBe(
    `${L.THEMES.length} themes (${black} true black), ${L.FONTS.length} typefaces, text size, spacing`,
  );
});
