import { env as testBindings, runInDurableObject, SELF } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import type { Env } from "../src/index";
import { buildRelayPayload, dnsPacketWithTxt, keypairFromSeed } from "./helpers";
import { z32Encode } from "../src/pkarr";

// Real-DO retention coverage for #10818. Runs once per vitest project; each
// project boots the worker with a different RETENTION_SECONDS (see
// vitest.config.ts). The PUT goes through SELF to the real PkarrStore
// Durable Object and the scheduled alarm is read back from real DO storage —
// no mock store, and the expected alarm is a literal per-setting constant,
// never retentionMilliseconds itself, so a wiring regression in the real DO
// fails this test instead of following a re-implemented expression.

const BASE = "https://pkarr.test";
const TS = 1_762_000_000_000_000n; // fixed microsecond timestamp base
const SEVEN_DAYS_MS = 604_800_000;
const CLOCK_SKEW_TOLERANCE_MS = 60_000;

// Every project value maps to a literal: invalid settings fall back to the
// seven-day default, the valid non-default uses its own retention.
const EXPECTED_RETENTION_MS: Record<string, number> = {
  "0": SEVEN_DAYS_MS,
  "7d": SEVEN_DAYS_MS,
  "8640000000000": SEVEN_DAYS_MS,
  "3600": 3_600_000,
};

const SEED_TAGS: Record<string, number> = {
  "0": 61,
  "7d": 62,
  "8640000000000": 63,
  "3600": 64,
};

const setting = (testBindings as unknown as Env).RETENTION_SECONDS;
const expectedRetentionMs = EXPECTED_RETENTION_MS[setting];
const seedTag = SEED_TAGS[setting] ?? 60;

describe("RETENTION_SECONDS through the real DO", () => {
  it(`schedules the alarm from the real PkarrStore for setting ${setting}`, async () => {
    expect(expectedRetentionMs).toBeDefined();
    const seed = new Uint8Array(32).fill(seedTag);
    const kp = keypairFromSeed(seed);
    const key = z32Encode(kp.publicKey);
    const payload = buildRelayPayload(kp, TS, dnsPacketWithTxt(`_iroh.${key}`, "retention-real-do"));

    const putResponse = await SELF.fetch(`${BASE}/pkarr/${key}`, {
      method: "PUT",
      headers: { "content-type": "application/x-pkarr-signed-packet" },
      body: payload,
    });
    expect(putResponse.status).toBe(204);

    const bindings = testBindings as unknown as Env;
    const stub = bindings.PKARR_STORE.get(bindings.PKARR_STORE.idFromName(key));
    const alarmAt = await runInDurableObject(stub, (_instance, state) => state.storage.getAlarm());
    expect(alarmAt).not.toBeNull();
    const skew = alarmAt! - Date.now();
    expect(skew).toBeGreaterThan(expectedRetentionMs - CLOCK_SKEW_TOLERANCE_MS);
    expect(skew).toBeLessThanOrEqual(expectedRetentionMs);
  });
});
