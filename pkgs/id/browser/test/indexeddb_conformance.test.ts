// Shared store conformance evaluation for the IndexedDB profile
// (slice 9110-05, #10213) — evidence first, per the issue.
//
// Oracle (from the issue): IndexedDB either passes the required
// browser subset or produces an explicit unsupported-capability
// verdict; it never advertises a long-lived async transaction.
//
// This test mirrors the Rust conformance suite
// (`pkgs/id/tests/conformance.rs`, slice 9110-04) for the third
// profile: the same portable vectors from
// `pkgs/id/tests/fixtures/port-v1.json` are executed — here by the
// real adapter (`src/indexeddb_store.ts`) inside a real browser
// engine (system Chromium, driven over CDP with Node builtins
// only) — and every observation must agree three ways: the
// adapter's observation, the fixture's expectation, and an
// independent TypeScript reference model that follows only the
// fixture's declared outcomes (below). The script runs twice and
// the traces must be identical (determinism).
//
// #9082 adopts the browser request-batch profile. A divergent step has
// its own indexeddb expectation, checked against the real engine and
// the independent model. The public browser app need not enable it.

import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { accessSync, constants, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { stripTypeScriptTypes } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const PACKAGE_DIR = join(dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE_PATH = join(
  PACKAGE_DIR,
  "..",
  "tests",
  "fixtures",
  "port-v1.json",
);
const FIXTURE = JSON.parse(readFileSync(FIXTURE_PATH, "utf8"));

/** The verdict under evaluation: the indexeddb capability report
 * this prototype claims, in the fixture's report schema. */
const EXPECTED_REPORT = {
  resource_id: "indexeddb",
  kind: "record-store",
  contract_version: 2,
  scope: "browser",
  status: "active",
  required_capabilities: [],
  optional_capabilities: [
    "store.scan",
    "store.txn.multi-record",
    "store.txn.whole-database",
  ],
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

// ----------------------------------------------------------------
// Reference model — a direct mirror of the Rust `ReferenceModel`:
// ground truth keyed by (namespace, id), commits deduplicated by
// stable operation ID, following only declared outcomes.

interface Write {
  namespace: string;
  id: string;
  value: string | null;
}

class ReferenceModel {
  records = new Map<string, string>();
  appliedOps = new Set<string>();

  apply(opId: string, writes: Write[]): void {
    if (this.appliedOps.has(opId)) return;
    for (const write of writes) {
      const key = `${write.namespace}\0${write.id}`;
      if (write.value === null) this.records.delete(key);
      else this.records.set(key, write.value);
    }
    this.appliedOps.add(opId);
  }

  get(namespace: string, id: string): string | undefined {
    return this.records.get(`${namespace}\0${id}`);
  }

  scan(namespace: string): Array<[string, string]> {
    const entries: Array<[string, string]> = [];
    for (const [key, value] of this.records) {
      const [ns, id] = key.split("\0");
      if (ns === namespace) entries.push([id, value]);
    }
    entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return entries;
  }
}

function scanObservation(entries: Array<[string, string]>): string {
  return `entries:${entries.map(([id, v]) => `${id}=${v}`).join(",")}`;
}

function parseWrites(step: any): Write[] {
  if (step.writes) {
    return step.writes.map((write: any) => ({
      namespace: write.namespace,
      id: write.id,
      value: write.value ?? null,
    }));
  }
  return [
    {
      namespace: step.namespace,
      id: step.id,
      value: step.op === "put" ? step.value : null,
    },
  ];
}

function expectFor(step: any): string {
  if (typeof step.expect === "string") return step.expect;
  const value = step.expect?.indexeddb;
  assert.equal(typeof value, "string", "missing indexeddb fixture expectation");
  return value;
}

// ----------------------------------------------------------------
// Browser harness: a static server for the type-stripped modules
// plus headless Chromium over CDP. No third-party dependencies.

function findChromium(): string {
  const candidates = [
    process.env.CHROMIUM_BIN,
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
  ].filter((candidate): candidate is string => Boolean(candidate));
  for (const candidate of candidates) {
    if (candidate.includes("/")) {
      try {
        accessSync(candidate, constants.X_OK);
        return candidate;
      } catch {
        continue;
      }
    }
    for (const dir of (process.env.PATH ?? "").split(":")) {
      const path = join(dir, candidate);
      try {
        accessSync(path, constants.X_OK);
        return path;
      } catch {
        continue;
      }
    }
  }
  throw new Error(
    "no Chromium binary found (set CHROMIUM_BIN); the conformance verdict requires a real browser engine",
  );
}

function startServer(): Promise<{ base: string; close: () => void }> {
  const adapterJs = stripTypeScriptTypes(
    readFileSync(join(PACKAGE_DIR, "src", "indexeddb_store.ts"), "utf8"),
  );
  const driverJs = stripTypeScriptTypes(
    readFileSync(join(PACKAGE_DIR, "test", "browser_driver.ts"), "utf8"),
  ).replace("../src/indexeddb_store.ts", "/src/indexeddb_store.js");
  const server = createServer((request, response) => {
    if (request.url === "/") {
      response.setHeader("content-type", "text/html");
      response.end("<!doctype html><title>id conformance</title>");
    } else if (request.url === "/src/indexeddb_store.js") {
      response.setHeader("content-type", "text/javascript");
      response.end(adapterJs);
    } else if (request.url === "/test/browser_driver.js") {
      response.setHeader("content-type", "text/javascript");
      response.end(driverJs);
    } else {
      response.statusCode = 404;
      response.end("not found");
    }
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const address = server.address() as { port: number };
      resolve({
        base: `http://127.0.0.1:${address.port}`,
        close: () => server.close(),
      });
    });
  });
}

interface Cdp {
  send(method: string, params?: unknown, sessionId?: string): Promise<any>;
  waitEvent(method: string, sessionId?: string): Promise<any>;
  close(): void;
}

function connectCdp(wsUrl: string): Promise<Cdp> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(wsUrl);
    let nextId = 1;
    const pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
    const waiters = new Map<string, Array<(params: any) => void>>();
    socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id !== undefined) {
        const entry = pending.get(message.id);
        if (!entry) return;
        pending.delete(message.id);
        if (message.error) entry.reject(new Error(JSON.stringify(message.error)));
        else entry.resolve(message.result);
        return;
      }
      const key = `${message.sessionId ?? ""}:${message.method}`;
      const list = waiters.get(key) ?? [];
      waiters.delete(key);
      for (const waiter of list) waiter(message.params);
    };
    socket.onerror = () => reject(new Error("CDP websocket error"));
    socket.onopen = () => {
      resolve({
        send(method, params, sessionId) {
          const id = nextId++;
          return new Promise((sendResolve, sendReject) => {
            pending.set(id, { resolve: sendResolve, reject: sendReject });
            socket.send(
              JSON.stringify({ id, method, params: params ?? {}, sessionId }),
            );
          });
        },
        waitEvent(method, sessionId) {
          const key = `${sessionId ?? ""}:${method}`;
          return new Promise((eventResolve) => {
            const list = waiters.get(key) ?? [];
            list.push(eventResolve);
            waiters.set(key, list);
          });
        },
        close() {
          socket.close();
        },
      });
    };
  });
}

