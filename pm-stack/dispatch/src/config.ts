// Environment configuration. Secrets are read by NAME from the environment
// only — never committed, never printed. Everything has a fixture-safe default.
import { homedir } from "node:os";
import { join } from "node:path";

const s = (v: string | undefined, d: string): string => (v && v.trim() !== "" ? v.trim() : d);

/** The six fleet repos dispatch may target. */
export const FLEET_REPOS = [
  "developing-today/code",
  "developing-today/opencode",
  "developing-today/openchamber",
  "dezren39/nix",
  "developing-today/zettel",
  "dezren39/dezren39",
] as const;

/** Repo -> Supabase queue name (repo-health pilot queues). */
export const REPO_QUEUES: Record<string, string> = {
  "developing-today/code": "repo-health-code-pilot",
  "developing-today/opencode": "repo-health-opencode-pilot",
  "developing-today/openchamber": "repo-health-openchamber-pilot",
  "dezren39/nix": "repo-health-nix-pilot",
  "developing-today/zettel": "repo-health-zettel-pilot",
  "dezren39/dezren39": "repo-health-dezren39-pilot",
};

/** Repo -> default session directory: the repo's laptop primary clone. */
export const REPO_DIRS: Record<string, string> = {
  "developing-today/code": "/home/user/git/code",
  "developing-today/opencode": "/home/user/git/opencode",
  "developing-today/openchamber": "/home/user/git/openchamber-fork",
  "dezren39/nix": "/home/user/git/nix",
  "developing-today/zettel": "/home/user/git/zettel",
  "dezren39/dezren39": "/home/user/dezren39",
};

/** Repos where the pm-stack GitHub App is installed (issue creation works):
 * the developing-today installation covers code, opencode and openchamber,
 * and the dezren39 installation covers dezren39/nix. developing-today/zettel
 * and dezren39/dezren39 are NOT covered — dispatch skips issue creation
 * there and returns a warning instead of failing. */
export const APP_INSTALLED_REPOS: ReadonlySet<string> = new Set([
  "developing-today/code",
  "developing-today/opencode",
  "developing-today/openchamber",
  "dezren39/nix",
]);

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
  return {
    port: Number(s(env.DISPATCH_PORT, "4879")),
    hosts: s(env.DISPATCH_HOSTS, "127.0.0.1")
      .split(",")
      .map((h) => h.trim())
      .filter(Boolean),
    buildVersion: s(env.DISPATCH_BUILD_VERSION, "dev"),
    apiToken: env.DISPATCH_API_TOKEN?.trim() ? env.DISPATCH_API_TOKEN.trim() : null,
    oidcAudience: s(env.DISPATCH_OIDC_AUDIENCE, "pm-dispatch"),
    repoAllowlist: allowlist,
    supabaseUrl: s(env.SUPABASE_URL, "https://nltgxorqwxhgfzseiirt.supabase.co").replace(/\/$/, ""),
    supabaseServiceRoleKey: env.SUPABASE_SERVICE_ROLE_KEY?.trim() ? env.SUPABASE_SERVICE_ROLE_KEY.trim() : null,
    opencodeUrl: s(env.OPENCODE_URL, "http://127.0.0.1:13420").replace(/\/$/, ""),
    opencodeAuthFile: s(
      env.OPENCODE_AUTH_FILE,
      join(homedir(), ".local", "state", "opencode", "service-prod.json"),
    ),
    githubAppId: env.GITHUB_APP_ID?.trim() ? env.GITHUB_APP_ID.trim() : null,
    githubAppPrivateKeyPem: appPem,
  };
}
