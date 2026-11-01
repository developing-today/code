import { describe, expect, test } from "bun:test";
import {
  ValidationError,
  parseModel,
  parseSessionRequest,
  parseTaskRequest,
  resolvePriority,
  resolveSessionDirectory,
  titleFromPrompt,
} from "../src/validate.ts";

describe("priority mapping", () => {
  test("names map to the design bands", () => {
    expect(resolvePriority("critical")).toBe(10);
    expect(resolvePriority("high")).toBe(20);
    expect(resolvePriority("normal")).toBe(40);
    expect(resolvePriority("low")).toBe(70);
    expect(resolvePriority("HIGH")).toBe(20);
  });

  test("absent defaults to normal (40)", () => {
    expect(resolvePriority(undefined)).toBe(40);
    expect(resolvePriority(null)).toBe(40);
  });

  test("scale numbers pass through", () => {
    for (const v of [10, 20, 30, 40, 50, 70, 90]) expect(resolvePriority(v)).toBe(v);
  });

  test("decimal integer strings follow the numeric scale and validation", () => {
    for (let v = 10; v <= 90; v++) expect(resolvePriority(String(v))).toBe(resolvePriority(v));
    for (const v of ["5", "95", "50.5", "", "0x32", "5e1"]) {
      expect(() => resolvePriority(v)).toThrow(ValidationError);
    }
  });

  test("off-scale integers in range snap to the nearest scale value", () => {
    expect(resolvePriority(15)).toBe(10); // tie -> down
    expect(resolvePriority(16)).toBe(20);
    expect(resolvePriority(45)).toBe(40); // tie -> down
    expect(resolvePriority(60)).toBe(50); // tie -> down
    expect(resolvePriority(80)).toBe(70); // tie -> down
    expect(resolvePriority(89)).toBe(90);
  });

  test("out-of-range and non-numeric priorities are rejected", () => {
    expect(() => resolvePriority(5)).toThrow(ValidationError);
    expect(() => resolvePriority(95)).toThrow(ValidationError);
    expect(() => resolvePriority(40.5)).toThrow(ValidationError);
    expect(() => resolvePriority("urgent")).toThrow(ValidationError);
    expect(() => resolvePriority(true)).toThrow(ValidationError);
  });

  test("prototype spellings are rejected, not resolved through the prototype chain (#2757)", () => {
    // PRIORITY_NAMES is a plain object: without the own-key guard these
    // returned inherited members (the Object constructor function,
    // Object.prototype, ...) - non-numbers that flowed toward
    // Packet.priority / p_priority. Every spelling that trims/lowercases
    // to an inherited member must take the ordinary unknown-name 400.
    for (const name of [
      "constructor",
      "Constructor",
      " CONSTRUCTOR ",
      "__proto__",
      "__PROTO__",
      "toString",
      "valueOf",
      "hasOwnProperty",
      "isPrototypeOf",
      "propertyIsEnumerable",
      "toLocaleString",
      "__defineGetter__",
      "__lookupGetter__",
    ]) {
      expect(() => resolvePriority(name)).toThrow(ValidationError);
      expect(() => resolvePriority(name)).toThrow(/unknown name/);
    }
    // The four real names still resolve.
    expect(resolvePriority("critical")).toBe(10);
    expect(resolvePriority(" low ")).toBe(70);
  });
});

describe("model parsing", () => {
  test("absent model is null", () => {
    expect(parseModel(undefined)).toBeNull();
    expect(parseModel(null)).toBeNull();
  });

  test("string form splits on the first slash only", () => {
    expect(parseModel("deepseek/deepseek-chat")).toEqual({ providerID: "deepseek", id: "deepseek-chat" });
    expect(parseModel("meta/claude/sonnet")).toEqual({ providerID: "meta", id: "claude/sonnet" });
  });

  test("object form accepts {providerID, id} and {provider, model}", () => {
    expect(parseModel({ providerID: "opencode-go", id: "gpt-5.6-luna" })).toEqual({
      providerID: "opencode-go",
      id: "gpt-5.6-luna",
    });
    expect(parseModel({ provider: "opencode-go", model: "gpt-5.6-luna" })).toEqual({
      providerID: "opencode-go",
      id: "gpt-5.6-luna",
    });
  });

  test("malformed models are rejected", () => {
    expect(() => parseModel("noslash")).toThrow(ValidationError);
    expect(() => parseModel("/model")).toThrow(ValidationError);
    expect(() => parseModel("provider/")).toThrow(ValidationError);
    expect(() => parseModel({ providerID: "x" })).toThrow(ValidationError);
    expect(() => parseModel(42)).toThrow(ValidationError);
  });
});

