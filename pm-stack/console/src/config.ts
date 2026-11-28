// pm-console configuration. Numeric variables follow the pm-stack
// environment contract (pm-stack/README.md, "Environment numbers"):
// decimal digits only, range-checked, and a bad value stops the process
// at boot naming the variable.
import { readFileSync } from "node:fs";
import { machineName } from "./edge-proto.ts";

/** A token issuer whose signed tokens identify a caller. */
export interface Issuer {
  /** Short name shown in the audit log and on /api/me. */
  name: string;
  /** Exact `iss` claim. */
  issuer: string;
  /** Where the issuer publishes its signing keys. */
  jwksUri: string;
  /** Required `aud` value. */
  audience: string;
  /**
   * Where the token arrives. "access" is Cloudflare Access: the
   * Cf-Access-Jwt-Assertion header, or the CF_Authorization cookie.
   * "bearer" is `Authorization: Bearer <token>`, the form any other
   * OIDC issuer's ID token takes.
   */
  source: "access" | "bearer";
}

export interface Config {
  port: number;
  bind: string;
  /** Email addresses that are the owner, lower-cased. */
  owners: string[];
  /** Certificate common names that are the owner (client certificates
   * checked at the edge and reported in the token's `common_name`). */
  certNames: string[];
  issuers: Issuer[];
  repo: string;
  queue: string;
  queueDbUrl: string;
  queueTokenName: string;
  fleetBin: string;
  /** Glob patterns of the systemd user units the console may manage. */
  unitPatterns: string[];
  /** Units that may be restarted but never stopped from here: stopping
   * either one cuts off the interface that would start it again. */
  protectedUnits: string[];
  auditFile: string;
  /** The dashboard (pm-aggregator) this service fronts on the public
   * hostname, or null to serve /console alone. Loopback only. */
  dashboardUrl: string | null;
  /** Android app identity served at /.well-known/assetlinks.json. */
  androidPackage: string;
  androidFingerprints: string[];
  /** The signed APK the console hands to the signed-in owner, if the
   * file exists. */
  androidApk: string;
  /** Where linked phones are kept (hashes of their tokens, never the tokens). */
  devicesFile: string;
  /** The public address the edge answers on (https, no path), or null
   * when this host is not behind one and reports to nobody. */
  edgeUrl: string | null;
  /** This host's key pair for signing what it reports to the edge. */
  edgeKeyFile: string;
  /** What became of each action the edge held for this host. */
  edgeDoneFile: string;
  /** What this machine is called, where machines are listed. Empty when
   * the environment does not say: the host's own name is used then. */
  machineName: string;
  /** The dashboard's decision-set store: the questions waiting on the
   * owner, and the per-set tokens that authorise an answer. */
  decisionsFile: string;
  /** Label on an issue that is an instruction from the owner. */
  promptLabel: string;
  /** Repositories whose issues may be answered from "Needs me". */
  needsRepos: string[];
}

export class ConfigError extends Error {}

type Env = Record<string, string | undefined>;

function port(env: Env, name: string, fallback: number): number {
  const raw = env[name];
  if (raw === undefined) return fallback;
  if (!/^[0-9]+$/.test(raw)) throw new ConfigError(`${name}: not a decimal port: ${JSON.stringify(raw)}`);
  const n = Number(raw);
  if (n < 1 || n > 65535) throw new ConfigError(`${name}: port out of range 1-65535: ${raw}`);
  return n;
}

function list(env: Env, name: string, fallback: string[] = []): string[] {
  const raw = env[name];
  if (raw === undefined) return fallback;
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s !== "");
}

