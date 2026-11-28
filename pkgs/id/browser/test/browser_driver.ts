/**
 * Page-side driver for the IndexedDB conformance evaluation
 * (#10213). This module is served to a real browser engine by
 * `indexeddb_conformance.test.ts` (type-stripped, with the
 * `.ts` import specifier rewritten to the served `.js` URL); it is
 * harness glue, not part of the adapter under test.
 *
 * It executes the shared port-v1 fixture vectors against
 * `IndexedDbStore` and returns raw observations in exactly the
 * string forms the Rust state machine uses
 * (`pkgs/id/tests/conformance.rs`), plus the IndexedDB-specific
 * probes the issue names: transaction lifetime, upgrade, reopen,
 * and quota/eviction. Expectation checking lives on the Node side.
 */

import {
  INDEXEDDB_DESCRIPTOR,
  IndexedDbStore,
  PortRefusal,
  negotiate,
  resolveOutcome,
} from "../src/indexeddb_store.ts";

interface FixtureStep {
  op: string;
  namespace?: string;
  id?: string;
  value?: string;
  op_id?: string;
  writes?: Array<{ namespace: string; id: string; value: string }>;
  required?: string[];
  protocol_version?: number;
  interactive?: boolean;
  fault?: string;
}

function parseWrites(step: FixtureStep) {
  if (step.writes) {
    return step.writes.map((write) => ({
      namespace: write.namespace,
      id: write.id,
      value: write.value ?? null,
    }));
  }
  return [
    {
      namespace: step.namespace as string,
      id: step.id as string,
      value: step.op === "put" ? (step.value as string) : null,
    },
  ];
}

function commitObservation(error: unknown): string {
  if (error instanceof PortRefusal) return `refused:${error.kind}`;
  throw error;
}

async function runVectors(
  fixture: { vectors: FixtureStep[] },
  runTag: string,
): Promise<{ trace: string[]; finalScans: Record<string, string> }> {
  const namespaces = ["groups", "tags"];
  const dbName = `id-conformance-${runTag}`;
  await IndexedDbStore.deleteDatabase(dbName);
  const store = await IndexedDbStore.open(dbName, namespaces);
  const trace: string[] = [];
  try {
    for (const step of fixture.vectors) {
      switch (step.op) {
        case "put":
        case "delete":
        case "commit_batch": {
          let observation: string;
          try {
            observation = await store.commitBatch(step.op_id as string, parseWrites(step));
          } catch (error) {
            observation = commitObservation(error);
          }
          trace.push(observation);
          break;
        }
        case "commit_fault": {
          // Faults are inert for a definitive (local) profile: the
          // commit call's return is the outcome. The fault field is
          // accepted and the batch commits normally, exactly as the
          // Rust state machine models drop-response / drop-request
          // for the redb profile.
          const reported = await store.commitBatch(step.op_id as string, parseWrites(step));
          trace.push(`reported:${reported},resolved:${resolveOutcome(reported)}`);
          break;
        }
        case "get": {
          const value = await store.get(step.namespace as string, step.id as string);
          trace.push(value === undefined ? "absent" : `value:${value}`);
          break;
        }
        case "scan": {
          const entries = await store.scan(step.namespace as string);
          trace.push(`entries:${entries.map(([id, v]) => `${id}=${v}`).join(",")}`);
          break;
        }
        case "negotiate": {
          try {
            negotiate(step.required ?? [], step.protocol_version ?? 2);
            trace.push("granted");
          } catch (error) {
            trace.push(commitObservation(error));
          }
          break;
        }
        default:
          throw new Error(`unknown fixture op ${step.op}`);
      }
    }
    const finalScans: Record<string, string> = {};
    for (const namespace of namespaces) {
      const entries = await store.scan(namespace);
      finalScans[namespace] = `entries:${entries.map(([id, v]) => `${id}=${v}`).join(",")}`;
    }
    return { trace, finalScans };
  } finally {
    store.close();
    await IndexedDbStore.deleteDatabase(dbName);
  }
}

/**
 * The transaction-lifetime probe: a request placed after the
 * transaction's task has ended must be refused by the engine
 * (an InvalidStateError DOMException, per the IndexedDB spec's
 * "transaction is not active" failure). This is the behavior the
 * adapter's request-batch shape exists to never trigger.
 */
async function probeTransactionLifetime(): Promise<string> {
  const dbName = "id-probe-lifetime";
  await IndexedDbStore.deleteDatabase(dbName);
  const store = await IndexedDbStore.open(dbName, ["kv"]);
  store.close();
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(dbName, 1);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    const transaction = db.transaction("kv", "readwrite");
    // Let the creating task end with no request placed.
    await new Promise((resolve) => setTimeout(resolve, 0));
    try {
      transaction.objectStore("kv").put("late", "k");
      return "no-error";
    } catch (error) {
      return (error as DOMException).name;
    }
  } finally {
    db.close();
    await IndexedDbStore.deleteDatabase(dbName);
  }
}

