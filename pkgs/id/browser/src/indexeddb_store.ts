/**
 * IndexedDB request-batch record-store adapter (port contract version 2).
 * #10213 evaluated it in Chromium; #9082 adopted that profile. The web
 * product still chooses its own store. This adapter is opt-in.
 *
 * The one structural rule the evaluation exists to prove: this
 * adapter never advertises — and cannot express — a long-lived
 * async transaction. Every operation is a *request batch*: the
 * caller hands over the full write set up front, and the adapter
 * places every request synchronously inside a single IndexedDB
 * transaction. No transaction handle, cursor, or request ever
 * escapes to the caller, so there is no way to hold a transaction
 * open across an `await` (an IndexedDB transaction goes inactive
 * as soon as the task that placed its last request ends; the
 * conformance suite demonstrates the engine's refusal of a late
 * request explicitly).
 *
 * Semantics mirror the Rust port contract
 * (`pkgs/id/crates/id-core/src/port.rs`,
 * `pkgs/id/crates/id-store/src/port_adapter.rs`) and the shared
 * conformance fixtures (`pkgs/id/tests/fixtures/port-v1.json`):
 * commits are deduplicated by stable operation ID inside the same
 * transaction as the writes, the commit outcome is definitive (a
 * local transaction's completion is known to the caller, never
 * unknown-until-readback), and a lost commit is retried as a new
 * operation under the same operation ID.
 */

export const PORT_CONTRACT_VERSION = 2;

/** The meta object store holding applied operation IDs. */
const META_STORE = "__port_meta";

export interface StoreWrite {
  namespace: string;
  id: string;
  /** `null` deletes the record. */
  value: string | null;
}

export type RefusalKind = "unsupported-capability" | "version-mismatch" | "conditional" | "limit-exceeded";

/** A contract refusal, mirroring `PortError` in the Rust port. */
export class PortRefusal extends Error {
  readonly kind: RefusalKind;
  constructor(kind: RefusalKind, message: string) {
    super(message);
    this.name = "PortRefusal";
    this.kind = kind;
  }
}

export interface IndexedDbDescriptor {
  resource_id: string;
  kind: string;
  contract_version: number;
  scope: string;
  required_capabilities: string[];
  optional_capabilities: string[];
  guarantees: {
    ordered: boolean;
    durable: boolean;
    atomic_commit: boolean;
    atomic_outbox: boolean;
    exactly_once: boolean;
  };
  limits: {
    max_record_bytes: number | null;
    max_batch_records: number | null;
    max_message_bytes: number | null;
  };
  atomic_scope: string;
  conditional_atomic_scope: { scope: string; condition: string } | null;
  commit_ambiguity: string;
  retry_rule: string;
}

/**
 * The capability declaration this prototype stands behind, in the
 * fixture's report schema. Honesty notes:
 *
 * - `atomic_scope: "whole-database"` is per request batch: one
 *   transaction may span every object store of the database, so a
 *   batch commits or aborts as a unit. It is *not* a session
 *   transaction — no capability here names one.
 * - `durable: true` means data survives closing and reopening the database (the
 *   conformance suite proves it by closing and reopening the
 *   database). Browser storage is best-effort under storage
 *   pressure unless the origin is granted persistence
 *   (`navigator.storage.persist()`); the suite records the
 *   engine's persistence state with the verdict rather than
 *   claiming more than the engine grants.
 * - `commit_ambiguity: "definitive"`: a local transaction reports
 *   its own outcome; network-style faults are inert for this
 *   profile, exactly as the Rust state machine models them for a
 *   definitive profile.
 */
export const INDEXEDDB_DESCRIPTOR: IndexedDbDescriptor = {
  resource_id: "indexeddb",
  kind: "record-store",
  contract_version: PORT_CONTRACT_VERSION,
  scope: "browser",
  required_capabilities: [],
  optional_capabilities: ["store.scan", "store.txn.multi-record", "store.txn.whole-database"],
  guarantees: {
    ordered: false,
    durable: true,
    atomic_commit: true,
    atomic_outbox: false,
    exactly_once: false,
  },
  limits: {
    max_record_bytes: null,
    max_batch_records: null,
    max_message_bytes: null,
  },
  atomic_scope: "whole-database",
  conditional_atomic_scope: null,
  commit_ambiguity: "definitive",
  retry_rule: "retry-as-new-operation",
};

