// Live forecast-admission switch for Stage-3 S4 (#12698, epic
// #12694; design doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3,
// decision D4), in the PM_AGG_STORE_BACKEND posture (store-backend.ts):
//
//   PM_AGG_FORECAST_ADMISSION        "off" (default) | "on"
//   PM_AGG_FORECAST_ADMISSION_POOL   canonical provider id of the ONE
//                                    pool forecast admission is live
//                                    for (required when on)
//   PM_AGG_FORECAST_ADMISSION_MARGIN_MS
//                                    a projected wall at least this
//                                    many ms before the window reset
//                                    yields pace_hold (required when
//                                    on; positive integer)
//
// The default is today's behaviour: no forecast input reaches the
// admission projection. Flipping the flag back to "off" is the whole
// rollback — no deploy, no data change. Pool and margin are
// configuration set from the S2 calibration (#12696); once live they
// change only via the design's proposal mechanism (D5). An assembler
// that supplies forecast rows to buildQuotaAdmission reads this
// config; the projection itself stays pure and env-free. Any invalid
// value fails loudly, never silently defaults.

export interface ForecastAdmissionConfig {
  enabled: boolean;
  /** Canonical provider id of the one live pool; null while off. */
  providerId: string | null;
  /** Configured margin in ms; null while off. */
  marginMs: number | null;
}

export function parseForecastAdmissionConfig(
  env: Record<string, string | undefined> = process.env,
): ForecastAdmissionConfig {
  const raw = (env.PM_AGG_FORECAST_ADMISSION ?? "").trim();
  if (raw === "" || raw === "off") return { enabled: false, providerId: null, marginMs: null };
  if (raw !== "on") {
    throw new Error(
      `Invalid PM_AGG_FORECAST_ADMISSION=${JSON.stringify(env.PM_AGG_FORECAST_ADMISSION)}; expected "off" or "on"`,
    );
  }
  const providerId = (env.PM_AGG_FORECAST_ADMISSION_POOL ?? "").trim();
  if (!providerId) {
    throw new Error("PM_AGG_FORECAST_ADMISSION=on requires PM_AGG_FORECAST_ADMISSION_POOL (the one live pool)");
  }
  const marginRaw = (env.PM_AGG_FORECAST_ADMISSION_MARGIN_MS ?? "").trim();
  const marginMs = Number(marginRaw);
  if (!marginRaw || !Number.isInteger(marginMs) || marginMs <= 0) {
    throw new Error(
      `Invalid PM_AGG_FORECAST_ADMISSION_MARGIN_MS=${JSON.stringify(env.PM_AGG_FORECAST_ADMISSION_MARGIN_MS)}; expected a positive integer (ms)`,
    );
  }
  return { enabled: true, providerId, marginMs };
}
