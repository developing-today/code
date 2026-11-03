// Bitwarden Secrets Manager resolution by NAME — the same flow the
// fleet-feed collector uses (scripts/collect-fleet-feed.ts
// resolveBwsSecret), generalized for where the bws binary and the
// machine-account access-token file actually live on each host:
//
//   binary:     $BWS_BIN, else /run/current-system/sw/bin/bws (laptop
//               NixOS), else ~/tools/bin/bws (sandbox)
//   token file: $BWS_ACCESS_TOKEN_FILE, else ~/.config/bws/
//               access-token (laptop), else ~/.config/bitwarden/
//               access-token (sandbox)
//
// Secret values are never logged, never written to disk, and only ever
// returned to the caller that asked for them by name. Lookups fail
// soft (null) and are cached briefly so a tick loop does not spawn
// bws per call.
import { execFile } from "node:child_process";
import { access, readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

const HOME = homedir();
const CACHE_TTL_MS = 5 * 60_000;
const cache = new Map<string, { value: string | null; at: number }>();

async function firstReadable(paths: string[]): Promise<string | null> {
  for (const p of paths) {
    try {
      await access(p);
      return p;
    } catch {
      // try the next candidate
    }
  }
  return null;
}

async function resolveBwsSecretUncached(key: string): Promise<string | null> {
  const bwsBin =
    process.env.BWS_BIN ?? (await firstReadable([join("/run/current-system/sw/bin/bws"), join(HOME, "tools/bin/bws")]));
  const accessTokenFile =
    process.env.BWS_ACCESS_TOKEN_FILE ??
    (await firstReadable([join(HOME, ".config/bws/access-token"), join(HOME, ".config/bitwarden/access-token")]));
  if (!bwsBin || !accessTokenFile) return null;
  try {
    const accessToken = (await readFile(accessTokenFile, "utf8")).trim();
    if (!accessToken) return null;
    const run = (args: string[]): Promise<string> =>
      new Promise((resolve, reject) => {
        execFile(
          bwsBin,
          args,
          { env: { ...process.env, BWS_ACCESS_TOKEN: accessToken }, maxBuffer: 16 * 1024 * 1024, timeout: 30_000 },
          (err, stdout) => (err ? reject(err) : resolve(stdout)),
        );
      });
    const list = JSON.parse(await run(["secret", "list"])) as { id?: string; key?: string }[];
    const entry = list.find((s) => s.key === key);
    if (!entry?.id) return null;
    const got = JSON.parse(await run(["secret", "get", entry.id])) as { value?: string };
    return got.value && got.value.trim() ? got.value.trim() : null;
  } catch {
    return null; // bws unreachable/unreadable — fail soft, no details logged
  }
}

/** Resolve one Bitwarden Secrets Manager secret by key (cached ~5 min). */
export async function resolveBwsSecret(key: string): Promise<string | null> {
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < CACHE_TTL_MS) return hit.value;
  const value = await resolveBwsSecretUncached(key);
  cache.set(key, { value, at: Date.now() });
  return value;
}

/** Drop a cached secret (e.g. after an auth failure suggests rotation). */
export function forgetBwsSecret(key: string): void {
  cache.delete(key);
}