async function launchChromium(): Promise<{
  cdp: Cdp;
  process: ChildProcess;
  userDataDir: string;
}> {
  const userDataDir = mkdtempSync(join(tmpdir(), "id-conformance-"));
  const child = spawn(
    findChromium(),
    [
      "--headless",
      "--no-sandbox",
      "--disable-gpu",
      "--disable-dev-shm-usage",
      "--no-first-run",
      "--remote-debugging-port=0",
      `--user-data-dir=${userDataDir}`,
      "about:blank",
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  try {
    const wsUrl = await new Promise<string>((resolve, reject) => {
      let buffer = "";
      let settled = false;
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        reject(new Error("chromium did not print a DevTools URL in 30s"));
      }, 30_000);
      const fail = (error: Error) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        reject(error);
      };
      child.stderr?.on("data", (chunk) => {
        buffer += String(chunk);
        const match = buffer.match(/DevTools listening on (ws:\/\/\S+)/);
        if (match && !settled) {
          settled = true;
          clearTimeout(timer);
          resolve(match[1]);
        }
      });
      child.on("exit", (code) =>
        fail(new Error(`chromium exited early with code ${code}`)),
      );
    });
    const cdp = await connectCdp(wsUrl);
    return { cdp, process: child, userDataDir };
  } catch (error) {
    // A failed launch must not leak the browser or its profile:
    // the caller's cleanup only runs on a successful launch.
    child.kill("SIGKILL");
    rmSync(userDataDir, { recursive: true, force: true });
    throw error;
  }
}

async function runInEngine(base: string): Promise<any> {
  const { cdp, process: child, userDataDir } = await launchChromium();
  try {
    const { targetId } = await cdp.send("Target.createTarget", {
      url: "about:blank",
    });
    const { sessionId } = await cdp.send("Target.attachToTarget", {
      targetId,
      flatten: true,
    });
    await cdp.send("Page.enable", {}, sessionId);
    const loaded = cdp.waitEvent("Page.loadEventFired", sessionId);
    await cdp.send("Page.navigate", { url: `${base}/` }, sessionId);
    await loaded;
    const expression = `(async () => {
      const module = await import("/test/browser_driver.js");
      return await module.runInBrowser(${JSON.stringify(FIXTURE)});
    })()`;
    const result = await cdp.send(
      "Runtime.evaluate",
      { expression, awaitPromise: true, returnByValue: true },
      sessionId,
    );
    if (result.exceptionDetails) {
      throw new Error(
        `browser evaluation failed: ${JSON.stringify(result.exceptionDetails).slice(0, 2000)}`,
      );
    }
    return result.result.value;
  } finally {
    cdp.close();
    child.kill("SIGKILL");
    rmSync(userDataDir, { recursive: true, force: true });
  }
}