function descriptorAdvertises(capability: string): boolean {
  return (
    INDEXEDDB_DESCRIPTOR.required_capabilities.includes(capability) ||
    INDEXEDDB_DESCRIPTOR.optional_capabilities.includes(capability)
  );
}

/**
 * Capability negotiation, mirroring `negotiate` in the Rust port:
 * the protocol version is checked first, then every required
 * capability must be advertised. Refusals happen before any write.
 */
export function negotiate(requiredCapabilities: string[], protocolVersion: number = PORT_CONTRACT_VERSION): string[] {
  if (protocolVersion !== INDEXEDDB_DESCRIPTOR.contract_version) {
    throw new PortRefusal(
      "version-mismatch",
      `protocol version ${protocolVersion} does not match declared contract version ${INDEXEDDB_DESCRIPTOR.contract_version}`,
    );
  }
  for (const capability of requiredCapabilities) {
    if (!descriptorAdvertises(capability)) {
      throw new PortRefusal(
        "unsupported-capability",
        `capability ${capability} is not advertised by the indexeddb profile`,
      );
    }
  }
  return [...requiredCapabilities];
}

type AtomicScope = "single-record" | "single-namespace" | "whole-database";

/** The scope a write set spans, by the profiles' own terms. */
export function scopeOf(writes: StoreWrite[]): AtomicScope {
  const namespaces = new Set(writes.map((write) => write.namespace));
  if (namespaces.size > 1) return "whole-database";
  if (writes.length > 1) return "single-namespace";
  return "single-record";
}

const SCOPE_ORDER: Record<AtomicScope, number> = {
  "single-record": 0,
  "single-namespace": 1,
  "whole-database": 2,
};

/**
 * The pre-write commit gate, mirroring `StoreProfile::check_commit`:
 * a batch whose scope exceeds the declared atomic scope is refused
 * before any request is placed. This profile declares whole-database
 * scope unconditionally, so conforming batches always pass; the gate
 * exists so the declaration is enforced, not just documented.
 */
export function checkCommit(writes: StoreWrite[]): void {
  const needed = scopeOf(writes);
  const declared = INDEXEDDB_DESCRIPTOR.atomic_scope as AtomicScope;
  if (SCOPE_ORDER[needed] > SCOPE_ORDER[declared]) {
    throw new PortRefusal(
      "conditional",
      `batch scope ${needed} exceeds the indexeddb profile's declared atomic scope ${declared}`,
    );
  }
  if (needed === "whole-database") {
    negotiate(["store.txn.whole-database"]);
  } else if (needed === "single-namespace") {
    negotiate(["store.txn.multi-record"]);
  }
}

/**
 * Outcome resolution, mirroring `StoreProfile::resolve_outcome`:
 * a definitive profile's reported outcome is final.
 */
export function resolveOutcome(reported: string): string {
  return reported;
}

function requestDone<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
  });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB transaction aborted"));
    transaction.onerror = () => {
      // A request error aborts the transaction; onabort reports it.
    };
  });
}

/**
 * The request-batch adapter. One instance owns one database connection;
 * namespaces are object stores created at open (upgrade) time.
 */
export class IndexedDbStore {
  private db: IDBDatabase;
  private readonly namespaces: string[];

  private constructor(db: IDBDatabase, namespaces: string[]) {
    this.db = db;
    this.namespaces = namespaces;
  }

