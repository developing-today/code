import { describe, expect, test } from "bun:test";
import type { PushSubscriptionRow } from "./alerts.ts";
import type { PushOutcome, PushTarget } from "./push.ts";
import type { PromptRecord } from "./prompts.ts";
import {
  buildPromptCompletionPayload,
  notifyPromptCompletion,
  promptCompletionTransition,
  promptCompletionUrl,
} from "./prompt-notify.ts";

const ORIGIN = "https://aggregator.example";

function record(overrides: Partial<PromptRecord> = {}): PromptRecord {
  return {
    scope: "page",
    target: "claude",
    mode: "async",
    page: "queue",
    sectionRef: null,
    prompt: "reprioritize the queue",
    replyTo: null,
    refs: [],
    notify: true,
    attachments: [],
    id: "prompt:abc",
    threadId: "prompt:abc",
    status: "queued",
    createdAt: "2026-11-23T00:00:00Z",
    archived: false,
    read: false,
    ...overrides,
  };
}

describe("promptCompletionTransition (#10035 S6)", () => {
  test("fires on queued -> answered with the notify opt-in", () => {
    expect(promptCompletionTransition(record(), record({ status: "answered", answer: "done" }))).toBe(true);
  });

  test("fires on running -> failed with the notify opt-in", () => {
    expect(
      promptCompletionTransition(record({ status: "running" }), record({ status: "failed", error: "boom" })),
    ).toBe(true);
  });

  test("never fires without the per-submission opt-in", () => {
    expect(
      promptCompletionTransition(record({ notify: false }), record({ status: "answered", notify: false })),
    ).toBe(false);
  });

  test("never re-fires for a re-patch of a terminal record", () => {
    const answered = record({ status: "answered", answer: "done" });
    expect(promptCompletionTransition(answered, record({ status: "answered", answer: "done again" }))).toBe(false);
  });

  test("does not fire for non-terminal transitions or a missing before-image", () => {
    expect(promptCompletionTransition(record(), record({ status: "running" }))).toBe(false);
    expect(promptCompletionTransition(record(), record({ archived: true }))).toBe(false);
    expect(promptCompletionTransition(null, record({ status: "answered" }))).toBe(false);
  });
});

describe("buildPromptCompletionPayload (#10035 S6)", () => {
  test("an answered prompt links to its own chat card and carries both excerpts", () => {
    const r = record({ status: "answered", answer: "the queue is reprioritized" });
    const payload = buildPromptCompletionPayload(r, ORIGIN);
    expect(payload.class).toBe("prompt");
    expect(payload.objectRef).toBe("prompt:abc");
    expect(payload.title).toBe("Prompt answered");
    expect(payload.url).toBe(`${ORIGIN}/chat#prompt-prompt:abc`);
    expect(payload.body).toContain("reprioritize the queue");
    expect(payload.body).toContain("the queue is reprioritized");
    expect(promptCompletionUrl(ORIGIN, r)).toBe(payload.url);
  });

  test("a failed prompt names the failure and carries the error", () => {
    const payload = buildPromptCompletionPayload(record({ status: "failed", error: "hook exited 1" }), ORIGIN);
    expect(payload.title).toBe("Prompt failed");
    expect(payload.body).toContain("hook exited 1");
  });
});

function sub(endpoint: string, classes: Record<string, boolean>): PushSubscriptionRow {
  return {
    id: `sub:${endpoint}`,
    endpoint,
    p256dh: "key",
    auth: "auth",
    classes,
    quiet: { enabled: false, start: "22:00", end: "07:00", tz: "UTC" },
    user_agent: null,
    created_at: "2026-11-01T00:00:00Z",
    last_seen_at: "2026-11-01T00:00:00Z",
    gone_count: 0,
  };
}

describe("notifyPromptCompletion (#10035 S6)", () => {
  test("fans out only to prompt-class subscriptions and ledgers every attempt", async () => {
    const sends: Array<{ class: string; objectRef: string; outcome: string }> = [];
    const alive: string[] = [];
    const gone: string[] = [];
    const store = {
      listSubscriptions: async () => [
        sub("https://push.example/ok", { prompt: true }),
        sub("https://push.example/gone", { prompt: true }),
        sub("https://push.example/err", { prompt: true }),
        sub("https://push.example/off", { prompt: false }),
        // A pre-class subscription: only "incident" defaults on for
        // old rows, so this one is not eligible until it opts in.
        sub("https://push.example/legacy", { incident: true }),
      ],
      logSend: async (entry: { class: string; objectRef: string; outcome: string }) => {
        sends.push(entry);
      },
      noteSubscriptionAlive: async (endpoint: string) => {
        alive.push(endpoint);
      },
      noteSubscriptionGone: async (endpoint: string) => {
        gone.push(endpoint);
      },
    };
    const send = async (target: PushTarget): Promise<PushOutcome> => {
      if (target.endpoint.endsWith("/ok")) return { ok: true, status: 201 };
      if (target.endpoint.endsWith("/gone")) return { ok: false, gone: true, status: 410, error: "gone" };
      return { ok: false, gone: false, status: 500, error: "vendor error" };
    };
    const summary = await notifyPromptCompletion(
      store,
      record({ status: "answered", answer: "done" }),
      ORIGIN,
      send,
    );
    expect(summary).toEqual({ subscriptions: 5, eligible: 3, sent: 1, gone: 1, errors: 1 });
    expect(sends).toEqual([
      { class: "prompt", objectRef: "prompt:abc", outcome: "sent" },
      { class: "prompt", objectRef: "prompt:abc", outcome: "gone" },
      { class: "prompt", objectRef: "prompt:abc", outcome: "error" },
    ]);
    expect(alive).toEqual(["https://push.example/ok"]);
    expect(gone).toEqual(["https://push.example/gone"]);
  });

  test("no eligible subscriptions is a clean no-op", async () => {
    const store = {
      listSubscriptions: async () => [sub("https://push.example/off", { prompt: false })],
      logSend: async () => {
        throw new Error("must not ledger a send that was never attempted");
      },
      noteSubscriptionAlive: async () => {},
      noteSubscriptionGone: async () => {},
    };
    const summary = await notifyPromptCompletion(store, record({ status: "answered" }), ORIGIN, async () => {
      throw new Error("must not send");
    });
    expect(summary).toEqual({ subscriptions: 1, eligible: 0, sent: 0, gone: 0, errors: 0 });
  });
});
