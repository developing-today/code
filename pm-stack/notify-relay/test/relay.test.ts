// Oracles for the pm-notify relay core (#8569, design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md):
//  - §1 seq rules: the DO keeps only the highest snapshot seq; an
//    equal seq is an idempotent retry; a lower seq is a regression.
//  - §2/§3 episode mechanics: the alarm at last-beat + 150s opens
//    exactly one silent episode; the first beat after silence
//    closes it with exactly one recovered owed, naming the
//    downtime; a fresh silence opens a fresh episode.
//  - §3 payload terms: the silent push rides class `incident`
//    with the since-when in the body; the recovered push names
//    the downtime length.
import { describe, expect, test } from "bun:test";
import {
  ALARM_AFTER_MS,
  applyAlarm,
  applyHeartbeat,
  applySnapshot,
  buildRecoveredPayload,
  buildSilentPayload,
  emptyRelayState,
  formatDuration,
  isAuthorized,
  takesSilentClass,
  validateHeartbeat,
  validateSnapshot,
  type MirroredSubscription,
  type RelayState,
} from "../src/relay.ts";

// A syntactically valid subscription: 65-byte point + 16-byte auth,
// base64url. Values are fixtures, not any real device's.
const point65 = Buffer.from(new Uint8Array(65).fill(7)).toString("base64url");
const auth16 = Buffer.from(new Uint8Array(16).fill(9)).toString("base64url");
const sub = (over: Partial<MirroredSubscription> = {}): MirroredSubscription => ({
  endpoint: "https://fcm.googleapis.com/fcm/send/fixture",
  p256dh: point65,
  auth: auth16,
  classes: { incident: true },
  quiet: { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true },
  ...over,
});

const beat = (state: RelayState, epoch: string, seq: number, now: number) =>
  applyHeartbeat(state, { stackEpoch: epoch, seq }, now).state;

describe("subscription snapshot seq rules (design §1)", () => {
  test("first snapshot stores; a higher seq replaces it whole", () => {
    const first = applySnapshot(emptyRelayState(), { snapshotSeq: 1, subscriptions: [sub()] });
    expect(first.outcome).toBe("stored");
    const second = applySnapshot(first.state, { snapshotSeq: 2, subscriptions: [] });
    expect(second.outcome).toBe("stored");
    expect(second.state.snapshot?.subscriptions).toEqual([]);
  });

  test("an equal seq is a duplicate: acked, state untouched", () => {
    const stored = applySnapshot(emptyRelayState(), { snapshotSeq: 5, subscriptions: [sub()] }).state;
    const dup = applySnapshot(stored, { snapshotSeq: 5, subscriptions: [] });
    expect(dup.outcome).toBe("duplicate");
    expect(dup.state.snapshot?.subscriptions).toHaveLength(1);
  });

  test("a lower seq is a regression: state untouched", () => {
    const stored = applySnapshot(emptyRelayState(), { snapshotSeq: 5, subscriptions: [sub()] }).state;
    const reg = applySnapshot(stored, { snapshotSeq: 4, subscriptions: [] });
    expect(reg.outcome).toBe("regression");
    expect(reg.state.snapshot?.snapshotSeq).toBe(5);
  });
});

describe("the dead-man's switch (design §2/§3)", () => {
  const t0 = 1_700_000_000_000;

  test("a heartbeat records the beat and re-arms the alarm at +150s", () => {
    const out = applyHeartbeat(emptyRelayState(), { stackEpoch: "epoch-a", seq: 1 }, t0);
    expect(out.state.heartbeat.lastBeatAt).toBe(t0);
    expect(out.state.heartbeat.stackEpoch).toBe("epoch-a");
    expect(out.alarmAt).toBe(t0 + ALARM_AFTER_MS);
    expect(out.recovered).toBeNull();
  });

  test("an alarm inside the window opens nothing; at the window it opens the episode, once", () => {
    const state = beat(emptyRelayState(), "epoch-a", 1, t0);
    expect(applyAlarm(state, t0 + ALARM_AFTER_MS - 1).opened).toBe(false);
    const opened = applyAlarm(state, t0 + ALARM_AFTER_MS);
    expect(opened.opened).toBe(true);
    // Silence is measured from the last beat, not the alarm fire.
    expect(opened.state.heartbeat.silentSince).toBe(t0);
    // A second fire while the episode is open opens nothing new.
    expect(applyAlarm(opened.state, t0 + ALARM_AFTER_MS + 60_000).opened).toBe(false);
  });

  test("an alarm with no beat ever seen opens nothing", () => {
    expect(applyAlarm(emptyRelayState(), t0).opened).toBe(false);
  });

  test("the first beat after silence closes the episode and owes one recovered, naming the downtime", () => {
    let state = beat(emptyRelayState(), "epoch-a", 1, t0);
    state = applyAlarm(state, t0 + ALARM_AFTER_MS).state;
    const back = t0 + 42 * 60_000;
    const out = applyHeartbeat(state, { stackEpoch: "epoch-b", seq: 1 }, back);
    expect(out.recovered).toEqual({ downtimeMs: 42 * 60_000, epochChanged: true });
    expect(out.state.heartbeat.silentSince).toBeNull();
    // And the beat after that owes nothing.
    expect(applyHeartbeat(out.state, { stackEpoch: "epoch-b", seq: 2 }, back + 60_000).recovered).toBeNull();
  });

  test("a same-epoch resume (laptop slept, did not die) also closes the episode", () => {
    let state = beat(emptyRelayState(), "epoch-a", 7, t0);
    state = applyAlarm(state, t0 + ALARM_AFTER_MS).state;
    const out = applyHeartbeat(state, { stackEpoch: "epoch-a", seq: 8 }, t0 + 10 * 60_000);
    expect(out.recovered).toEqual({ downtimeMs: 10 * 60_000, epochChanged: false });
    expect(out.state.heartbeat.silentSince).toBeNull();
  });

  test("a second silence opens a second episode (the switch re-arms)", () => {
    let state = beat(emptyRelayState(), "epoch-a", 1, t0);
    state = applyAlarm(state, t0 + ALARM_AFTER_MS).state;
    state = applyHeartbeat(state, { stackEpoch: "epoch-b", seq: 1 }, t0 + 20 * 60_000).state;
    const t1 = t0 + 20 * 60_000;
    expect(applyAlarm(state, t1 + ALARM_AFTER_MS).opened).toBe(true);
  });
});