// Give one test database a genuine engine constraint. The second write in
// the batch conflicts after a write in another namespace has been queued.
// Retrying the same operation ID also checks that its dedupe marker rolled back.
async function probeAbortedBatch() {
  const name = "id-probe-atomic-abort";
  await IndexedDbStore.deleteDatabase(name);
  const seed = await new Promise<IDBDatabase>((resolve, reject) => {
    const open = indexedDB.open(name, 1);
    open.onupgradeneeded = () => {
      open.result.createObjectStore("notes");
      open.result.createObjectStore("tags").createIndex("unique-value", "", { unique: true });
    };
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error);
  });
  seed.close();
  const store = await IndexedDbStore.open(name, ["notes", "tags"], 2);
  try {
    await store.commitBatch("setup", [{ namespace: "tags", id: "original", value: "taken" }]);
    const note = { namespace: "notes", id: "note", value: "kept" };
    const tag = { namespace: "tags", id: "new", value: "taken" };
    let error = "committed";
    try {
      await store.commitBatch("batch", [note, tag]);
    } catch (cause) {
      if (!(cause instanceof DOMException)) throw cause;
      error = cause.name;
    }
    const afterAbort = [await store.scan("notes"), await store.scan("tags")];
    await store.commitBatch("batch", [note, { ...tag, value: "free" }]);
    const afterRetry = [await store.scan("notes"), await store.scan("tags")];
    return { error, afterAbort, afterRetry };
  } finally {
    store.close();
    await IndexedDbStore.deleteDatabase(name);
  }
}

async function probeReopen(): Promise<string | undefined> {
  const dbName = "id-probe-reopen";
  await IndexedDbStore.deleteDatabase(dbName);
  let store = await IndexedDbStore.open(dbName, ["kv"]);
  await store.commitBatch("probe-reopen-1", [{ namespace: "kv", id: "k", value: "persisted-value" }]);
  store.close();
  store = await IndexedDbStore.open(dbName, ["kv"]);
  try {
    return await store.get("kv", "k");
  } finally {
    store.close();
    await IndexedDbStore.deleteDatabase(dbName);
  }
}

async function probeUpgrade(): Promise<{
  preserved: string | undefined;
  newStoreValue: string | undefined;
}> {
  const dbName = "id-probe-upgrade";
  await IndexedDbStore.deleteDatabase(dbName);
  let store = await IndexedDbStore.open(dbName, ["a"], 1);
  await store.commitBatch("probe-upgrade-1", [{ namespace: "a", id: "k", value: "v1-data" }]);
  store.close();
  store = await IndexedDbStore.open(dbName, ["a", "b"], 2);
  try {
    const preserved = await store.get("a", "k");
    await store.commitBatch("probe-upgrade-2", [{ namespace: "b", id: "k", value: "v2-data" }]);
    const newStoreValue = await store.get("b", "k");
    return { preserved, newStoreValue };
  } finally {
    store.close();
    await IndexedDbStore.deleteDatabase(dbName);
  }
}

/**
 * The reserved-namespace probe: the internal operation-ID store
 * is refused as a user namespace at open time, so user records
 * can never share the dedupe table.
 */
async function probeMetaNamespace(): Promise<string> {
  try {
    const store = await IndexedDbStore.open("id-probe-meta", ["__port_meta"]);
    store.close();
    return "opened";
  } catch (error) {
    if (error instanceof PortRefusal) return `refused:${error.kind}`;
    throw error;
  }
}

async function probeStorage(): Promise<{
  usage: number | null;
  quota: number | null;
  persisted: boolean | null;
}> {
  try {
    const estimate = await navigator.storage.estimate();
    const persisted = await navigator.storage.persisted();
    return {
      usage: estimate.usage ?? null,
      quota: estimate.quota ?? null,
      persisted,
    };
  } catch {
    return { usage: null, quota: null, persisted: null };
  }
}

export async function runInBrowser(fixture: { vectors: FixtureStep[] }): Promise<unknown> {
  const run1 = await runVectors(fixture, "run1");
  const run2 = await runVectors(fixture, "run2");
  return {
    engine: navigator.userAgent,
    traceRun1: run1.trace,
    traceRun2: run2.trace,
    finalScans: run1.finalScans,
    report: { ...INDEXEDDB_DESCRIPTOR, status: "active" },
    probes: {
      transactionLifetime: await probeTransactionLifetime(),
      reopenValue: await probeReopen(),
      abortedBatch: await probeAbortedBatch(),
      upgrade: await probeUpgrade(),
      storage: await probeStorage(),
      metaNamespace: await probeMetaNamespace(),
    },
  };
}
