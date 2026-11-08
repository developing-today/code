// #3729 — a failed VAPID configure attempt must not latch for the
// process lifetime. Oracle: the stubbed secrets layer's call count and
// the stubbed web-push's recorded calls — recovery is proven by the
// secret being RE-RESOLVED after a failure, not by the module's own
// return values alone. The three tests are one narrative in order:
// push.ts module state (configured / configurePromise) is the thing
// under test, so they share it deliberately.
import { describe, expect, mock, test } from "bun:test";

let resolveCalls = 0;
let resolveImpl: () => Promise<string | null> = async () => null;
const forgotten: string[] = [];
mock.module("../src/secrets.ts", () => ({
  resolveBwsSecret: async (_name: string) => {
    resolveCalls += 1;
    return resolveImpl();
  },
  forgetBwsSecret: (name: string) => {
    forgotten.push(name);
  },
}));

const vapidCalls: unknown[][] = [];
let sendImpl: () => Promise<{ statusCode: number }> = async () => ({ statusCode: 201 });
const webpushStub = {
  setVapidDetails: (...args: unknown[]) => {
    vapidCalls.push(args);
  },
  sendNotification: async () => sendImpl(),
};
mock.module("web-push", () => ({ default: webpushStub, ...webpushStub }));

const { ensurePushConfigured, sendPush } = await import("../src/push.ts");

const target = { endpoint: "https://push.example/sub-1", p256dh: "k", auth: "a" };

describe("ensurePushConfigured failure latch (#3729)", () => {
  test("concurrent callers share one in-flight attempt; a failed attempt is retried, not latched", async () => {
    let resolveFirst!: (value: string | null) => void;
    const firstResult = new Promise<string | null>((resolve) => {
      resolveFirst = resolve;
    });
    resolveImpl = () => (resolveCalls === 1 ? firstResult : Promise.resolve("vapid-private-key-1"));

    const a = ensurePushConfigured();
    const b = ensurePushConfigured();
    const c = ensurePushConfigured();
    expect(resolveCalls).toBe(1); // one in-flight attempt, shared
    resolveFirst(null); // transient bws failure
    expect(await Promise.all([a, b, c])).toEqual([false, false, false]);
    expect(vapidCalls.length).toBe(0);

    // bws has recovered: the next call re-resolves instead of
    // returning the settled failure.
    expect(await ensurePushConfigured()).toBe(true);
    expect(resolveCalls).toBe(2);
    expect(vapidCalls.length).toBe(1);
    expect(vapidCalls[0][2]).toBe("vapid-private-key-1");

    // Success IS memoized.
    expect(await ensurePushConfigured()).toBe(true);
    expect(resolveCalls).toBe(2);
  });

  test("sendPush succeeds after recovery", async () => {
    sendImpl = async () => ({ statusCode: 201 });
    expect(await sendPush(target, { hello: "world" })).toEqual({ ok: true, status: 201 });
  });

  test("401/403 rotation still forgets the secret and reconfigures; a genuinely absent key soft-fails per send", async () => {
    sendImpl = async () => {
      throw Object.assign(new Error("forbidden"), { statusCode: 403 });
    };
    const rotated = await sendPush(target, { hello: "world" });
    expect(rotated.ok).toBe(false);
    expect(rotated.status).toBe(403);
    expect(forgotten.length).toBe(1);

    // Next send reconfigures from scratch; with the key genuinely
    // absent it fails soft with the existing outcome — no throw.
    resolveImpl = async () => null;
    const before = resolveCalls;
    expect(await sendPush(target, { hello: "world" })).toEqual({
      ok: false,
      gone: false,
      error: "VAPID private key unavailable (bws)",
    });
    expect(resolveCalls).toBeGreaterThan(before);

    // And that soft failure does not latch either: the key appearing
    // makes the following send work.
    resolveImpl = async () => "vapid-private-key-2";
    sendImpl = async () => ({ statusCode: 201 });
    expect(await sendPush(target, { hello: "world" })).toEqual({ ok: true, status: 201 });
    expect(vapidCalls[vapidCalls.length - 1][2]).toBe("vapid-private-key-2");
  });
});
