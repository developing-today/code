// The dashboard saved-views store (#1276). A saved view is a named,
// per-person filter set over the canonical work-item projection — the
// same items the Board / Projects / Roadmap pages and their
// /api/*.json twins render — so a view is a parameter set over the
// existing pages, never a new surface. Views key on canonical item
// identity (owner/repo#N refs), never render order, so they survive
// feed republishes.
//
// Storage follows the comments store discipline (#906, comments.ts):
// one JSON file in the aggregator's data directory
// (PM_AGG_VIEWS_FILE); writes are atomic (unique temp file + rename)
// and serialized through a per-store promise chain; a missing file is
// the only silent empty start — an unparseable or wrongly-shaped file
// is quarantined to <file>.corrupt-<timestamp> with a loud log, never
// silently overwritten.
//
// Mutations arrive through the mediated action path (#1276 scope):
// preference verbs save-view / rename-view / delete-view /
// set-default-view are recorded in the actions store like every other
// dashboard action, and the aggregator applies them to this store and
// marks the action applied — every change is audited in both places.
//
// Identity: a view's owner is the dashboard author key
// ("<kind>:<id>" from the validated DashAuthor, see ownerKey below) —
// the same per-person identity the preference actions already carry.
// A person's default view applies on landing (the page handlers
// resolve it when the request names that viewer) until cleared.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { DashAuthor } from "./model.ts";

/** The cross-cutting filter dimensions a saved view pins (#1276). */
export interface ViewFilters {
  repo?: string;
  area?: string;
  horizon?: string;
  owner?: string;
  state?: string;
  epic?: string;
}

export const VIEW_FILTER_KEYS = ["repo", "area", "horizon", "owner", "state", "epic"] as const;

