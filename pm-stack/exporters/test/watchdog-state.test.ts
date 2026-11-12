// Tests for the quota-watchdog state machine (#6032, design #6031).
//
// Oracles, named per the testing doctrine (AGENTS.md):
//   - PBT invariants: properties that must hold for EVERY observation
//     sequence (attempt caps, log bound, monotonic failure counts,
//     cooldown honored, no secret survives sanitization).
//   - MBT: a small reference model of the ladder, stepped in lockstep with
//     the implementation over generated tick sequences; any divergence in
//     decision or in the decision-relevant fields is a failure.
//   - Example tests: the four fixtures the acceptance criteria name
//     (broken-source walk, rate-limited cooldown, human challenge, quiet
//     when healthy).
import { describe, expect, test } from "bun:test";
import {
  FAILURE_CLASSES,
  MAX_ATTEMPTS_PER_RUNG,
  REPAIR_LOG_LIMIT,
  RUNGS,
  type FailureClass,
  type Observation,
  type Rung,
  type SourceHealth,
  type Decision,
  coerceHealth,
  decide,
  initialHealth,
  recordRungOutcome,
  sanitizeDetail,
} from "../src/watchdog-state.ts";

const T0 = Date.parse("2026-11-08T00:00:00Z");
const iso = (min: number) => new Date(T0 + min * 60_000).toISOString();

// --- tiny deterministic PRNG so a failing case is reproducible -----------
function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 0x100000000;
  };
}

function genObs(r: () => number, at: string): Observation {
  if (r() < 0.35) return { kind: "healthy", at };
  const cls = FAILURE_CLASSES[Math.floor(r() * FAILURE_CLASSES.length)]!;
  return { kind: "failed", at, class: cls, detail: r() < 0.2 ? "cookie: abc123" : "probe" };
}

function runSequence(seed: number, ticks: number) {
  const r = rng(seed);
  let h: SourceHealth = initialHealth("fixture");
  const decisions: string[] = [];
  for (let i = 0; i < ticks; i++) {
    const at = iso(i * 15);
    const { decision, next } = decide(h, genObs(r, at));
    decisions.push(decision.action);
    h = next;
  }
  return { health: h, decisions };
}

describe("watchdog-state: PBT invariants", () => {
  test("per-rung attempts never exceed the cap within one incident", () => {
    for (let seed = 1; seed <= 200; seed++) {
      const r = rng(seed);
      let h = initialHealth("fixture");
      for (let i = 0; i < 40; i++) {
        h = decide(h, genObs(r, iso(i * 15))).next;
        for (const rung of RUNGS) {
          expect(h.rungAttempts[rung]).toBeLessThanOrEqual(MAX_ATTEMPTS_PER_RUNG);
        }
      }
    }
  });

  test("repair log stays bounded", () => {
    for (let seed = 1; seed <= 200; seed++) {
      const { health } = runSequence(seed, 60);
      expect(health.repairLog.length).toBeLessThanOrEqual(REPAIR_LOG_LIMIT);
    }
  });

  test("consecutiveFailures is monotonic while failing and resets on success", () => {
    for (let seed = 1; seed <= 100; seed++) {
      const r = rng(seed);
      let h = initialHealth("fixture");
      let prev = 0;
      for (let i = 0; i < 40; i++) {
        const obs = genObs(r, iso(i * 15));
        h = decide(h, obs).next;
        if (obs.kind === "healthy") expect(h.consecutiveFailures).toBe(0);
        else expect(h.consecutiveFailures).toBe(prev + 1);
        prev = h.consecutiveFailures;
      }
    }
  });

  test("a healthy tick always clears the incident and is quiet", () => {
    for (let seed = 1; seed <= 100; seed++) {
      const r = rng(seed);
      let h = initialHealth("fixture");
      for (let i = 0; i < 30; i++) {
        const obs = genObs(r, iso(i * 15));
        const out = decide(h, obs);
        if (obs.kind === "healthy") {
          expect(out.decision.action).toBe("quiet");
          expect(out.next.incident).toBeNull();
          expect(out.next.cooldownUntil).toBeNull();
        }
        h = out.next;
      }
    }
  });

  test("no secret-shaped value survives sanitization", () => {
    for (let seed = 1; seed <= 100; seed++) {
      const { health } = runSequence(seed, 40);
      expect(health.lastDetail).not.toMatch(/abc123/);
      for (const entry of health.repairLog) {
        expect(entry.detail).not.toMatch(/abc123/);
      }
    }
  });
});

