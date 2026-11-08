import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  renewMetaSession,
  type MetaExporterDoc,
  type MetaRenewalState,
} from "./meta-renewal.ts";

const NOW = new Date("2026-11-05T16:45:00.000Z");
const fresh = (minsAgo: number): string =>
  new Date(NOW.getTime() - minsAgo * 60_000).toISOString();

async function statePath(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "meta-renewal-"));
  return join(dir, "meta-renewal.json");
}

const okDoc = (minsAgo = 5): MetaExporterDoc => ({
  status: "ok",
  measuredAt: fresh(minsAgo),
  attemptedAt: fresh(minsAgo),
});
const expiredDoc = (): MetaExporterDoc => ({
  status: "session-expired",
  measuredAt: null,
  attemptedAt: fresh(1),
});

describe("meta session keeper", () => {
  test("a fresh ok exporter document is a valid session, no recovery attempted", async () => {
    let ran = false;
    const state = await renewMetaSession({
      statePath: await statePath(),
      now: () => NOW,
      readLatest: async () => okDoc(),
      ensureExporterArmed: async () => "armed",
      runExporter: async () => {
        ran = true;
      },
    });
    expect(state.status).toBe("valid");
    expect(state.recovery.attempted).toBe(false);
    expect(state.alert).toBeNull();
    expect(state.lastGoodAt).toBe(fresh(5));
    expect(ran).toBe(false);
  });

  test("an expired session that a fresh probe fixes is recovered", async () => {
    let probes = 0;
    const state = await renewMetaSession({
      statePath: await statePath(),
      now: () => NOW,
      readLatest: async () => (++probes === 1 ? expiredDoc() : okDoc(0)),
      ensureExporterArmed: async () => "armed",
      runExporter: async () => undefined,
    });
    expect(state.status).toBe("recovered");
    expect(state.recovery).toEqual({ attempted: true, result: "recovered" });
    expect(state.alert).toBeNull();
  });

  test("units restored from .backup during recovery are reported as such", async () => {
    let probes = 0;
    const state = await renewMetaSession({
      statePath: await statePath(),
      now: () => NOW,
      readLatest: async () => (++probes === 1 ? expiredDoc() : okDoc(0)),
      ensureExporterArmed: async () => "restored",
      runExporter: async () => undefined,
    });
    expect(state.status).toBe("recovered");
    expect(state.recovery.result).toBe("recovered-units-restored");
  });

  test("a still-expired session raises an alert from the first bad tick, and alertSince persists", async () => {
    const path = await statePath();
    const deps = {
      statePath: path,
      readLatest: async () => expiredDoc(),
      ensureExporterArmed: async () => "armed" as const,
      runExporter: async () => undefined,
    };
    const first = await renewMetaSession({ ...deps, now: () => NOW });
    expect(first.status).toBe("attention");
    expect(first.alert).toContain("emails a login code");
    expect(first.alertSince).toBe(NOW.toISOString());

    const later = new Date(NOW.getTime() + 15 * 60_000);
    const second = await renewMetaSession({ ...deps, now: () => later });
    expect(second.status).toBe("attention");
    expect(second.alertSince).toBe(NOW.toISOString());

    const onDisk = JSON.parse(await readFile(path, "utf8")) as MetaRenewalState;
    expect(onDisk.provider).toBe("meta");
    expect(onDisk.alertSince).toBe(NOW.toISOString());
  });

  test("a stale ok document (exporter silent) is attention, not valid", async () => {
    const state = await renewMetaSession({
      statePath: await statePath(),
      now: () => NOW,
      readLatest: async () => okDoc(120),
      ensureExporterArmed: async () => "armed",
      runExporter: async () => undefined,
    });
    expect(state.status).toBe("attention");
    expect(state.lastGoodAt).toBe(fresh(120));
    expect(state.alert).toContain("not producing fresh readings");
  });

  test("missing exporter units produce the units-missing alert", async () => {
    const state = await renewMetaSession({
      statePath: await statePath(),
      now: () => NOW,
      readLatest: async () => null,
      ensureExporterArmed: async () => "missing",
      runExporter: async () => undefined,
    });
    expect(state.status).toBe("attention");
    expect(state.recovery.result).toBe("exporter-missing");
    expect(state.alert).toContain("units");
  });
});
