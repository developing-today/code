/**
 * The #15793 criterion-1 answer for atlas-search: the deployed commit
 * from the VERSION var (stamped per deploy at build time — see
 * cloudflare.config.ts), or "unknown" when the deploy was unstamped;
 * the edge probe records the commit only when one is named (commitOf in
 * pm-stack/console/edge/src/worker-checks.ts reads the `version` field).
 */
export function versionAnswer(env: { VERSION?: string }): { ok: true; service: string; version: string } {
  const version = typeof env.VERSION === "string" ? env.VERSION.trim() : "";
  return { ok: true, service: "atlas-search", version: version || "unknown" };
}
