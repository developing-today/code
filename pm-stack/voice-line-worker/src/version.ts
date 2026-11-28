// The /version answer (#15793 criterion 1): the commit this Worker was
// deployed from, as stamped in the VERSION var at build time (see
// cloudflare.config.ts). It reads no state and no secret, so it answers
// ahead of every gate; "unknown" when no build stamped it, which the
// edge probe records honestly rather than guessing.
export function versionAnswer(env: { VERSION?: string }): { ok: true; service: string; version: string } {
  return { ok: true, service: "voice-line", version: env.VERSION ?? "unknown" };
}
