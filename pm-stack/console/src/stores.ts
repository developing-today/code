// The Turso stores, as the platform names them.
//
// The repository holds a list of every Turso database the platform's
// code names (pm-stack/platform/stores.json): what each is, what uses
// it, its address and the name of the secret that holds its token. This
// reads that list for the app's Platform screen. A token is only ever
// named here, never valued.

export interface StoreRow {
  /** Its database name on Turso. */
  name: string;
  title: string;
  does: string;
  /** What reads or writes it. */
  usedBy: string[];
  /** Where it answers. */
  address: string;
  /** The secret that holds its token, or how the token is set. */
  token: string;
  /** The tables the app reads from it. */
  tables: string[];
  /** Where its schema version is recorded, when it is. */
  schemaVersion: string | null;
}

export const STORES_FILE = "pm-stack/platform/stores.json";

const NAME = /^[a-z0-9][a-z0-9-]{0,62}$/;
const TABLE = /^[a-z_][a-z0-9_]{0,62}$/;
const ADDRESS = /^https:\/\/[a-z0-9-]+-dezren39\.aws-[a-z0-9-]+\.turso\.io$/;
const text = (value: unknown, max: number): string | null =>
  typeof value === "string" && value.trim() !== "" && value.length <= max ? value : null;
const textList = (value: unknown, max: number, item: RegExp | null): string[] | null => {
  if (!Array.isArray(value) || value.length > 50) return null;
  const items = value.map((v) => text(v, max));
  if (items.some((v) => v === null || (item !== null && !item.test(v)))) return null;
  return items as string[];
};

/** One store as the list may give it, or null. */
export function storeRow(value: unknown): StoreRow | null {
  const v = value as Record<string, unknown> | null;
  if (!v || typeof v !== "object") return null;
  const title = text(v.title, 80);
  const does = text(v.does, 400);
  const token = text(v.token, 120);
  const usedBy = textList(v.used_by, 120, null);
  const tables = textList(v.tables, 63, TABLE);
  if (typeof v.name !== "string" || !NAME.test(v.name) || !title || !does || !token || !usedBy || !tables) return null;
  if (typeof v.address !== "string" || !ADDRESS.test(v.address)) return null;
  if (v.schema_version !== null && v.schema_version !== undefined && text(v.schema_version, 200) === null) return null;
  return {
    name: v.name,
    title,
    does,
    usedBy,
    address: v.address,
    token,
    tables,
    schemaVersion: (v.schema_version as string | null | undefined) ?? null,
  };
}

/** The list as the file gives it: the rows that are well formed, in
 * the order the file gives them, and how many were not. */
export function storesList(value: unknown): { checked: string | null; stores: StoreRow[]; malformed: number } {
  const v = value as { checked?: unknown; stores?: unknown } | null;
  const given = Array.isArray(v?.stores) ? v.stores : [];
  const seen = new Set<string>();
  const stores = given.map(storeRow).filter((row): row is StoreRow => {
    if (!row || seen.has(row.name)) return false;
    seen.add(row.name);
    return true;
  });
  return {
    checked: typeof v?.checked === "string" && /^\d{4}-\d{2}-\d{2}$/.test(v.checked) ? v.checked : null,
    stores,
    malformed: given.length - stores.length,
  };
}
