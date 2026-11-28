import { describe, expect, test } from "bun:test";
import { parseForecastAdmissionConfig } from "../src/quota-admission-forecast-config.ts";

// Oracle: Stage-3 design D4 (#12698) — live forecast admission enters
// behind a backend-style flag in the PM_AGG_STORE_BACKEND posture:
// default is today's behaviour, the flag names ONE pool and the
// configured margin, and flipping the flag back is the whole rollback.
// Invalid configuration fails loudly, never silently defaults.
describe("forecast admission flag: oracle = design D4 flag posture", () => {
  test("unset flag is off — today's behaviour is the default", () => {
    expect(parseForecastAdmissionConfig({})).toEqual({ enabled: false, providerId: null, marginMs: null });
  });

  test("explicit off is off even with pool and margin present", () => {
    const c = parseForecastAdmissionConfig({
      PM_AGG_FORECAST_ADMISSION: "off",
      PM_AGG_FORECAST_ADMISSION_POOL: "opendesign",
      PM_AGG_FORECAST_ADMISSION_MARGIN_MS: "3600000",
    });
    expect(c.enabled).toBe(false);
  });

  test("on requires a named pool and a positive margin", () => {
    expect(() => parseForecastAdmissionConfig({ PM_AGG_FORECAST_ADMISSION: "on" })).toThrow();
    expect(() =>
      parseForecastAdmissionConfig({ PM_AGG_FORECAST_ADMISSION: "on", PM_AGG_FORECAST_ADMISSION_POOL: "opendesign" }),
    ).toThrow();
    expect(() =>
      parseForecastAdmissionConfig({
        PM_AGG_FORECAST_ADMISSION: "on",
        PM_AGG_FORECAST_ADMISSION_POOL: "opendesign",
        PM_AGG_FORECAST_ADMISSION_MARGIN_MS: "0",
      }),
    ).toThrow();
    expect(() =>
      parseForecastAdmissionConfig({
        PM_AGG_FORECAST_ADMISSION: "on",
        PM_AGG_FORECAST_ADMISSION_POOL: "opendesign",
        PM_AGG_FORECAST_ADMISSION_MARGIN_MS: "abc",
      }),
    ).toThrow();
  });

  test("on with pool and margin parses", () => {
    expect(
      parseForecastAdmissionConfig({
        PM_AGG_FORECAST_ADMISSION: "on",
        PM_AGG_FORECAST_ADMISSION_POOL: "opendesign",
        PM_AGG_FORECAST_ADMISSION_MARGIN_MS: "3600000",
      }),
    ).toEqual({ enabled: true, providerId: "opendesign", marginMs: 3_600_000 });
  });

  test("an unrecognized flag value fails loudly", () => {
    expect(() => parseForecastAdmissionConfig({ PM_AGG_FORECAST_ADMISSION: "yes" })).toThrow(
      /PM_AGG_FORECAST_ADMISSION/,
    );
  });
});
