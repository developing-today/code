import { describe, expect, test } from "bun:test";
import {
  buildPacket,
  canonicalDedupeKey,
  dispatchDedupeKey,
  validatePayload,
} from "../src/packet.ts";

const caller = { type: "bearer" } as const;

describe("dedupe keys", () => {
  test("canonical keys follow the fleet families", () => {
    expect(canonicalDedupeKey("small_fix", "developing-today/code", 42)).toBe(
      "repo-health:v1:fix:developing-today/code:issue:42",
    );
    expect(canonicalDedupeKey("triage", "developing-today/opencode", 7)).toBe(
      "repo-health:v1:triage:developing-today/opencode:issue:7",
    );
    expect(canonicalDedupeKey("ci_investigation", "dezren39/nix", 9)).toBe(
      "repo-health:v1:triage:dezren39/nix:issue:9",
    );
    expect(canonicalDedupeKey("pr_shepherd", "developing-today/code", 3)).toBeNull();
  });

  test("dispatch keys are unique triage-shaped keys with an 8-hex suffix", () => {
    const a = dispatchDedupeKey("developing-today/code");
    const b = dispatchDedupeKey("developing-today/code");
    expect(a).toMatch(/^repo-health:v1:triage:developing-today\/code:dispatch:[0-9a-f]{8}$/);
    expect(a).not.toBe(b);
  });
});

describe("packet building", () => {
  test("an issue-backed small_fix packet passes the payload self-check", () => {
    const packet = buildPacket(
      {
        repo: "developing-today/opencode",
        kind: "small_fix",
        title: "fix the thing",
        prompt: "please fix the thing",
        priority: 20,
        caller,
        model: { providerID: "opencode-go", id: "gpt-5.6-luna" },
        directory: "/home/user/git/opencode",
        issue: { number: 952, url: "https://github.com/developing-today/opencode/issues/952" },
      },
      canonicalDedupeKey("small_fix", "developing-today/opencode", 952)!,
    );
    expect(validatePayload(packet.payload)).toEqual([]);
    expect(packet.payload.origin).toBe("dispatch");
    expect(packet.payload.model_hint).toEqual({ providerID: "opencode-go", id: "gpt-5.6-luna" });
    expect((packet.payload.issue as { number: number }).number).toBe(952);
    expect(packet.payload.timebox_minutes).toBe(45);
    expect(packet.payload.max_attempts).toBe(2);
  });

  test("a no-issue triage packet still passes via the dispatch origin identity", () => {
    const packet = buildPacket(
      {
        repo: "developing-today/code",
        kind: "triage",
        title: "look into it",
        prompt: "look into it",
        priority: 40,
        caller: {
          type: "oidc",
          repository: "developing-today/code",
          actor: "dezren39",
        },
        model: null,
        directory: "/home/user/git/code",
        issue: null,
      },
      dispatchDedupeKey("developing-today/code"),
    );
    expect(validatePayload(packet.payload)).toEqual([]);
    expect(packet.payload.timebox_minutes).toBe(20);
    expect(packet.payload.max_attempts).toBe(3);
    expect(packet.payload.issue).toBeUndefined();
  });
});
