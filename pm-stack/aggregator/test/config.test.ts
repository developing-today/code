import { describe, expect, test } from "bun:test";
import { loadConfig } from "../src/config.ts";

describe("loadConfig PM_AGG_PORT", () => {
  test("keeps the existing default for unset and empty values", () => {
    expect(loadConfig({}).port).toBe(4870);
    expect(loadConfig({ PM_AGG_PORT: "" }).port).toBe(4870);
    expect(loadConfig({ PM_AGG_PORT: "   " }).port).toBe(4870);
  });

  test("accepts decimal integer ports from 1 through 65535", () => {
    expect(loadConfig({ PM_AGG_PORT: "1" }).port).toBe(1);
    expect(loadConfig({ PM_AGG_PORT: "4871" }).port).toBe(4871);
    expect(loadConfig({ PM_AGG_PORT: "65535" }).port).toBe(65535);
  });

  test("rejects invalid values with the variable, value, and expected range", () => {
    for (const value of ["0", "abc", "12.5", "-1", "99999", "0x10", "1e3", "4870oops"]) {
      let thrown: unknown;
      try {
        loadConfig({ PM_AGG_PORT: value });
      } catch (error) {
        thrown = error;
      }

      expect(thrown).toBeInstanceOf(Error);
      expect((thrown as Error).message).toContain("PM_AGG_PORT");
      expect((thrown as Error).message).toContain(JSON.stringify(value));
      expect((thrown as Error).message).toContain("1-65535");
    }
  });
});

describe("loadConfig PM_AGG_HOSTS", () => {
  test("keeps the existing default for unset and empty values", () => {
    expect(loadConfig({}).hosts).toEqual(["127.0.0.1"]);
    expect(loadConfig({ PM_AGG_HOSTS: "" }).hosts).toEqual(["127.0.0.1"]);
    expect(loadConfig({ PM_AGG_HOSTS: "   " }).hosts).toEqual(["127.0.0.1"]);
  });

  test("trims and preserves configured hosts", () => {
    expect(loadConfig({ PM_AGG_HOSTS: "localhost, 127.0.0.1" }).hosts).toEqual([
      "localhost",
      "127.0.0.1",
    ]);
  });

  test("rejects lists that contain no non-empty host", () => {
    for (const value of [",", " , , "]) {
      let thrown: unknown;
      try {
        loadConfig({ PM_AGG_HOSTS: value });
      } catch (error) {
        thrown = error;
      }

      expect(thrown).toBeInstanceOf(Error);
      expect((thrown as Error).message).toContain("PM_AGG_HOSTS");
      expect((thrown as Error).message).toContain(JSON.stringify(value));
    }
  });
});
