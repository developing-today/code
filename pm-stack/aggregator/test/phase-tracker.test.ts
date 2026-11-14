// #9923: tests for the Phase tracker panel's pure core — the
// derivation of per-phase validation state from live GhItem states.
// Fixture titles reproduce the four audit issues' real titles
// (verified against live GitHub on 2026-11-14), since the phase
// mapping comes from those titles.
import { describe, expect, test } from "bun:test";
import {
  PHASE_TRACKER_REPO,
  derivePhaseValidation,
  openFollowUpsFor,
} from "../src/phase-tracker.ts";
import type { GhItem } from "../src/model.ts";

const item = (number: number, state: GhItem["state"], title?: string): GhItem => ({
  ref: `${PHASE_TRACKER_REPO}#${number}`,
  kind: "issue",
  url: `https://github.com/${PHASE_TRACKER_REPO}/issues/${number}`,
  title: title ?? `live title ${number}`,
  state,
  labels: [],
  author: "dezren39",
  updatedAt: "2026-11-14T00:00:00Z",
});

const AUDIT_TITLES: Record<number, string> = {
  1075: "id: validation — Phases 0–8 + platform completion acceptance audit",
  1076: "id: validation — Phase 9 portable platform",
  1077: "id: validation — Phase 10 worlds as places",
  1078: "id: validation — delivered Phase 11 items (claimed-done audit)",
};

function allClosed(): Map<string, GhItem> {
  return new Map(
    [1075, 1076, 1077, 1078].map((n) => [
      `${PHASE_TRACKER_REPO}#${n}`,
      item(n, "closed", AUDIT_TITLES[n]),
    ]),
  );
}

describe("derivePhaseValidation (#9923)", () => {
  test("all audits closed: phases 0-11 validated, 12-13 have no audit", () => {
    const rows = derivePhaseValidation(allClosed());
    expect(rows.map((r) => r.phase)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
    for (const r of rows.slice(0, 12)) expect(r.state).toBe("validated");
    expect(rows[12].state).toBe("no-audit");
    expect(rows[13].state).toBe("no-audit");
    expect(rows[12].auditIssue).toBeNull();
  });

  test("phase-to-audit mapping follows the audit titles", () => {
    const rows = derivePhaseValidation(allClosed());
    for (let p = 0; p <= 8; p++) expect(rows[p].auditIssue).toBe(1075);
    expect(rows[9].auditIssue).toBe(1076);
    expect(rows[10].auditIssue).toBe(1077);
    expect(rows[11].auditIssue).toBe(1078);
    expect(rows[9].auditTitle).toBe(AUDIT_TITLES[1076]);
  });

  test("a reopened validation issue flips only its phases to validation-open", () => {
    const items = allClosed();
    items.set(`${PHASE_TRACKER_REPO}#1077`, item(1077, "open", AUDIT_TITLES[1077]));
    const rows = derivePhaseValidation(items);
    expect(rows[10].state).toBe("validation-open");
    expect(rows[10].auditState).toBe("open");
    for (const r of rows.slice(0, 10)) expect(r.state).toBe("validated");
    expect(rows[11].state).toBe("validated");
  });

  test("an audit missing from the source renders unresolved, never invented", () => {
    const items = allClosed();
    items.delete(`${PHASE_TRACKER_REPO}#1075`);
    const rows = derivePhaseValidation(items);
    for (let p = 0; p <= 8; p++) {
      expect(rows[p].state).toBe("unresolved");
      expect(rows[p].auditState).toBeNull();
      expect(rows[p].auditIssue).toBe(1075);
    }
    expect(rows[9].state).toBe("validated");
  });

  test("open follow-ups referencing an audit surface on its phases; closed ones do not", () => {
    const items = allClosed();
    items.set(
      `${PHASE_TRACKER_REPO}#1501`,
      item(1501, "open", "id: follow-up to #1076 — recheck portable platform claim"),
    );
    items.set(
      `${PHASE_TRACKER_REPO}#1502`,
      item(1502, "closed", "id: follow-up to #1076 — already settled"),
    );
    const rows = derivePhaseValidation(items);
    expect(rows[9].openFollowUps.map((f) => f.issueNumber)).toEqual([1501]);
    expect(rows[9].state).toBe("validated");
    expect(rows[10].openFollowUps).toEqual([]);
    expect(openFollowUpsFor(1076, items).length).toBe(1);
    expect(openFollowUpsFor(1075, items)).toEqual([]);
  });

  test("an audit never counts as its own follow-up, even when open", () => {
    const items = allClosed();
    items.set(`${PHASE_TRACKER_REPO}#1076`, item(1076, "open", AUDIT_TITLES[1076]));
    const rows = derivePhaseValidation(items);
    expect(rows[9].openFollowUps).toEqual([]);
    expect(rows[9].state).toBe("validation-open");
  });
});