describe("payloads (design §3 terms)", () => {
  test("the silent payload is an incident naming the silence start", () => {
    const t0 = 1_700_000_000_000;
    let state = beat(emptyRelayState(), "epoch-a", 1, t0);
    state = { ...applyAlarm(state, t0 + ALARM_AFTER_MS).state, origin: "https://pm-notify.example.workers.dev" };
    const payload = buildSilentPayload(state);
    expect(payload.class).toBe("incident");
    expect(payload.objectRef).toBe("stack:silent");
    expect(payload.title).toBe("PM stack is down");
    expect(payload.body).toContain(new Date(t0).toISOString());
    expect(payload.url).toBe("https://pm-notify.example.workers.dev/status");
  });

  test("the recovered payload names the downtime length", () => {
    const payload = buildRecoveredPayload(emptyRelayState(), 42 * 60_000);
    expect(payload.title).toBe("PM stack recovered");
    expect(payload.body).toContain("42m");
  });

  test("formatDuration reads like the stack's other durations", () => {
    expect(formatDuration(45_000)).toBe("45s");
    expect(formatDuration(5 * 60_000)).toBe("5m");
    expect(formatDuration(60 * 60_000)).toBe("1h");
    expect(formatDuration((2 * 60 + 5) * 60_000)).toBe("2h 5m");
  });

  test("the incident class gate: default on, explicit off respected, quiet never consulted", () => {
    expect(takesSilentClass(sub({ classes: {} }))).toBe(true);
    expect(takesSilentClass(sub({ classes: { incident: false } }))).toBe(false);
    expect(takesSilentClass(sub({ quiet: { start: "00:00", end: "23:59", tz: "UTC", enabled: true } }))).toBe(true);
  });
});

describe("validation (design §1 shapes, rejected loudly)", () => {
  test("a well-formed snapshot validates and normalizes the endpoint", () => {
    const out = validateSnapshot({ snapshotSeq: 3, subscriptions: [sub()] });
    expect(out.ok).toBe(true);
  });

  test("snapshot rejects a bad seq, a non-https endpoint, and wrong key lengths", () => {
    expect(validateSnapshot({ snapshotSeq: 0, subscriptions: [] }).ok).toBe(false);
    expect(validateSnapshot({ snapshotSeq: 1, subscriptions: [sub({ endpoint: "http://fcm.googleapis.com/x" })] }).ok).toBe(false);
    expect(validateSnapshot({ snapshotSeq: 1, subscriptions: [sub({ p256dh: "AAAA" })] }).ok).toBe(false);
    expect(validateSnapshot({ snapshotSeq: 1, subscriptions: [sub({ auth: "AAAA" })] }).ok).toBe(false);
    expect(validateSnapshot({ snapshotSeq: 1, subscriptions: [sub({ classes: { incident: "yes" } as never })] }).ok).toBe(false);
  });

  test("heartbeat rejects an empty epoch and a negative seq", () => {
    expect(validateHeartbeat({ stackEpoch: "", seq: 1 }).ok).toBe(false);
    expect(validateHeartbeat({ stackEpoch: "e", seq: -1 }).ok).toBe(false);
    expect(validateHeartbeat({ stackEpoch: "e", seq: 0, at: "2026-11-23T00:00:00Z" }).ok).toBe(true);
  });
});

describe("shared-secret auth (fail closed)", () => {
  const headers = (init: Record<string, string>) => new Headers(init);
  test("no configured secret authorizes nothing", () => {
    expect(isAuthorized(headers({ Authorization: "Bearer x" }), undefined)).toBe(false);
  });
  test("bearer and header forms match; wrong secrets do not", () => {
    expect(isAuthorized(headers({ Authorization: "Bearer s3cret" }), "s3cret")).toBe(true);
    expect(isAuthorized(headers({ "X-Relay-Secret": "s3cret" }), "s3cret")).toBe(true);
    expect(isAuthorized(headers({ Authorization: "Bearer other" }), "s3cret")).toBe(false);
    expect(isAuthorized(headers({}), "s3cret")).toBe(false);
  });
});
