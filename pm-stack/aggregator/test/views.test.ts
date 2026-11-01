// #1276: saved views — named per-person filter sets over the canonical
// projection. Covers the views store (round-trip, per-owner defaults,
// duplicate names, corrupt-file quarantine), the pure filter matching
// (keyed on canonical refs, so a republish that reorders items yields
// the identical set), the shareable-URL encode/parse round-trip, and
// the mediated preference verbs that mutate the store.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { validateNewAction } from "../src/actions.ts";
import {
  ViewsStore,
  applyView,
  filtersFromQuery,
  ownerKey,
  validateViewFilters,
  viewUrl,
  type ViewItemLike,
} from "../src/views.ts";

const drew = { kind: "drew" as const, id: "drew", label: "Drew" };
const claude = { kind: "muse" as const, id: "claude", label: "Muse" };
const drewKey = ownerKey(drew);

function tmpStore(): { dir: string; file: string; store: ViewsStore } {
  const dir = mkdtempSync(join(tmpdir(), "agg-views-"));
  const file = join(dir, "views.json");
  return { dir, file, store: new ViewsStore(file) };
}

describe("views store (#1276)", () => {
  test("save -> reload round-trip persists views with filters and timestamps", async () => {
    const { file, store } = tmpStore();
    const v = await store.save({ owner: drewKey, name: "Code bugs", filters: { repo: "developing-today/code", state: "open" } });
    expect(v.id).toStartWith("vw-");
    expect(v.isDefault).toBe(false);
    const reloaded = new ViewsStore(file);
    const all = await reloaded.forOwner(drewKey);
    expect(all).toHaveLength(1);
    expect(all[0]).toMatchObject({ name: "Code bugs", filters: { repo: "developing-today/code", state: "open" } });
    expect(await reloaded.forOwner(ownerKey(claude))).toEqual([]);
  });

  test("duplicate name per owner is refused; same name for another owner is fine", async () => {
    const { store } = tmpStore();
    await store.save({ owner: drewKey, name: "Mine", filters: { area: "Fleet" } });
    await expect(store.save({ owner: drewKey, name: "mine", filters: { area: "Other" } })).rejects.toThrow("already exists");
    await store.save({ owner: ownerKey(claude), name: "Mine", filters: { area: "Fleet" } });
    expect(await store.all()).toHaveLength(2);
  });

  test("rename, delete, and set-default are per-owner; default is singular and survives reload", async () => {
    const { file, store } = tmpStore();
    const a = await store.save({ owner: drewKey, name: "A", filters: { repo: "dezren39/nix" } });
    const b = await store.save({ owner: drewKey, name: "B", filters: { horizon: "Now" } });
    await store.save({ owner: ownerKey(claude), name: "C", filters: { repo: "dezren39/nix" } });

    await store.setDefault(a.id, drewKey);
    expect((await store.defaultFor(drewKey))?.id).toBe(a.id);
    await store.setDefault(b.id, drewKey);
    expect((await store.defaultFor(drewKey))?.id).toBe(b.id);
    expect((await store.get(a.id))?.isDefault).toBe(false);
    // Another owner's views cannot be renamed/deleted/defaulted by drew.
    const c = (await store.forOwner(ownerKey(claude)))[0];
    expect(await store.rename(c.id, drewKey, "stolen")).toBeNull();
    expect(await store.remove(c.id, drewKey)).toBe(false);
    expect(await store.setDefault(c.id, drewKey)).toBeNull();

    await store.rename(b.id, drewKey, "B renamed");
    expect((await store.get(b.id))?.name).toBe("B renamed");
    expect(await store.remove(a.id, drewKey)).toBe(true);
    await store.setDefault(null, drewKey);
    expect(await store.defaultFor(drewKey)).toBeNull();

    const reloaded = new ViewsStore(file);
    expect((await reloaded.all()).map((v) => v.name).sort()).toEqual(["B renamed", "C"]);
  });

  test("corrupt views file is quarantined with bytes preserved, never silently overwritten", async () => {
    const { dir, file, store } = tmpStore();
    writeFileSync(file, '{"version":1,"views":[{"id":"vw-1"');
    expect(await store.all()).toEqual([]);
    const quarantined = readdirSync(dir).filter((f) => f.startsWith("views.json.corrupt-"));
    expect(quarantined).toHaveLength(1);
    await store.save({ owner: drewKey, name: "Fresh", filters: { state: "open" } });
    expect(existsSync(file)).toBe(true);
    expect(readdirSync(dir).filter((f) => f.startsWith("views.json.corrupt-"))).toHaveLength(1);
  });
});