describe("request validation", () => {
  test("action-shaped priority strings resolve for both request types", () => {
    for (const [priority, expected] of [["10", 10], ["45", 40], ["50", 50], ["90", 90], ["high", 20], [undefined, 40]] as const) {
      // Both actions use jq --arg, which serializes priority as a JSON string.
      const payload = JSON.parse(JSON.stringify({ repo: "developing-today/code", title: "t", prompt: "p", priority }));
      expect(parseTaskRequest(payload).priority).toBe(expected);
      expect(parseSessionRequest(payload).priority).toBe(expected);
    }
  });
  test("sessions require repo and prompt; defaults issue+wait on", () => {
    const r = parseSessionRequest({ repo: "developing-today/code", prompt: "do the thing" });
    expect(r.issue).toBe(true);
    expect(r.wait).toBe(true);
    expect(r.priority).toBe(40);
    expect(() => parseSessionRequest({ prompt: "x" })).toThrow(/repo/);
    expect(() => parseSessionRequest({ repo: "developing-today/code" })).toThrow(/prompt/);
    expect(() => parseSessionRequest("nope")).toThrow(ValidationError);
  });

  test("a prototype-spelling priority 400s at parse time, so no non-number reaches the packet (#2757)", () => {
    expect(() =>
      parseSessionRequest({ repo: "developing-today/code", prompt: "x", priority: "constructor" }),
    ).toThrow(ValidationError);
    expect(() =>
      parseSessionRequest({ repo: "developing-today/code", prompt: "x", priority: "__proto__" }),
    ).toThrow(ValidationError);
    expect(() =>
      parseTaskRequest({ repo: "developing-today/code", title: "t", prompt: "p", priority: "constructor" }),
    ).toThrow(ValidationError);
    expect(() =>
      parseTaskRequest({ repo: "developing-today/code", title: "t", prompt: "p", priority: "__proto__" }),
    ).toThrow(ValidationError);
    // A valid named priority still parses to a number.
    expect(parseSessionRequest({ repo: "developing-today/code", prompt: "x", priority: "high" }).priority).toBe(20);
  });

  test("tasks accept prompt or body, default kind triage and issue off", () => {
    const a = parseTaskRequest({ repo: "developing-today/code", title: "t", prompt: "p" });
    expect(a.kind).toBe("triage");
    expect(a.issue).toBe(false);
    const b = parseTaskRequest({ repo: "developing-today/code", title: "t", body: "p", kind: "small_fix" });
    expect(b.prompt).toBe("p");
    expect(b.kind).toBe("small_fix");
    expect(() => parseTaskRequest({ repo: "developing-today/code", title: "t" })).toThrow(/prompt/);
    expect(() => parseTaskRequest({ repo: "developing-today/code", title: "t", prompt: "p", kind: "bogus" })).toThrow(
      /kind/,
    );
  });

  test("titleFromPrompt takes the first non-empty line, capped", () => {
    expect(titleFromPrompt("\n\n  fix the thing  \nmore")).toBe("fix the thing");
    expect(titleFromPrompt("x".repeat(200)).length).toBe(120);
  });
});

describe("task issue_number parsing", () => {
  const base = { repo: "developing-today/code", title: "t", prompt: "p" };

  test("accepts a positive integer and exposes it as issueNumber", () => {
    expect(parseTaskRequest({ ...base, issue_number: 2672 }).issueNumber).toBe(2672);
    expect(parseTaskRequest({ ...base, issue_number: 1 }).issueNumber).toBe(1);
  });

  test("absent issue_number is undefined", () => {
    expect(parseTaskRequest(base).issueNumber).toBeUndefined();
    expect(parseTaskRequest({ ...base, issue_number: null }).issueNumber).toBeUndefined();
  });

  test("rejects zero, negatives, floats and strings", () => {
    expect(() => parseTaskRequest({ ...base, issue_number: 0 })).toThrow(/issue_number/);
    expect(() => parseTaskRequest({ ...base, issue_number: -3 })).toThrow(ValidationError);
    expect(() => parseTaskRequest({ ...base, issue_number: 1.5 })).toThrow(ValidationError);
    expect(() => parseTaskRequest({ ...base, issue_number: "42" })).toThrow(ValidationError);
    expect(() => parseTaskRequest({ ...base, issue_number: true })).toThrow(ValidationError);
  });
});

describe("session directory scoping (issue #2903)", () => {
  const repoDirs = {
    "developing-today/code": "/home/user/git/code",
    "developing-today/opencode": "/home/user/git/opencode",
  };
  const code = "developing-today/code";

  test("OIDC + omitted directory resolves to the repo clone", () => {
    expect(resolveSessionDirectory("oidc", code, undefined, repoDirs)).toBe("/home/user/git/code");
  });

  test("OIDC + own clone directory is accepted", () => {
    expect(resolveSessionDirectory("oidc", code, "/home/user/git/code", repoDirs)).toBe("/home/user/git/code");
  });

  test("OIDC + own clone with trailing slashes is accepted and normalized", () => {
    expect(resolveSessionDirectory("oidc", code, "/home/user/git/code///", repoDirs)).toBe("/home/user/git/code");
  });

  test("OIDC + another allowlisted clone is rejected", () => {
    expect(() => resolveSessionDirectory("oidc", code, "/home/user/git/opencode", repoDirs)).toThrow(ValidationError);
  });

  test("OIDC + arbitrary directory is rejected", () => {
    expect(() => resolveSessionDirectory("oidc", code, "/tmp/evil", repoDirs)).toThrow(ValidationError);
    expect(() => resolveSessionDirectory("oidc", code, "/home/user/git", repoDirs)).toThrow(ValidationError);
  });

  test("bearer + omitted directory resolves to the repo clone", () => {
    expect(resolveSessionDirectory("bearer", code, undefined, repoDirs)).toBe("/home/user/git/code");
  });

  test("bearer + foreign directory is the documented deliberate choice", () => {
    expect(resolveSessionDirectory("bearer", code, "/tmp/ops-scratch", repoDirs)).toBe("/tmp/ops-scratch");
  });
});
