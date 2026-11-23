import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  BREAK_GLASS_MAX_TTL_MS,
  BreakGlassTokenStore,
  breakGlassIssuanceAuditEvent,
  consumeBreakGlassToken,
  issueBreakGlassToken,
} from "../src/break-glass.ts";
import { EventsStore } from "../src/events.ts";
import type { NewAuditEvent } from "../src/events.ts";

const dirs: string[] = [];
async function setup() {
  const dir = await mkdtemp(join(tmpdir(), "break-glass-"));
  dirs.push(dir);
  const tokensFile = join(dir, "break-glass-tokens.json");
  return {
    dir,
    tokensFile,
    events: new EventsStore(join(dir, "events.json")),
    store: new BreakGlassTokenStore(tokensFile),
  };
}

afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

const T0 = new Date("2026-11-23T05:30:00.000Z");

// Oracle: design #1253 section 6 — the owner break-glass token is
// single-use, lives at most one hour, and its issuance is an audit
// event written before the token works; everything else fails closed.
describe("aggregator break-glass owner token (#9608)", () => {
  test("issuance is audited, and the token then works exactly once", async () => {
    const { events, store } = await setup();
    const issued = await issueBreakGlassToken({ store, events, ttlMs: 30 * 60 * 1000, now: T0 });
    expect(issued.issuedAt).toBe(T0.toISOString());
    expect(issued.expiresAt).toBe(new Date(T0.getTime() + 30 * 60 * 1000).toISOString());
    expect(await events.has("break-glass-token-issued", `break-glass:${issued.id}`)).toBe(true);
    const listed = await events.list({ kind: "break-glass-token-issued" });
    expect(listed).toHaveLength(1);
    expect(listed[0]).toMatchObject({ actor: "owner:local-cli", ref: `break-glass:${issued.id}` });

    const consumed = await consumeBreakGlassToken({ store, token: issued.token, now: T0 });
    expect(consumed).toMatchObject({ ok: true, id: issued.id });
    const second = await consumeBreakGlassToken({ store, token: issued.token, now: T0 });
    expect(second.ok).toBe(false);
    if (!second.ok) expect(second.error).toContain("already consumed");
  });

  test("the audit event is written before the token record is persisted", async () => {
    const { tokensFile, events, store } = await setup();
    let tokensAtAuditTime = -1;
    const orderingEvents = {
      record: async (event: NewAuditEvent) => {
        const raw = await readFile(tokensFile, "utf8").catch(() => "");
        tokensAtAuditTime = raw === "" ? 0 : (JSON.parse(raw).tokens as unknown[]).length;
        return events.record(event);
      },
    };
    await issueBreakGlassToken({ store, events: orderingEvents, ttlMs: 60_000, now: T0 });
    expect(tokensAtAuditTime).toBe(0);
  });

  test("when the audit write fails, issuance fails closed: no token exists", async () => {
    const { tokensFile, store } = await setup();
    const failingEvents = {
      record: async () => {
        throw new Error("audit store unavailable");
      },
    };
    await expect(
      issueBreakGlassToken({ store, events: failingEvents, ttlMs: 60_000, now: T0 }),
    ).rejects.toThrow(/audit store unavailable/);
    const raw = await readFile(tokensFile, "utf8").catch(() => "");
    expect(raw).not.toContain("tokenHash");
    expect((await consumeBreakGlassToken({ store, token: "anything", now: T0 })).ok).toBe(false);
  });

  test("ttl is capped at one hour: longer or non-positive requests are refused", async () => {
    const { events, store } = await setup();
    await expect(
      issueBreakGlassToken({ store, events, ttlMs: BREAK_GLASS_MAX_TTL_MS + 1, now: T0 }),
    ).rejects.toThrow(/one hour/);
    await expect(issueBreakGlassToken({ store, events, ttlMs: 0, now: T0 })).rejects.toThrow();
    await expect(issueBreakGlassToken({ store, events, ttlMs: -1, now: T0 })).rejects.toThrow();
    const issued = await issueBreakGlassToken({ store, events, now: T0 });
    expect(Date.parse(issued.expiresAt) - T0.getTime()).toBe(BREAK_GLASS_MAX_TTL_MS);
  });

  test("expired, unknown, and empty tokens fail closed", async () => {
    const { events, store } = await setup();
    const issued = await issueBreakGlassToken({ store, events, ttlMs: 60_000, now: T0 });
    const later = new Date(T0.getTime() + 60_001);
    const expired = await consumeBreakGlassToken({ store, token: issued.token, now: later });
    expect(expired.ok).toBe(false);
    if (!expired.ok) expect(expired.error).toContain("expired");
    expect((await consumeBreakGlassToken({ store, token: "not-a-token", now: T0 })).ok).toBe(false);
    expect((await consumeBreakGlassToken({ store, token: "", now: T0 })).ok).toBe(false);
    expect((await consumeBreakGlassToken({ store, token: "   ", now: T0 })).ok).toBe(false);
  });

  test("the token itself is never persisted — only its sha256 hash", async () => {
    const { tokensFile, events, store } = await setup();
    const issued = await issueBreakGlassToken({ store, events, ttlMs: 60_000, now: T0 });
    const raw = await readFile(tokensFile, "utf8");
    expect(raw).not.toContain(issued.token);
    const parsed = JSON.parse(raw) as { version: number; tokens: Array<Record<string, unknown>> };
    expect(parsed.version).toBe(1);
    expect(parsed.tokens).toHaveLength(1);
    expect(parsed.tokens[0]).toMatchObject({ id: issued.id, consumedAt: null });
    expect(parsed.tokens[0]!.tokenHash).toMatch(/^[0-9a-f]{64}$/);
  });

  test("issuance audit event shape follows the identity-registry precedent", () => {
    const event = breakGlassIssuanceAuditEvent({
      id: "bg_example",
      expiresAt: "2026-11-23T06:30:00.000Z",
      ttlMs: 3_600_000,
    });
    expect(event).toMatchObject({
      actor: "owner:local-cli",
      kind: "break-glass-token-issued",
      ref: "break-glass:bg_example",
      attribution: { confidence: "authenticated", asserted: true },
      source: { identity: "pm-aggregator", link: "#9608" },
    });
    expect(event.summary).toContain("single-use");
    expect(event.summary).toContain("2026-11-23T06:30:00.000Z");
  });
});
