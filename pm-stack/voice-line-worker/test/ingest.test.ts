import { describe, expect, test } from "bun:test";
import { parseIngestStatus } from "../src/ingest";

describe("parseIngestStatus (#11014)", () => {
  test("refuses a body that is not JSON", () => {
    expect(parseIngestStatus("{not json")).toEqual({ ok: false, error: "invalid_json" });
    expect(parseIngestStatus("")).toEqual({ ok: false, error: "invalid_json" });
  });

  test("refuses JSON that is not a plain object, null included", () => {
    for (const raw of ["null", "[]", "3", '"x"', "true"]) {
      expect(parseIngestStatus(raw)).toEqual({ ok: false, error: "expected_object" });
    }
  });

  test("refuses a string, negative, fractional or non-finite count, naming the field", () => {
    expect(parseIngestStatus('{"queued":"abc"}')).toEqual({ ok: false, error: "invalid_count", field: "queued" });
    expect(parseIngestStatus('{"claimed":"3"}')).toEqual({ ok: false, error: "invalid_count", field: "claimed" });
    expect(parseIngestStatus('{"parked":-4}')).toEqual({ ok: false, error: "invalid_count", field: "parked" });
    expect(parseIngestStatus('{"blocked":1.5}')).toEqual({ ok: false, error: "invalid_count", field: "blocked" });
    expect(parseIngestStatus('{"queued":1e300}')).toEqual({ ok: false, error: "invalid_count", field: "queued" });
    expect(parseIngestStatus('{"queued":true}')).toEqual({ ok: false, error: "invalid_count", field: "queued" });
  });

  test("defaults missing or null counts to zero", () => {
    expect(parseIngestStatus("{}")).toEqual({ ok: true, snapshot: { queued: 0, claimed: 0, parked: 0, blocked: 0 } });
    expect(parseIngestStatus('{"queued":null}')).toEqual({
      ok: true,
      snapshot: { queued: 0, claimed: 0, parked: 0, blocked: 0 },
    });
  });

  test("accepts a valid body and stores exactly the four counts", () => {
    expect(parseIngestStatus('{"queued":12,"claimed":3,"parked":0,"blocked":1,"extra":"x"}')).toEqual({
      ok: true,
      snapshot: { queued: 12, claimed: 3, parked: 0, blocked: 1 },
    });
  });
});
