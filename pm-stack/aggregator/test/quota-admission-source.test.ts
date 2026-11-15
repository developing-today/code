import { describe, expect, test } from "bun:test";
import { buildQuotaAdmission, type Binding } from "../src/quota-admission.ts";
import { normalizeQuotaWindowEvidence, type QuotaWindowReceipt } from "../src/quota-admission-source.ts";

const now = Date.parse("2026-11-15T18:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();
const scope = { provider: "billing-door", account: "workspace-a", bucket: "plan" };
const receipt: QuotaWindowReceipt = {
  ...scope,
  kind: "weekly",
  period: "week-1",
  sourceProviderId: "quota-door",
  sourceCredentialId: "credential-a",
  sourceWindowName: "provider-weekly",
  source: "provider-source",
  id: "receipt-a",
  denominator: "allowance-1",
  measuredAt: now - 1_000,
  resetsAt: now + 60_000,
  percent: 0,
  windowSeconds: 604_800,
};
const window = {
  name: receipt.sourceWindowName,
  state: "ok",
  source: receipt.source,
  pctOfLimit: receipt.percent,
  windowSeconds: receipt.windowSeconds,
  measuredAt: iso(receipt.measuredAt),
  resetsAt: iso(receipt.resetsAt),
};
const row = {
  providerId: receipt.sourceProviderId,
  credentialId: receipt.sourceCredentialId,
  state: "ok",
  source: receipt.source,
  serviceId: "display-name",
  measuredAt: iso(now),
  windows: [window],
};
const document = { version: 1, generatedAt: iso(now), providers: [row] };
const check = (doc: unknown, read = receipt) => normalizeQuotaWindowEvidence(doc, read, now);
const binding: Binding = {
  ...scope,
  aliases: ["route-credential"],
  windows: [
    {
      kind: receipt.kind,
      period: receipt.period,
      resetsAt: receipt.resetsAt,
      freshnessMs: 20 * 60_000,
      collectionMs: 15 * 60_000,
    },
  ],
};

describe("quota-current admission source; oracle = exact provider window receipt (#10323)", () => {
  test("only identity and original observation fields survive, including genuine zero", () => {
    const result = check(document);
    expect(result).toEqual({
      reason: null,
      observation: {
        ...scope,
        kind: "weekly",
        period: "week-1",
        id: "receipt-a",
        source: "provider-source",
        measuredAt: receipt.measuredAt,
        resetsAt: receipt.resetsAt,
        percent: 0,
        state: "ok",
        denominator: "allowance-1",
      },
    });
    expect(check({ ...document, generatedAt: iso(now + 999_999) })).toEqual(result);
    expect(check(document, { ...receipt, denominator: "allowance-2" }).observation?.denominator).toBe("allowance-2");
  });

  test("exact duplicate split rows do not add observations; conflicts fail independent of order", () => {
    const split = { ...row, serviceId: "display-alias" };
    const conflict = { ...split, windows: [{ ...window, pctOfLimit: 50 }] };
    expect(check({ ...document, providers: [row, split] })).toEqual(check(document));
    for (const providers of [
      [row, conflict],
      [conflict, row],
    ]) {
      expect(check({ ...document, providers })).toEqual({ observation: null, reason: "conflicting_window_evidence" });
    }
  });

  test("a display alias, sibling credential or provider-wide row grants no evidence", () => {
    for (const patch of [{ providerId: "other" }, { credentialId: "other" }, { credentialId: undefined }]) {
      expect(check({ ...document, providers: [{ ...row, ...patch }] }).reason).toBe("missing_window_evidence");
    }
    expect(check({ ...document, providers: [{ ...row, windows: [] }] }).reason).toBe("missing_window_evidence");
  });

  test("failed or retained reads cannot reuse a healthy duplicate", () => {
    for (const patch of [{ state: "error" }, { state: "lapsed" }, { retainedFromFailedTick: iso(now) }]) {
      expect(check({ ...document, providers: [row, { ...row, ...patch }] }).reason).toBe("unavailable_provider_read");
    }
  });

  test("null percentages, untrusted source/time, expired mapping fields, and malformed files fail closed", () => {
    for (const patch of [
      { pctOfLimit: null },
      { state: "error" },
      { source: "display" },
      { windowSeconds: 10 },
      { measuredAt: iso(now) },
      { resetsAt: null },
      { resetsAt: "2026-11-31T18:01:00Z" },
    ])
      expect(
        check({ ...document, providers: [{ ...row, windows: [{ ...window, ...patch }] }] }).observation,
      ).toBeNull();
    for (const patch of [{ denominator: "" }, { id: "" }, { measuredAt: now + 1 }, { percent: 101 }]) {
      expect(check(document, { ...receipt, ...patch }).reason).toBe("invalid_receipt");
    }
    expect(check({ ...document, version: 2 }).reason).toBe("invalid_document");
    expect(check({ ...document, sanitized: { droppedProviders: 0, droppedWindows: 1 } }).reason).toBe(
      "incomplete_document",
    );
    expect(check({ ...document, providers: [{ ...row, windows: [window, null] }] }).reason).toBe("invalid_window_row");
  });

  test("projection uses the original stale window clock and preserves prior walls", () => {
    const measuredAt = now - 21 * 60_000;
    const read = check(
      { ...document, providers: [{ ...row, windows: [{ ...window, measuredAt: iso(measuredAt) }] }] },
      { ...receipt, measuredAt },
    ).observation;
    expect(read?.measuredAt).toBe(measuredAt);
    const input = { generation: "g", bindings: [binding], observations: read ? [read] : [], stops: [], priorWalls: [] };
    expect(buildQuotaAdmission(input, now).accounts[0].windows[0].reasons).toEqual(["stale_observation"]);
    const priorWalls = [
      {
        ...scope,
        kind: receipt.kind,
        period: receipt.period,
        resetsAt: receipt.resetsAt,
        evidenceId: "wall",
      },
    ];
    expect(buildQuotaAdmission({ ...input, observations: [], priorWalls }, now).accounts[0].verdict).toBe("walled");
  });
});
