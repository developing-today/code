// #2851: the cost-explorer saved-setups store keyed setups by name in
// a plain JSON.parse'd object, so a setup named "__proto__" assigned
// the store's prototype instead of becoming an entry — silently never
// saved — and importing a "__proto__" entry mutated the live store's
// prototype while dropping the entry. The fix makes loadSetups()
// return a null-prototype store. These tests extract the REAL
// loadSetups/storeSetups from src/render.ts and drive the save/import
// handler cores verbatim against a stub localStorage.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const renderSource = readFileSync(join(import.meta.dir, "..", "src", "render.ts"), "utf8");
const storeFns = renderSource.slice(
  renderSource.indexOf("function loadSetups()"),
  renderSource.indexOf("function refreshSetupList"),
);
expect(storeFns).toContain("function storeSetups(");

function makeStore() {
  const backing = new Map<string, string>();
  const localStorage = {
    getItem: (k: string) => (backing.has(k) ? backing.get(k)! : null),
    setItem: (k: string, v: string) => void backing.set(k, String(v)),
  };
  const factory = new Function("localStorage", `${storeFns}; return { loadSetups, storeSetups };`) as (ls: unknown) => {
    loadSetups: () => Record<string, { tab?: string; marker?: string }>;
    storeSetups: (obj: unknown) => void;
  };
  const { loadSetups, storeSetups } = factory(localStorage);
  // The save handler core (render.ts setupSaveEl), verbatim shape.
  const saveNamed = (name: string) => {
    const all = loadSetups();
    all[name] = { tab: "aa", marker: `saved-${name}` };
    storeSetups(all);
  };
  // The import handler core (render.ts setupImportEl reader.onload).
  const importText = (text: string) => {
    const parsed = JSON.parse(text);
    if (parsed && typeof parsed === "object") {
      const all = loadSetups();
      Object.keys(parsed).forEach((k) => {
        all[k] = parsed[k];
      });
      storeSetups(all);
    }
  };
  return { backing, loadSetups, saveNamed, importText };
}

describe("setups store (#2851)", () => {
  test("the shipped loadSetups builds a null-prototype store", () => {
    expect(storeFns).toContain("Object.create(null)");
    const { loadSetups } = makeStore();
    expect(Object.getPrototypeOf(loadSetups())).toBeNull();
  });

  test("a setup named '__proto__' saves, reloads, and exports as an entry", () => {
    const { backing, loadSetups, saveNamed } = makeStore();
    saveNamed("Normal");
    saveNamed("__proto__");
    const reloaded = loadSetups();
    expect(Object.keys(reloaded).sort()).toEqual(["Normal", "__proto__"]);
    expect(Object.prototype.hasOwnProperty.call(reloaded, "__proto__")).toBe(true);
    expect(reloaded["__proto__"].marker).toBe("saved-__proto__");
    // Export is JSON.stringify(loadSetups()) — the entry must be in it.
    const exported = JSON.parse(backing.get("xpl-setups-v1")!);
    expect(Object.keys(exported).sort()).toEqual(["Normal", "__proto__"]);
    expect(exported["__proto__"].marker).toBe("saved-__proto__");
  });

  test("controls: constructor / toString / ordinary names behave as before", () => {
    const { loadSetups, saveNamed } = makeStore();
    for (const name of ["constructor", "toString", "Normal"]) saveNamed(name);
    const reloaded = loadSetups();
    expect(Object.keys(reloaded).sort()).toEqual(["Normal", "constructor", "toString"]);
    expect(reloaded["constructor"].marker).toBe("saved-constructor");
    expect(reloaded["toString"].marker).toBe("saved-toString");
  });

  test("importing a raw '__proto__' entry stores it as an entry, no prototype mutation", () => {
    const { loadSetups, importText } = makeStore();
    importText('{"__proto__":{"tab":"polluted-tab","marker":"imported-proto"},"Real":{"tab":"aa","marker":"real"}}');
    const reloaded = loadSetups();
    expect(Object.keys(reloaded).sort()).toEqual(["Real", "__proto__"]);
    expect(Object.prototype.hasOwnProperty.call(reloaded, "__proto__")).toBe(true);
    expect(reloaded["__proto__"].tab).toBe("polluted-tab");
    expect(Object.getPrototypeOf(reloaded)).toBeNull();
    // Nothing leaked into unrelated objects' prototype chain.
    expect(({} as { tab?: string }).tab).toBeUndefined();
  });
});
