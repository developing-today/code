import { describe, expect, test } from "bun:test";
import { parseControlFrame } from "../src/control";

describe("parseControlFrame", () => {
  test("reads the type of a JSON object frame", () => {
    expect(parseControlFrame('{"type":"start"}')).toEqual({ ok: true, type: "start" });
    expect(parseControlFrame('{"type":"stop"}')).toEqual({ ok: true, type: "stop" });
    expect(parseControlFrame("{}")).toEqual({ ok: true, type: undefined });
  });

  test("rejects text that is not JSON", () => {
    expect(parseControlFrame("stop")).toEqual({ ok: false, error: "control frame must be JSON" });
    expect(parseControlFrame("")).toEqual({ ok: false, error: "control frame must be JSON" });
  });

  test("rejects JSON that is not an object, null included, without throwing", () => {
    for (const frame of ["null", "1", '"stop"', "[]", "true"]) {
      expect(parseControlFrame(frame)).toEqual({
        ok: false,
        error: "control frame must be a JSON object",
      });
    }
  });
});
