// #1275: command palette (Cmd+K) + keyboard-first triage on /attention.
// The palette searches only data already served (page manifest + the
// /api/*.json twins); one-key actions submit the card's existing .aform
// so the POST /api/actions envelope — and its audit entry — is the
// click path's, byte for byte.
import { describe, expect, test } from "bun:test";
import { PAGES } from "../src/pages.ts";
import {
  KEY_HINT,
  KEY_VERBS,
  KEYBOARD_SCRIPT,
  filterEntries,
  flattenItems,
  pageEntries,
  paletteHtml,
  refToUrl,
} from "../src/keyboard.ts";
import { renderAttentionShell } from "../src/render.ts";

describe("palette entries from the manifest", () => {
  test("every manifest page is a palette entry with its route", () => {
    const entries = pageEntries();
    expect(entries.length).toBe(PAGES.length);
    for (const p of PAGES) {
      const e = entries.find((x) => x.href === p.route);
      expect(e?.title).toBe(p.title);
      expect(e?.kind).toBe("page");
    }
  });

  test("filter matches title and blurb, case-insensitively", () => {
    const hits = filterEntries(pageEntries(), "NEEDS");
    expect(hits.some((e) => e.href === "/attention")).toBe(true);
    expect(filterEntries(pageEntries(), "no-such-page-xyz")).toEqual([]);
    expect(filterEntries(pageEntries(), "").length).toBeGreaterThan(0);
  });
});

describe("flattenItems over the served twins", () => {
  test("attention items (title + ref, no URL) link to the GitHub issue", () => {
    const twin = [
      { id: "developing-today/code#1275", kind: "task", title: "Palette work", ref: "developing-today/code#1275", tier: "needs-drew" },
      { id: "session:abc", kind: "session", title: "A session", tier: "needs-drew" },
    ];
    const entries = flattenItems(twin, "attention");
    expect(entries).toContainEqual({
      title: "Palette work",
      hint: "attention",
      href: "https://github.com/developing-today/code/issues/1275",
      kind: "item",
    });
    // Session items have no URL of their own: they land on /attention.
    expect(entries).toContainEqual({ title: "A session", hint: "attention", href: "/attention", kind: "item" });
  });

  test("url/href/route fields win; untitled objects are skipped", () => {
    const entries = flattenItems({ rows: [{ name: "Svc", url: "/quota/service/x" }, { nope: 1 }] }, "overview");
    expect(entries).toEqual([{ title: "Svc", hint: "overview", href: "/quota/service/x", kind: "item" }]);
  });

  test("refToUrl only accepts owner/repo#n", () => {
    expect(refToUrl("developing-today/code#42")).toBe("https://github.com/developing-today/code/issues/42");
    expect(refToUrl("session:abc")).toBeNull();
    expect(refToUrl(42)).toBeNull();
  });
});

describe("keyboard layer wiring", () => {
  test("the one-key map covers the triage verbs from the packet", () => {
    expect(KEY_VERBS.s).toEqual(["snooze"]);
    expect(KEY_VERBS.e).toContain("escalate");
    expect(KEY_VERBS.e).toContain("de-escalate");
    expect(KEY_VERBS.g).toEqual(["delegate"]);
    expect(KEY_VERBS.x).toEqual(["dismiss"]);
    expect(KEY_VERBS.m).toEqual(["mark-done"]);
  });

  test("the script opens on Cmd/Ctrl+K and submits existing forms — never builds an action body", () => {
    expect(KEYBOARD_SCRIPT).toContain("metaKey");
    expect(KEYBOARD_SCRIPT).toContain("requestSubmit()");
    // No second write path: the keyboard script must not POST anywhere.
    expect(KEYBOARD_SCRIPT).not.toContain("/api/actions");
    expect(KEYBOARD_SCRIPT).not.toContain('method: "POST"');
    expect(KEYBOARD_SCRIPT).not.toContain("method: 'POST'");
  });

  test("palette markup + script + key hint ship in the shared shell", () => {
    const html = paletteHtml();
    expect(html).toContain('id="cmdk"');
    expect(html).toContain('id="cmdk-input"');
    expect(html).toContain('id="kbd-help"');
    expect(html).toContain("/api/attention.json");
    const shell = renderAttentionShell({
      board: "test",
      roadmap: "test",
      items: "test",
      comments: "test",
      quota: "test",
      usage: "test",
      attention: "test",
      orchestration: "test",
      dns: "test",
      registry: "test",
      aa: "test",
    });
    expect(shell).toContain('id="cmdk"');
    expect(shell).toContain("cmdk-pages");
    expect(KEY_HINT).toContain("⌘K");
    expect(shell).toContain("⌘K");
  });
});