const items: ViewItemLike[] = [
  { ref: "developing-today/code#1", repo: "developing-today/code", area: "Fleet", horizon: "Now", owner: "dezren39", state: "open", epic: "developing-today/code#100" },
  { ref: "developing-today/code#2", repo: "developing-today/code", area: "Fleet", horizon: "Next", owner: "someone", state: "closed", epic: null },
  { ref: "dezren39/nix#3", repo: "dezren39/nix", area: "Infra", horizon: "Now", owner: "dezren39", state: "open", epic: null },
];

describe("view filter matching (#1276)", () => {
  test("filters compose across dimensions and compare case-insensitively", () => {
    expect(applyView(items, { repo: "Developing-Today/Code", state: "OPEN" }).map((i) => i.ref)).toEqual(["developing-today/code#1"]);
    expect(applyView(items, { owner: "dezren39", horizon: "now" }).map((i) => i.ref)).toEqual([
      "developing-today/code#1",
      "dezren39/nix#3",
    ]);
    expect(applyView(items, { epic: "developing-today/code#100" }).map((i) => i.ref)).toEqual(["developing-today/code#1"]);
  });

  test("republish survival: reordering the feed does not change the matched set", () => {
    const filters = { area: "Fleet" };
    const forward = applyView(items, filters).map((i) => i.ref);
    const reversed = applyView([...items].reverse(), filters).map((i) => i.ref).sort();
    expect([...forward].sort()).toEqual(reversed);
  });

  test("validateViewFilters rejects unknown keys, empty values, and empty sets", () => {
    expect(validateViewFilters({ repo: "a/b" }).ok).toBe(true);
    expect(validateViewFilters({}).ok).toBe(false);
    expect(validateViewFilters({ bogus: "x" }).ok).toBe(false);
    expect(validateViewFilters({ repo: "" }).ok).toBe(false);
    expect(validateViewFilters("nope").ok).toBe(false);
  });
});

describe("shareable view URLs (#1276)", () => {
  test("the URL encodes id + full filter set and parses back exactly", async () => {
    const { store } = tmpStore();
    const v = await store.save({
      owner: drewKey,
      name: "Fleet now",
      filters: { repo: "developing-today/code", area: "Fleet", horizon: "Now" },
    });
    const url = viewUrl("/board", v);
    const parsed = new URL(url, "http://localhost");
    expect(parsed.pathname).toBe("/board");
    expect(parsed.searchParams.get("view")).toBe(v.id);
    // Another viewer reproduces the view from the URL alone.
    expect(filtersFromQuery(parsed.searchParams)).toEqual(v.filters);
    expect(applyView(items, filtersFromQuery(parsed.searchParams)!).map((i) => i.ref)).toEqual(["developing-today/code#1"]);
  });

  test("a plain page URL carries no view", () => {
    expect(filtersFromQuery(new URL("http://localhost/board").searchParams)).toBeNull();
  });
});

describe("mediated view preference verbs (#1276)", () => {
  const base = {
    kind: "preference" as const,
    objectRef: "dash:views",
    author: drew,
  };
  let seq = 0;
  const token = () => `ct-views-${++seq}-abcdefgh`;

  test("save-view requires a name and a non-empty filter set", () => {
    const ok = validateNewAction({ ...base, clientToken: token(), verb: "save-view", payload: { viewName: "Mine", viewFilters: { repo: "a/b" } } });
    expect(ok.ok).toBe(true);
    const noFilters = validateNewAction({ ...base, clientToken: token(), verb: "save-view", payload: { viewName: "Mine" } });
    expect(noFilters.ok).toBe(false);
    const badFilters = validateNewAction({ ...base, clientToken: token(), verb: "save-view", payload: { viewName: "Mine", viewFilters: { nope: "x" } } });
    expect(badFilters.ok).toBe(false);
  });

  test("rename-view and delete-view require viewId; set-default-view allows clearing", () => {
    expect(validateNewAction({ ...base, clientToken: token(), verb: "rename-view", payload: { viewName: "X" } }).ok).toBe(false);
    expect(validateNewAction({ ...base, clientToken: token(), verb: "rename-view", payload: { viewId: "vw-1", viewName: "X" } }).ok).toBe(true);
    expect(validateNewAction({ ...base, clientToken: token(), verb: "delete-view", payload: {} }).ok).toBe(false);
    expect(validateNewAction({ ...base, clientToken: token(), verb: "delete-view", payload: { viewId: "vw-1" } }).ok).toBe(true);
    expect(validateNewAction({ ...base, clientToken: token(), verb: "set-default-view", payload: {} }).ok).toBe(true);
    expect(validateNewAction({ ...base, clientToken: token(), verb: "set-default-view", payload: { viewId: "vw-1" } }).ok).toBe(true);
  });
});