export interface SavedView {
  id: string;
  /** Display name, unique per owner (case-insensitive). */
  name: string;
  /** Dashboard author key — see ownerKey. */
  owner: string;
  filters: ViewFilters;
  isDefault: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ViewsFile {
  version: 1;
  views: SavedView[];
}

/** The per-person key views are stored under: "<kind>:<id>". */
export function ownerKey(author: DashAuthor): string {
  return `${author.kind}:${author.id}`;
}

const FILTER_VALUE = /^[\w ./#:-]+$/;

export function validateViewFilters(f: unknown): { ok: true; value: ViewFilters } | { ok: false; error: string } {
  if (typeof f !== "object" || f === null) return { ok: false, error: "filters must be an object" };
  const out: ViewFilters = {};
  for (const [k, v] of Object.entries(f as Record<string, unknown>)) {
    if (!(VIEW_FILTER_KEYS as readonly string[]).includes(k)) return { ok: false, error: `unknown filter key: ${k}` };
    if (typeof v !== "string" || v.trim() === "" || v.length > 200 || !FILTER_VALUE.test(v)) {
      return { ok: false, error: `filter ${k} must be a non-empty string` };
    }
    out[k as keyof ViewFilters] = v.trim();
  }
  if (Object.keys(out).length === 0) return { ok: false, error: "a view needs at least one filter" };
  return { ok: true, value: out };
}

export function validateViewName(n: unknown): string | null {
  if (typeof n !== "string") return null;
  const name = n.trim();
  return name !== "" && name.length <= 120 ? name : null;
}

let counter = 0;
export function newViewId(now = new Date()): string {
  counter = (counter + 1) % 46656;
  const rand = Math.random().toString(36).slice(2, 7);
  return `vw-${now.getTime().toString(36)}-${counter.toString(36)}${rand}`;
}

// -- filter application -------------------------------------------------
// One matcher over the canonical item shape. Each page/projection maps
// its items onto these fields (see server.ts): ref is the canonical
// owner/repo#N key; epic is the item's epic ref when it has one.

export interface ViewItemLike {
  ref: string;
  repo?: string | null;
  area?: string | null;
  horizon?: string | null;
  owner?: string | null;
  state?: string | null;
  epic?: string | null;
}

function eq(a: string | null | undefined, b: string): boolean {
  return typeof a === "string" && a.toLowerCase() === b.toLowerCase();
}

export function itemMatchesView(item: ViewItemLike, filters: ViewFilters): boolean {
  if (filters.repo) {
    const repo = item.repo ?? item.ref.replace(/#\d+$/, "");
    if (!eq(repo, filters.repo)) return false;
  }
  if (filters.area && !eq(item.area, filters.area)) return false;
  if (filters.horizon && !eq(item.horizon, filters.horizon)) return false;
  if (filters.owner && !eq(item.owner, filters.owner)) return false;
  if (filters.state && !eq(item.state, filters.state)) return false;
  if (filters.epic && !eq(item.epic, filters.epic)) return false;
  return true;
}

export function applyView<T extends ViewItemLike>(items: readonly T[], filters: ViewFilters): T[] {
  return items.filter((i) => itemMatchesView(i, filters));
}

// -- shareable URLs -----------------------------------------------------
// A view URL encodes BOTH the view id (store lookup for the owner) and
// the full filter set as vf.* query params, so the URL alone reproduces
// the view exactly for another signed-in viewer even before — or
// without — a store lookup.

export function viewQuery(view: Pick<SavedView, "id" | "filters">): string {
  const p = new URLSearchParams();
  p.set("view", view.id);
  for (const k of VIEW_FILTER_KEYS) {
    const v = view.filters[k];
    if (v) p.set(`vf.${k}`, v);
  }
  return p.toString();
}

export function viewUrl(page: string, view: Pick<SavedView, "id" | "filters">): string {
  return `${page}?${viewQuery(view)}`;
}

/** The filter set a request's vf.* params encode, or null if none. */
export function filtersFromQuery(params: URLSearchParams): ViewFilters | null {
  const out: ViewFilters = {};
  let any = false;
  for (const k of VIEW_FILTER_KEYS) {
    const v = params.get(`vf.${k}`);
    if (v) {
      out[k] = v;
      any = true;
    }
  }
  return any ? out : null;
}

// -- the store ------------------------------------------------------------

export class ViewsStore {
  private views: SavedView[] = [];
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly file: string) {}

  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") {
        this.views = [];
        return;
      }
      this.views = [];
      await this.quarantine(`could not be read (${err instanceof Error ? err.message : String(err)})`);
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || !Array.isArray((parsed as ViewsFile).views)) {
        throw new Error("store file does not have the expected { views: [...] } shape");
      }
      this.views = (parsed as ViewsFile).views.filter(
        (v) => typeof v === "object" && v !== null && typeof (v as SavedView).id === "string",
      );
    } catch (err) {
      this.views = [];
      await this.quarantine(`is unparseable (${err instanceof Error ? err.message : String(err)})`);
    }
  }

  private async quarantine(why: string): Promise<void> {
    const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
    try {
      await rename(this.file, quarantined);
      console.error(
        `[views] WARNING: views store ${this.file} ${why}; quarantined the original bytes to ${quarantined} and started empty. ` +
          `The quarantined file holds the saved views — do not delete it.`,
      );
    } catch (renameErr) {
      console.error(
        `[views] WARNING: views store ${this.file} ${why} and could not be quarantined ` +
          `(${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
          `the next persist will overwrite it, so copy it aside now to preserve the saved views.`,
      );
    }
  }

  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      const data: ViewsFile = { version: 1, views: this.views };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  async all(): Promise<SavedView[]> {
    await this.load();
    return [...this.views].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  }

  async forOwner(owner: string): Promise<SavedView[]> {
    return (await this.all()).filter((v) => v.owner === owner);
  }

  async get(id: string): Promise<SavedView | null> {
    await this.load();
    return this.views.find((v) => v.id === id) ?? null;
  }

  async defaultFor(owner: string): Promise<SavedView | null> {
    return (await this.forOwner(owner)).find((v) => v.isDefault) ?? null;
  }

  /** Create a view for the owner. Names are unique per owner. */
  async save(input: { owner: string; name: string; filters: ViewFilters }): Promise<SavedView> {
    await this.load();
    if (this.views.some((v) => v.owner === input.owner && v.name.toLowerCase() === input.name.toLowerCase())) {
      throw new Error(`a view named "${input.name}" already exists for this owner`);
    }
    const now = new Date().toISOString();
    const view: SavedView = {
      id: newViewId(),
      name: input.name,
      owner: input.owner,
      filters: { ...input.filters },
      isDefault: false,
      createdAt: now,
      updatedAt: now,
    };
    this.views.push(view);
    await this.persist();
    return view;
  }

  async rename(id: string, owner: string, name: string): Promise<SavedView | null> {
    await this.load();
    const v = this.views.find((x) => x.id === id && x.owner === owner);
    if (!v) return null;
    if (this.views.some((x) => x.owner === owner && x.id !== id && x.name.toLowerCase() === name.toLowerCase())) {
      throw new Error(`a view named "${name}" already exists for this owner`);
    }
    v.name = name;
    v.updatedAt = new Date().toISOString();
    await this.persist();
    return v;
  }

  async remove(id: string, owner: string): Promise<boolean> {
    await this.load();
    const before = this.views.length;
    this.views = this.views.filter((v) => !(v.id === id && v.owner === owner));
    if (this.views.length === before) return false;
    await this.persist();
    return true;
  }

  /** Make one of the owner's views the default (at most one per
   *  owner); id null clears the owner's default. */
  async setDefault(id: string | null, owner: string): Promise<SavedView | null> {
    await this.load();
    let target: SavedView | null = null;
    for (const v of this.views) {
      if (v.owner !== owner) continue;
      v.isDefault = id !== null && v.id === id;
      if (v.isDefault) target = v;
    }
    if (id !== null && !target) return null;
    await this.persist();
    return target;
  }
}
