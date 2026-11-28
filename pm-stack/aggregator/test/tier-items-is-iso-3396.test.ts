// #3396: tier-items isIso must refuse impossible calendar dates.
// Date.parse normalizes 2026-02-30 to Mar 2 instead of failing, so
// sanitizeDelegation used to keep delegation.updatedAt, proposal.at
// and unable.at carrying days that never existed — and proposalExpired
// clocked the 48h proposal TTL from the phantom day. The sanitizer now
// re-checks the calendar day with validate.ts parseIsoDate strictness.
// Fail-soft contract is unchanged: a refused timestamp drops the whole
// delegation, never the item.
import { describe, expect, test } from "bun:test";
import type { TierItem } from "../src/model.ts";
import { proposalExpired, sanitizeTierItemExtensions } from "../src/tier-items.ts";

const BASE: TierItem = {
  id: "developing-today/code#3396",
  kind: "decision",
  title: "Impossible-date delegation timestamps",
  source: "backlog",
  reason: "A scope decision only the user can make.",
  since: "2026-10-26T10:00:00.000Z",
  lastSeenAt: "2026-10-27T03:00:00.000Z",
  ref: "developing-today/code#3396",
};

const VALID = "2026-10-27T02:00:00.000Z";
const IMPOSSIBLE = ["2026-02-30T02:00:00.000Z", "2026-02-29T02:00:00.000Z", "2026-04-31T02:00:00.000Z"];

function rawWith(delegation: Record<string, unknown>): TierItem {
  return { ...BASE, whyUser: "decision", delegation } as unknown as TierItem;
}

function proposalDelegation(at: string, updatedAt: string = VALID): Record<string, unknown> {
  return {
    mode: "first-pass",
    status: "proposal-ready",
    updatedAt,
    runId: "orch-2026-10-27T02:00:00Z",
    proposal: { summary: "Recommend CSV", at },
  };
}

function unableDelegation(at: string, updatedAt: string = VALID): Record<string, unknown> {
  return {
    mode: "user",
    status: "unable",
    updatedAt,
    unable: { reason: "No access", whatItTried: ["retried"], whatItNeeds: "a token", at },
  };
}

describe("isIso impossible calendar dates (#3396)", () => {
  test("impossible updatedAt drops the delegation, never the item", () => {
    for (const at of IMPOSSIBLE) {
      const out = sanitizeTierItemExtensions(rawWith({ mode: "user", status: "idle", updatedAt: at }));
      expect(out.delegation).toBeUndefined();
      expect(out.id).toBe(BASE.id);
      expect(out.title).toBe(BASE.title);
    }
  });

  test("impossible proposal.at drops the delegation", () => {
    for (const at of IMPOSSIBLE) {
      const out = sanitizeTierItemExtensions(rawWith(proposalDelegation(at)));
      expect(out.delegation).toBeUndefined();
      expect(out.id).toBe(BASE.id);
    }
  });

  test("impossible unable.at drops the delegation", () => {
    for (const at of IMPOSSIBLE) {
      const out = sanitizeTierItemExtensions(rawWith(unableDelegation(at)));
      expect(out.delegation).toBeUndefined();
      expect(out.id).toBe(BASE.id);
    }
  });

  test("valid timestamps still pass: leap day, offset-bearing, fractional-second", () => {
    for (const at of ["2024-02-29T02:00:00.000Z", "2026-03-01T12:00:00+02:00", "2026-03-01T12:00:00.123Z"]) {
      const out = sanitizeTierItemExtensions(rawWith({ mode: "user", status: "idle", updatedAt: at }));
      expect(out.delegation?.updatedAt).toBe(at);
    }
    const proposal = sanitizeTierItemExtensions(rawWith(proposalDelegation("2026-03-01T12:00:00+02:00")));
    expect(proposal.delegation?.proposal?.at).toBe("2026-03-01T12:00:00+02:00");
  });

  test("proposalExpired TTL consequence: an impossible proposal.at no longer clocks the TTL", () => {
    // Before the fix this delegation survived sanitization and
    // proposalExpired read the phantom Feb-30 (parsed as Mar 2) as
    // unexpired well past a Feb-28 control. Now the delegation drops
    // at sanitize time, so there is no live proposal to expire.
    const out = sanitizeTierItemExtensions(rawWith(proposalDelegation("2026-02-30T00:00:00.000Z", "2026-02-27T00:00:00.000Z")));
    expect(out.delegation).toBeUndefined();
    expect(proposalExpired(out.delegation, Date.parse("2026-03-10T00:00:00.000Z"))).toBe(false);
    // A valid old proposal still expires on schedule.
    const live = sanitizeTierItemExtensions(rawWith(proposalDelegation("2026-02-27T00:00:00.000Z", "2026-02-27T00:00:00.000Z")));
    expect(live.delegation).toBeDefined();
    expect(proposalExpired(live.delegation, Date.parse("2026-03-10T00:00:00.000Z"))).toBe(true);
    expect(proposalExpired(live.delegation, Date.parse("2026-02-27T12:00:00.000Z"))).toBe(false);
  });
});
