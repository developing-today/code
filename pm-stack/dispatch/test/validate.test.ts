import { describe, expect, test } from "bun:test";
import {
  ValidationError,
  parseModel,
  parseSessionRequest,
  parseTaskRequest,
  resolvePriority,
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
  test("sessions require repo and prompt; defaults issue+wait on", () => {
    const r = parseSessionRequest({ repo: "developing-today/code", prompt: "do the thing" });
    expect(r.issue).toBe(true);
    expect(r.wait).toBe(true);
    expect(r.priority).toBe(40);
    expect(() => parseSessionRequest({ prompt: "x" })).toThrow(/repo/);
    expect(() => parseSessionRequest({ repo: "developing-today/code" })).toThrow(/prompt/);
    expect(() => parseSessionRequest("nope")).toThrow(ValidationError);
  });

  test("tasks accept prompt or body, default kind triage and issue off", () => {
    const a = parseTaskRequest({ repo: "developing-today/code", title: "t", prompt: "p" });
    expect(a.kind).toBe("triage");
    expect(a.issue).toBe(false);
    const b = parseTaskRequest({ repo: "developing-today/code", title: "t", body: "p", kind: "small_fix" });
    expect(b.prompt).toBe("p");
    expect(b.kind).toBe("small_fix");
    expect(() => parseTaskRequest({ repo: "developing-today/code", title: "t" })).toThrow(/prompt/);
    expect(() => parseTaskRequest({ repo: "developing-today/code", title: "t", prompt: "p", kind: "bogus" })).toThrow(/kind/);
  });

  test("titleFromPrompt takes the first non-empty line, capped", () => {
    expect(titleFromPrompt("\n\n  fix the thing  \nmore")).toBe("fix the thing");
    expect(titleFromPrompt("x".repeat(200)).length).toBe(120);
  });
});