function issuerFrom(value: unknown, where: string): Issuer {
  const v = value as Record<string, unknown>;
  for (const key of ["name", "issuer", "jwksUri", "audience"]) {
    if (typeof v?.[key] !== "string" || v[key] === "") throw new ConfigError(`${where}: missing "${key}"`);
  }
  const source = v.source === undefined ? "bearer" : v.source;
  if (source !== "access" && source !== "bearer")
    throw new ConfigError(`${where}: source must be "access" or "bearer"`);
  for (const key of ["issuer", "jwksUri"]) {
    if (!/^https:\/\//.test(v[key] as string)) throw new ConfigError(`${where}: "${key}" must be an https URL`);
  }
  return {
    name: v.name as string,
    issuer: v.issuer as string,
    jwksUri: v.jwksUri as string,
    audience: v.audience as string,
    source,
  };
}

export function readConfig(
  env: Env = process.env,
  readFile: (p: string) => string = (p) => readFileSync(p, "utf8"),
): Config {
  const home = env.HOME ?? "";
  const issuers: Issuer[] = [];

  // Cloudflare Access, the edge sign-in. Both variables or neither.
  const team = env.PM_CONSOLE_ACCESS_TEAM_DOMAIN;
  const aud = env.PM_CONSOLE_ACCESS_AUD;
  if ((team === undefined) !== (aud === undefined)) {
    throw new ConfigError("PM_CONSOLE_ACCESS_TEAM_DOMAIN and PM_CONSOLE_ACCESS_AUD must be set together");
  }
  if (team !== undefined && aud !== undefined) {
    if (!/^[a-z0-9-]+\.cloudflareaccess\.com$/.test(team)) {
      throw new ConfigError(
        `PM_CONSOLE_ACCESS_TEAM_DOMAIN: not a cloudflareaccess.com team domain: ${JSON.stringify(team)}`,
      );
    }
    if (aud === "") throw new ConfigError("PM_CONSOLE_ACCESS_AUD: empty");
    issuers.push({
      name: "cloudflare-access",
      issuer: `https://${team}`,
      jwksUri: `https://${team}/cdn-cgi/access/certs`,
      audience: aud,
      source: "access",
    });
  }

  // Any further issuer (the central OIDC provider, Google directly) is
  // one entry in a JSON file: [{name, issuer, jwksUri, audience, source}].
  const file = env.PM_CONSOLE_ISSUERS_FILE;
  if (file !== undefined && file !== "") {
    let parsed: unknown;
    try {
      parsed = JSON.parse(readFile(file));
    } catch (error) {
      throw new ConfigError(`PM_CONSOLE_ISSUERS_FILE: cannot read ${file}: ${(error as Error).message}`);
    }
    if (!Array.isArray(parsed)) throw new ConfigError("PM_CONSOLE_ISSUERS_FILE: expected a JSON array");
    parsed.forEach((entry, i) => issuers.push(issuerFrom(entry, `PM_CONSOLE_ISSUERS_FILE[${i}]`)));
  }
  const names = issuers.map((i) => i.name);
  if (new Set(names).size !== names.length) throw new ConfigError("issuer names must be unique");

  const fingerprints = list(env, "PM_CONSOLE_ANDROID_SHA256");
  for (const f of fingerprints) {
    if (!/^([0-9A-F]{2}:){31}[0-9A-F]{2}$/.test(f)) {
      throw new ConfigError(
        `PM_CONSOLE_ANDROID_SHA256: not a colon-separated SHA-256 fingerprint: ${JSON.stringify(f)}`,
      );
    }
  }

  // Loopback only. A request with no token is trusted when it comes from
  // this machine, so the listener must not be reachable from another one.
  const bind = env.PM_CONSOLE_BIND ?? "127.0.0.1";
  if (bind !== "127.0.0.1" && bind !== "::1" && bind !== "localhost") {
    throw new ConfigError(`PM_CONSOLE_BIND: only a loopback address is supported, not ${JSON.stringify(bind)}`);
  }

  // The dashboard has no sign-in of its own. The console passes requests
  // to it only for a verified owner, so the address has to be one that
  // nothing but this machine can reach or answer.
  let dashboardUrl: string | null = env.PM_CONSOLE_DASHBOARD_URL ?? "http://127.0.0.1:4870";
  if (dashboardUrl === "") dashboardUrl = null;
  if (dashboardUrl !== null) {
    let parsed: URL | null = null;
    try {
      parsed = new URL(dashboardUrl);
    } catch {
      // reported below
    }
    const host = parsed?.hostname ?? "";
    if (
      !parsed ||
      parsed.protocol !== "http:" ||
      !(host === "127.0.0.1" || host === "localhost" || host === "[::1]") ||
      parsed.pathname !== "/" ||
      parsed.search !== "" ||
      parsed.username !== "" ||
      parsed.password !== ""
    ) {
      throw new ConfigError(
        `PM_CONSOLE_DASHBOARD_URL: expected http://127.0.0.1:<port> (loopback, no path), got ${JSON.stringify(dashboardUrl)}`,
      );
    }
    dashboardUrl = parsed.origin;
  }

  let edgeUrl: string | null = env.PM_CONSOLE_EDGE_URL ?? null;
  if (edgeUrl === "") edgeUrl = null;
  if (edgeUrl !== null) {
    let parsed: URL | null = null;
    try {
      parsed = new URL(edgeUrl);
    } catch {
      // reported below
    }
    if (
      !parsed ||
      parsed.protocol !== "https:" ||
      parsed.pathname !== "/" ||
      parsed.search !== "" ||
      parsed.hash !== "" ||
      parsed.username !== "" ||
      parsed.password !== ""
    ) {
      throw new ConfigError(
        `PM_CONSOLE_EDGE_URL: expected https://<host> with no path, got ${JSON.stringify(edgeUrl)}`,
      );
    }
    edgeUrl = parsed.origin;
  }

  return {
    port: port(env, "PM_CONSOLE_PORT", 4872),
    bind,
    owners: list(env, "PM_CONSOLE_OWNER_EMAILS").map((e) => e.toLowerCase()),
    certNames: list(env, "PM_CONSOLE_CERT_NAMES"),
    issuers,
    repo: env.PM_CONSOLE_REPO ?? "developing-today/code",
    queue: env.PM_CONSOLE_QUEUE ?? "repo-health-code-pilot",
    queueDbUrl: env.PM_CONSOLE_QUEUE_DB_URL ?? "https://fleet-queue-dezren39.aws-us-east-1.turso.io",
    queueTokenName: env.PM_CONSOLE_QUEUE_TOKEN_NAME ?? "turso.fleet-queue-rw-token",
    fleetBin: env.PM_CONSOLE_FLEET_BIN ?? "fleet",
    unitPatterns: list(env, "PM_CONSOLE_UNITS", [
      "pm-*",
      "opencode*",
      "openchamber*",
      "queue-broker*",
      "secrets-broker*",
      "fleet-*",
    ]),
    protectedUnits: list(env, "PM_CONSOLE_PROTECTED_UNITS", ["pm-tunnel.service", "pm-console.service"]),
    auditFile: env.PM_CONSOLE_AUDIT_FILE ?? `${home}/.local/state/pm-console/audit.jsonl`,
    dashboardUrl,
    androidPackage: env.PM_CONSOLE_ANDROID_PACKAGE ?? "today.developing.pmconsole",
    androidFingerprints: fingerprints,
    androidApk: env.PM_CONSOLE_ANDROID_APK ?? `${home}/.local/state/pm-console/android/pm-console.apk`,
    devicesFile: env.PM_CONSOLE_DEVICES_FILE ?? `${home}/.local/state/pm-console/devices.json`,
    edgeUrl,
    edgeKeyFile: env.PM_CONSOLE_EDGE_KEY_FILE ?? `${home}/.local/state/pm-console/edge-key.json`,
    edgeDoneFile: env.PM_CONSOLE_EDGE_DONE_FILE ?? `${home}/.local/state/pm-console/edge-done.json`,
    machineName: machineName(env.PM_CONSOLE_MACHINE),
    decisionsFile: env.PM_CONSOLE_DECISIONS_FILE ?? `${home}/.local/state/pm-aggregator/decisions.json`,
    promptLabel: env.PM_CONSOLE_PROMPT_LABEL ?? "owner/prompt",
    needsRepos: list(env, "PM_CONSOLE_NEEDS_REPOS", [
      "developing-today/code",
      "developing-today/opencode",
      "developing-today/openchamber",
      "dezren39/nix",
    ]),
  };
}
