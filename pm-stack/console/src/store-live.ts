// One store, asked live (#14254).
//
// The Platform screen's Stores section: for each store the list of
// record names (stores.ts), the store itself is asked over its HTTP
// API — does it answer, how big it is, how many rows are in each table
// the app reads, and its schema version. This file holds the asking's
// shape: the statements, and how an answer (or no answer) is read.
// The asking itself — a token fetched by name, a deadline, one retry
// after a rotation — is Ops.platformStores, on the pattern the queue
// store's pipeline already uses. Reads only: every statement here is
// a question, and nothing in this slice changes any store.
//
// A store that does not answer is said as not answering, with the
// error in plain words. Its size, rows and version are then null —
// never a stale success carried over from an earlier asking.
import type { StoreRow } from "./stores.ts";

export interface StoreLive extends StoreRow {
  /** Did the store itself answer this asking? */
  answers: boolean;
  /** Why it did not answer, said plainly; null when it did. */
  error: string | null;
  /** The database's size in bytes (its pages, at their size), or
   * null when it did not answer. */
  sizeBytes: number | null;
  /** Rows in each table the app reads, by the table's name; a table
   * the store refused (or that is not there) is null, said apart
   * from a count of zero. Empty when the list names no tables. */
  rows: Record<string, number | null>;
  /** The schema version the store itself reports (its
   * `user_version`); where the store reports none, what the list
   * records of its schema version; null when neither says one. */
  liveSchemaVersion: string | null;
}

/** The statements one asking carries, in order: the size, one count
 * per table the app reads, and the schema version. Table names come
 * from the list of record, whose parser only lets a plain table name
 * through (stores.ts), so naming one here names nothing else. */
export function storeLiveStatements(store: StoreRow): { sql: string; args: (string | number)[] }[] {
  return [
    { sql: "SELECT page_count * page_size AS size FROM pragma_page_count(), pragma_page_size()", args: [] },
    ...store.tables.map((table) => ({ sql: `SELECT count(*) AS n FROM "${table}"`, args: [] })),
    { sql: "SELECT user_version AS v FROM pragma_user_version()", args: [] },
  ];
}

type Cell = { type?: string; value?: string | number | null } | undefined;

/** A whole number as a store sends it (see fromStore in ops.ts). */
function whole(cell: Cell): number | null {
  const value = cell?.value ?? null;
  if (typeof value === "number") return Number.isSafeInteger(value) ? value : null;
  if (typeof value === "string" && /^-?[0-9]+$/.test(value)) {
    const n = Number(value);
    return Number.isSafeInteger(n) ? n : null;
  }
  return null;
}

export type StoreResult = { cols: { name: string }[]; rows: Cell[][] } | undefined;

/** A store that did not answer: every live field null, the error
 * said plainly. */
export function storeNotAnswering(store: StoreRow, error: string): StoreLive {
  return {
    ...store,
    answers: false,
    error,
    sizeBytes: null,
    rows: Object.fromEntries(store.tables.map((t) => [t, null])),
    liveSchemaVersion: null,
  };
}

/** Read one asking's results, in the order storeLiveStatements
 * asked them. A statement the store refused leaves only its own
 * field null; the store still answered. */
export function storeAnswered(store: StoreRow, results: StoreResult[]): StoreLive {
  const rows: Record<string, number | null> = {};
  store.tables.forEach((table, i) => {
    rows[table] = whole(results[1 + i]?.rows[0]?.[0]);
  });
  const version = whole(results[1 + store.tables.length]?.rows[0]?.[0]);
  return {
    ...store,
    answers: true,
    error: null,
    sizeBytes: whole(results[0]?.rows[0]?.[0]),
    rows,
    liveSchemaVersion: version === null ? store.schemaVersion : String(version),
  };
}
