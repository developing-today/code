// Environment configuration. Secrets are read by NAME from the environment
// only — never committed, never printed. Everything has a fixture-safe
// default except SUPABASE_URL, which is required: its previous default
// pointed at the deleted legacy muse-dev project (#5120), so a missing
// value now fails loudly at startup instead of silently queueing nowhere.
import { homedir } from "node:os";
import { join } from "node:path";

const s = (v: string | undefined, d: string): string => (v && v.trim() !== "" ? v.trim() : d);

/** The six fleet repos dispatch may target. */
export const FLEET_REPOS = [
  "developing-today/code",
  "developing-today-forks/opencode",
  "developing-today-forks/openchamber",
  "dezren39/darwin-nix",
  "developing-today/zettel",
  "dezren39/dezren39",
] as const;

/** Repo -> Supabase queue name (repo-health pilot queues). */
export const REPO_QUEUES: Record<string, string> = {
  "developing-today/code": "repo-health-code-pilot",
  "developing-today-forks/opencode": "repo-health-opencode-pilot",
  "developing-today-forks/openchamber": "repo-health-openchamber-pilot",
  "dezren39/darwin-nix": "repo-health-nix-pilot",
  "developing-today/zettel": "repo-health-zettel-pilot",
  "dezren39/dezren39": "repo-health-dezren39-pilot",
};

/** Repo -> default session directory: the repo's laptop primary clone. */
export const REPO_DIRS: Record<string, string> = {
  "developing-today/code": "/home/user/git/code",
  "developing-today-forks/opencode": "/home/user/git/opencode",
  "developing-today-forks/openchamber": "/home/user/git/openchamber-fork",
  "dezren39/darwin-nix": "/home/user/git/nix",
  "developing-today/zettel": "/home/user/git/zettel",
  "dezren39/dezren39": "/home/user/dezren39",
};

/** Repos where the pm-stack GitHub App is installed (issue creation works):
 * the developing-today installation covers code, and the dezren39
 * installation covers dezren39/darwin-nix. developing-today/zettel and
 * dezren39/dezren39 are NOT covered, and neither are opencode and
 * openchamber since their move to developing-today-forks (the app is not
 * installed on that org yet — grants item G2) — dispatch skips issue
 * creation in all of those and returns a warning instead of failing. */
export const APP_INSTALLED_REPOS: ReadonlySet<string> = new Set(["developing-today/code", "dezren39/darwin-nix"]);

export interface Config {
  port: number;
  /** Bind addresses (DISPATCH_HOSTS, comma-separated). Default loopback. */
  hosts: string[];
  buildVersion: string;
  /** Static bearer token (DISPATCH_API_TOKEN); null disables bearer auth. */
  apiToken: string | null;
  /** Expected OIDC audience for GitHub Actions tokens. */
  oidcAudience: string;
  /** Repos either auth form may dispatch for. */
  repoAllowlist: string[];
  supabaseUrl: string;
  supabaseServiceRoleKey: string | null;
  opencodeUrl: string;
  /** File holding the OpenCode server's state incl. its password (read at
   * runtime on the laptop; the value is never logged or stored here). */
  opencodeAuthFile: string;
  githubAppId: string | null;
  /** Decoded PEM for the GitHub App private key (memory only). */
  githubAppPrivateKeyPem: string | null;
}

export function loadConfig(env: Record<string, string | undefined> = process.env): Config {
  const portValue = s(env.DISPATCH_PORT, "4879");
  if (!/^\d+$/.test(portValue) || Number(portValue) < 1 || Number(portValue) > 65535) {
    throw new Error(`Invalid DISPATCH_PORT "${portValue}": expected a decimal integer from 1 to 65535`);
  }
  const port = Number(portValue);
  const rawHosts = env.DISPATCH_HOSTS && env.DISPATCH_HOSTS.trim() !== "" ? env.DISPATCH_HOSTS : "127.0.0.1";
  const hosts = rawHosts
    .split(",")
    .map((h) => h.trim())
    .filter(Boolean);
  if (hosts.length === 0) {
    throw new Error(`Invalid DISPATCH_HOSTS "${rawHosts}": expected at least one host`);
  }

  const allowlist = s(env.DISPATCH_REPO_ALLOWLIST, FLEET_REPOS.join(","))
    .split(",")
    .map((r) => r.trim())
    .filter(Boolean);
  let appPem: string | null = null;
  const appB64 = env.GITHUB_APP_PRIVATE_KEY_BASE64?.trim();
  if (appB64) {
    try {
      appPem = Buffer.from(appB64, "base64").toString("utf8");
    } catch {
      appPem = null;
    }
  }
  const supabaseUrl = env.SUPABASE_URL?.trim();
  if (!supabaseUrl) {
    throw new Error(
      "SUPABASE_URL is not set: pm-dispatch requires the fleet Supabase (muse-dev2) project URL. There is no default — the previous one pointed at the deleted legacy muse-dev project (#5120). The laptop deployment renders it from the pm-stack SOPS file (pm-stack/dispatch/install.sh).",
    );
  }
  return {
    port,
    hosts,
    buildVersion: s(env.DISPATCH_BUILD_VERSION, "dev"),
    apiToken: env.DISPATCH_API_TOKEN?.trim() ? env.DISPATCH_API_TOKEN.trim() : null,
    oidcAudience: s(env.DISPATCH_OIDC_AUDIENCE, "pm-dispatch"),
    repoAllowlist: allowlist,
    supabaseUrl: supabaseUrl.replace(/\/$/, ""),
    supabaseServiceRoleKey: env.SUPABASE_SERVICE_ROLE_KEY?.trim() ? env.SUPABASE_SERVICE_ROLE_KEY.trim() : null,
    opencodeUrl: s(env.OPENCODE_URL, "http://127.0.0.1:13420").replace(/\/$/, ""),
    opencodeAuthFile: s(env.OPENCODE_AUTH_FILE, join(homedir(), ".local", "state", "opencode", "service-prod.json")),
    githubAppId: env.GITHUB_APP_ID?.trim() ? env.GITHUB_APP_ID.trim() : null,
    githubAppPrivateKeyPem: appPem,
  };
}