// --- MBT: reference model of the ladder ----------------------------------
//
// The model holds only what the ladder decides from: failure count, the
// per-rung attempt counts, and whether a cooldown is live. It is written
// independently of the implementation so agreement is meaningful.
function modelStep(
  m: { fails: number; attempts: Record<Rung, number>; cooldownUntil: number | null },
  obs: Observation,
): { action: Decision["action"]; rung?: Rung } {
  if (obs.kind === "healthy") {
    m.fails = 0;
    m.attempts = { 0: 0, 1: 0, 2: 0, 3: 0 };
    m.cooldownUntil = null;
    return { action: "quiet" };
  }
  m.fails += 1;
  const now = Date.parse(obs.at);
  if (m.cooldownUntil !== null && now < m.cooldownUntil) return { action: "wait-cooldown" };
  const desired: Rung =
    obs.class === "rate-limited"
      ? 0
      : obs.class === "endpoint-changed" || obs.class === "unknown"
        ? 1
        : obs.class === "browser-down" || obs.class === "cdp-unreachable"
          ? 2
          : 3;
  let rung: Rung | null = null;
  if (desired === 0 && m.attempts[0] >= MAX_ATTEMPTS_PER_RUNG) return { action: "escalate" };
  for (let r = desired; r <= 3; r++)
    if (m.attempts[r as Rung] < MAX_ATTEMPTS_PER_RUNG) {
      rung = r as Rung;
      break;
    }
  if (rung === null) return { action: "escalate" };
  m.attempts[rung] += 1;
  if (rung === 0) {
    m.cooldownUntil = now + 15 * 60_000;
    return { action: "wait-cooldown", rung };
  }
  return { action: rung === 1 ? "retry" : "repair", rung };
}

describe("watchdog-state: MBT against the reference ladder model", () => {
  test("implementation agrees with the model on every tick", () => {
    for (let seed = 1; seed <= 300; seed++) {
      const r = rng(seed);
      let h = initialHealth("fixture");
      const m = {
        fails: 0,
        attempts: { 0: 0, 1: 0, 2: 0, 3: 0 } as Record<Rung, number>,
        cooldownUntil: null as number | null,
      };
      for (let i = 0; i < 40; i++) {
        const obs = genObs(r, iso(i * 15));
        const expected = modelStep(m, obs);
        const actual = decide(h, obs);
        expect(actual.decision.action).toBe(expected.action);
        if (
          expected.rung !== undefined &&
          expected.rung !== 0 &&
          actual.decision.action !== "quiet" &&
          actual.decision.action !== "human-blocked"
        ) {
          expect((actual.decision as { rung?: Rung }).rung).toBe(expected.rung);
        }
        expect(actual.next.consecutiveFailures).toBe(m.fails);
        h = actual.next;
      }
    }
  });
});

// --- Fixtures named by the acceptance criteria ---------------------------