  /**
   * Open (creating on first use) the database at `version`,
   * ensuring every named namespace exists as an object store.
   * Reopening with a higher version and additional namespaces is
   * the upgrade path; existing stores and records are preserved.
   */
  static async open(name: string, namespaces: string[], version = 1): Promise<IndexedDbStore> {
    if (namespaces.includes(META_STORE)) {
      throw new PortRefusal("unsupported-capability", `namespace ${META_STORE} is reserved for operation-ID records`);
    }
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(name, version);
      request.onupgradeneeded = () => {
        const database = request.result;
        for (const namespace of namespaces) {
          if (!database.objectStoreNames.contains(namespace)) {
            database.createObjectStore(namespace);
          }
        }
        if (!database.objectStoreNames.contains(META_STORE)) {
          database.createObjectStore(META_STORE);
        }
      };
      let settled = false;
      request.onsuccess = () => {
        if (settled) {
          // The open was already rejected (blocked): rejecting the
          // promise did not cancel the pending request, so close
          // the late connection instead of leaking it.
          request.result.close();
          return;
        }
        settled = true;
        resolve(request.result);
      };
      request.onerror = () => {
        if (settled) return;
        settled = true;
        reject(request.error ?? new Error("IndexedDB open failed"));
      };
      request.onblocked = () => {
        if (settled) return;
        settled = true;
        reject(new Error("IndexedDB open blocked by another connection"));
      };
    });
    return new IndexedDbStore(db, namespaces);
  }

  /** Delete a database outright (evaluation cleanup). */
  static async deleteDatabase(name: string): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      const request = indexedDB.deleteDatabase(name);
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error ?? new Error("IndexedDB delete failed"));
      // onblocked is informational: the delete proceeds once the
      // remaining connections close; it is not a failure.
    });
  }

  close(): void {
    this.db.close();
  }

  private assertNamespace(namespace: string): void {
    if (!this.namespaces.includes(namespace)) {
      throw new PortRefusal("unsupported-capability", `namespace ${namespace} was not declared at open time`);
    }
  }

  /**
   * Commit one request batch atomically. All requests — the
   * operation-ID dedupe probe, the writes, and the operation-ID
   * record — are placed inside a single transaction; the writes
   * are placed from the dedupe request's success callback, while
   * the transaction is still active, never after an `await` of
   * anything but that transaction's own requests. A batch whose
   * operation ID was already applied commits as a no-op
   * (idempotent retry under the retry-as-new-operation rule).
   * Resolves `"committed"` when the transaction completes; rejects
   * if it aborts.
   */
  async commitBatch(opId: string, writes: StoreWrite[]): Promise<"committed"> {
    checkCommit(writes);
    for (const write of writes) this.assertNamespace(write.namespace);
    const stores = [...new Set([...writes.map((write) => write.namespace), META_STORE])];
    const transaction = this.db.transaction(stores, "readwrite");
    const done = transactionDone(transaction);
    const meta = transaction.objectStore(META_STORE);
    const probe = meta.get(opId);
    probe.onsuccess = () => {
      if (probe.result !== undefined) return; // Already applied.
      for (const write of writes) {
        const store = transaction.objectStore(write.namespace);
        if (write.value === null) store.delete(write.id);
        else store.put(write.value, write.id);
      }
      meta.put(true, opId);
    };
    await done;
    return "committed";
  }

  /** Read one record in its own single-request transaction. */
  async get(namespace: string, id: string): Promise<string | undefined> {
    this.assertNamespace(namespace);
    const transaction = this.db.transaction(namespace, "readonly");
    const value = await requestDone(transaction.objectStore(namespace).get(id));
    return value as string | undefined;
  }

  /**
   * Scan one namespace in key order. Both requests are placed
   * synchronously, before either is awaited.
   */
  async scan(namespace: string): Promise<Array<[string, string]>> {
    this.assertNamespace(namespace);
    const transaction = this.db.transaction(namespace, "readonly");
    const store = transaction.objectStore(namespace);
    const keysRequest = store.getAllKeys();
    const valuesRequest = store.getAll();
    const keys = await requestDone(keysRequest);
    const values = await requestDone(valuesRequest);
    return keys.map((key, index) => [String(key), values[index] as string]);
  }
}