// ----------------------------------------------------------------

test("the shared fixture declares the adopted request-batch profile exactly once", () => {
  const reports = FIXTURE.capability_reports.filter(
    (entry: any) => entry.resource_id === "indexeddb",
  );
  assert.deepEqual(reports, [EXPECTED_REPORT]);
});

test(
  "indexeddb conformance: shared vectors + engine probes in Chromium",
  { timeout: 180_000 },
  async () => {
    const server = await startServer();
    let result: any;
    try {
      result = await runInEngine(server.base);
    } finally {
      server.close();
    }

    // Determinism: the script runs twice, traces identical.
    assert.deepEqual(
      result.traceRun2,
      result.traceRun1,
      "the vector script must be deterministic across runs",
    );

    // Three-way agreement per step: adapter observation, fixture
    // expectation, reference model.
    const model = new ReferenceModel();
    const vectors = FIXTURE.vectors as any[];
    assert.equal(result.traceRun1.length, vectors.length);
    vectors.forEach((step, index) => {
      const expect = expectFor(step);
      const observation = result.traceRun1[index];
      switch (step.op) {
        case "put":
        case "delete":
        case "commit_batch": {
          assert.equal(
            observation,
            expect,
            `step ${index} (${step.op}) diverged`,
          );
          if (expect === "committed") model.apply(step.op_id, parseWrites(step));
          break;
        }
        case "commit_fault": {
          assert.equal(
            observation,
            expect,
            `step ${index} (commit_fault) diverged`,
          );
          const resolved = observation.split("resolved:")[1];
          if (resolved === "committed") {
            model.apply(step.op_id, parseWrites(step));
          }
          break;
        }
        case "get": {
          const modelValue = model.get(step.namespace, step.id);
          const modelObservation =
            modelValue === undefined ? "absent" : `value:${modelValue}`;
          assert.equal(observation, modelObservation, `step ${index} (get) vs model`);
          assert.equal(observation, expect, `step ${index} (get) vs fixture`);
          break;
        }
        case "scan": {
          const modelObservation = scanObservation(
            model.scan(step.namespace),
          );
          assert.equal(observation, modelObservation, `step ${index} (scan) vs model`);
          assert.equal(observation, expect, `step ${index} (scan) vs fixture`);
          break;
        }
        case "negotiate": {
          assert.equal(
            observation,
            expect,
            `step ${index} (negotiate) diverged`,
          );
          break;
        }
        default:
          assert.fail(`unknown fixture op ${step.op}`);
      }
    });

    // Final state: the adapter's visible state equals the model's
    // in every namespace the script touched.
    const namespaces = [
      ...new Set(
        vectors
          .map((step) => step.namespace)
          .filter((ns): ns is string => typeof ns === "string"),
      ),
    ];
    for (const namespace of namespaces) {
      assert.equal(
        result.finalScans[namespace],
        scanObservation(model.scan(namespace)),
        `final state diverged in namespace ${namespace}`,
      );
    }

    // The verdict: the adapter's emitted report is exactly the
    // adopted declaration independently pinned by the shared fixture.
    assert.deepEqual(result.report, EXPECTED_REPORT);

    // Engine probes (the issue's named evaluation areas).
    assert.equal(
      result.probes.transactionLifetime,
      "InvalidStateError",
      "a request placed after its transaction's task ends must be refused by the engine — the fact the request-batch shape is built on",
    );
    assert.deepEqual(result.probes.abortedBatch, {
      error: "ConstraintError",
      afterAbort: [[], [["original", "taken"]]],
      afterRetry: [[["note", "kept"]], [["new", "free"], ["original", "taken"]]],
    });
    assert.equal(result.probes.reopenValue, "persisted-value");
    assert.deepEqual(result.probes.upgrade, {
      preserved: "v1-data",
      newStoreValue: "v2-data",
    });
    assert.equal(typeof result.probes.storage.quota, "number");
    assert.ok(result.probes.storage.quota > 0);
    assert.equal(typeof result.probes.storage.persisted, "boolean");
    assert.equal(
      result.probes.metaNamespace,
      "refused:unsupported-capability",
      "the internal operation-ID store must not be openable as a user namespace",
    );

    // The evidence record: engine, verdict, probes.
    console.log(
      JSON.stringify(
        { engine: result.engine, report: result.report, probes: result.probes },
        null,
        2,
      ),
    );
  },
);