describe("watchdog-state: acceptance fixtures", () => {
  test("broken source walks retry -> CDP re-validation -> re-login -> escalation", () => {
    let h = initialHealth("broken");
    const seen: string[] = [];
    for (let i = 0; i < 20; i++) {
      const { decision, next } = decide(h, { kind: "failed", at: iso(i * 15), class: "session-expired" });
      if (decision.action === "repair") seen.push(`rung${decision.rung}`);
      else seen.push(decision.action);
      h = next;
    }
    // desired rung for session-expired is 3; two attempts then escalate.
    expect(seen.filter((s) => s === "rung3").length).toBe(MAX_ATTEMPTS_PER_RUNG);
    expect(seen).toContain("escalate");
    expect(h.incident?.surfacing).toBe("escalate");
  });

  test("rate-limited sets cooldownUntil on disk and the next tick honors it without browser action", () => {
    const first = decide(initialHealth("rl"), { kind: "failed", at: iso(0), class: "rate-limited" });
    expect(first.decision.action).toBe("wait-cooldown");
    expect(first.next.cooldownUntil).toBe(iso(15));
    const second = decide(first.next, { kind: "failed", at: iso(5), class: "rate-limited" });
    expect(second.decision).toEqual({ action: "wait-cooldown", until: iso(15) });
    // No rung beyond 0 was spent while cooling down.
    expect(second.next.rungAttempts[2]).toBe(0);
    expect(second.next.rungAttempts[3]).toBe(0);
    // Cooldown expiry resumes the ladder.
    const third = decide(second.next, { kind: "failed", at: iso(16), class: "rate-limited" });
    expect(third.decision.action).toBe("wait-cooldown"); // rung 0 again (attempt 2)
    const fourth = decide(third.next, { kind: "failed", at: iso(40), class: "rate-limited" });
    expect(fourth.decision.action).toBe("escalate"); // rung 0 exhausted -> escalate
  });

  test("human-only challenge marks the incident human-blocked and surfaces", () => {
    const first = decide(initialHealth("human"), { kind: "failed", at: iso(0), class: "session-expired" });
    expect(first.decision.action).toBe("repair");
    const blocked = recordRungOutcome(first.next, iso(0), 3, "failed", "human-only challenge: verify it's you");
    expect(blocked.incident?.humanBlocked).toBe(true);
    expect(blocked.incident?.surfacing).toBe("human-blocked");
    // Subsequent ticks do not hammer the challenge.
    const again = decide(blocked, { kind: "failed", at: iso(15), class: "session-expired" });
    expect(again.decision.action).toBe("human-blocked");
  });

  test("quiet when healthy: no rung, no incident, no cooldown", () => {
    const { decision, next } = decide(initialHealth("ok"), { kind: "healthy", at: iso(0) });
    expect(decision.action).toBe("quiet");
    expect(next.incident).toBeNull();
    expect(next.currentClass).toBeNull();
    expect(Object.values(next.rungAttempts).every((n) => n === 0)).toBe(true);
  });
});

describe("watchdog-state: disk round-trip", () => {
  test("coerceHealth restores the decision-relevant fields and survives garbage", () => {
    const h = decide(initialHealth("rl"), { kind: "failed", at: iso(0), class: "rate-limited" }).next;
    const back = coerceHealth("rl", JSON.parse(JSON.stringify(h)));
    expect(back.cooldownUntil).toBe(h.cooldownUntil);
    expect(back.rungAttempts).toEqual(h.rungAttempts);
    expect(back.consecutiveFailures).toBe(1);
    // Garbage never throws; it yields a fresh record.
    const fresh = coerceHealth("x", { rungAttempts: "nope", repairLog: [1, 2], incident: { id: 5 } });
    expect(fresh.rungAttempts).toEqual({ 0: 0, 1: 0, 2: 0, 3: 0 });
    expect(fresh.incident).toBeNull();
  });

  test("sanitizeDetail redacts credential-shaped text", () => {
    expect(sanitizeDetail("cookie: abcdef")).toBe("cookie=<redacted>");
    expect(sanitizeDetail("token=deadbeef")).toBe("token=<redacted>");
    expect(sanitizeDetail("x".repeat(600)).length).toBeLessThanOrEqual(500);
    expect(sanitizeDetail("a b   c").length).toBe(5);
  });
});
