import { describe, expect, test } from "bun:test";
import { loadConfig } from "../src/config.ts";

describe("loadConfig projector hot-list credentials", () => {
  test("trims the optional projector bearer token and leaves it unset by default", () => {
    expect(loadConfig({ PM_AGG_DEV: "1" }).projector.apiToken).toBeNull();
    expect(loadConfig({ PM_AGG_DEV: "1", PM_PROJECTOR_API_TOKEN: "  token  " }).projector.apiToken).toBe("token");
    expect(loadConfig({ PM_AGG_DEV: "1", PM_PROJECTOR_API_TOKEN: "   " }).projector.apiToken).toBeNull();
  });
});

describe("loadConfig PM_AGG_PORT", () => {
  test("keeps the existing default for unset and empty values", () => {
    expect(loadConfig({ PM_AGG_DEV: "1" }).port).toBe(4870);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_PORT: "" }).port).toBe(4870);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_PORT: "   " }).port).toBe(4870);
  });

  test("accepts decimal integer ports from 1 through 65535", () => {
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_PORT: "1" }).port).toBe(1);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_PORT: "4871" }).port).toBe(4871);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_PORT: "65535" }).port).toBe(65535);
  });

  test("rejects invalid values with the variable, value, and expected range", () => {
    for (const value of ["0", "abc", "12.5", "-1", "99999", "0x10", "1e3", "4870oops"]) {
      let thrown: unknown;
      try {
        loadConfig({ PM_AGG_DEV: "1", PM_AGG_PORT: value });
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

describe("loadConfig default Source gate (#1195)", () => {
  test("explicit live and explicit fixtures are honored as-is", () => {
    expect(loadConfig({ PM_AGG_SOURCE: "live" }).source).toBe("live");
    expect(loadConfig({ PM_AGG_SOURCE: "fixtures" }).source).toBe("fixtures");
    expect(loadConfig({ PM_AGG_SOURCE: "  live  " }).source).toBe("live");
  });

  test("unset, empty, or invalid PM_AGG_SOURCE refuses to start without the dev switch", () => {
    for (const env of [{}, { PM_AGG_SOURCE: "" }, { PM_AGG_SOURCE: "   " }, { PM_AGG_SOURCE: "bogus" }]) {
      let thrown: unknown;
      try {
        loadConfig(env);
      } catch (error) {
        thrown = error;
      }
      expect(thrown).toBeInstanceOf(Error);
      expect((thrown as Error).message).toContain("PM_AGG_SOURCE");
      expect((thrown as Error).message).toContain("PM_AGG_DEV=1");
    }
  });

  test("the dev switch allows the implicit fixture default", () => {
    for (const env of [
      { PM_AGG_DEV: "1" },
      { PM_AGG_DEV: "1", PM_AGG_SOURCE: "" },
      { PM_AGG_DEV: "1", PM_AGG_SOURCE: "bogus" },
    ]) {
      expect(loadConfig(env).source).toBe("fixtures");
    }
    // Explicit choices still win under the dev switch.
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_SOURCE: "live" }).source).toBe("live");
  });
});

describe("loadConfig PM_AGG_HOSTS", () => {
  test("keeps the existing default for unset and empty values", () => {
    expect(loadConfig({ PM_AGG_DEV: "1" }).hosts).toEqual(["127.0.0.1"]);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_HOSTS: "" }).hosts).toEqual(["127.0.0.1"]);
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_HOSTS: "   " }).hosts).toEqual(["127.0.0.1"]);
  });

  test("trims and preserves configured hosts", () => {
    expect(loadConfig({ PM_AGG_DEV: "1", PM_AGG_HOSTS: "localhost, 127.0.0.1" }).hosts).toEqual([
      "localhost",
      "127.0.0.1",
    ]);
  });

  test("rejects lists that contain no non-empty host", () => {
    for (const value of [",", " , , "]) {
      let thrown: unknown;
      try {
        loadConfig({ PM_AGG_DEV: "1", PM_AGG_HOSTS: value });
      } catch (error) {
        thrown = error;
      }

      expect(thrown).toBeInstanceOf(Error);
      expect((thrown as Error).message).toContain("PM_AGG_HOSTS");
      expect((thrown as Error).message).toContain(JSON.stringify(value));
    }
  });
});
