import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type { OpenDesignCredentialLike } from "./opendesign-renewal.ts";

export type GitRun = (cwd: string, args: string[]) => Promise<string>;

export interface OpenChamberCredentialsModule {
  readManagedCredential: (providerId: string) => OpenDesignCredentialLike | null;
  writeManagedCredential: (providerId: string, value: OpenDesignCredentialLike) => unknown;
}

export interface OpenChamberProviderModule {
  fetchOpenDesignQuota: (credential: OpenDesignCredentialLike) => Promise<unknown>;
}

export interface ResolvedOpenChamberLib {
  dir: string;
  root: string;
  revision: string;
  credentials: OpenChamberCredentialsModule;
  provider: OpenChamberProviderModule;
}

export interface ResolveLibOptions {
  gitRun?: GitRun;
  log?: (message: string) => void;
}

const REQUIRED_MODULE_EXPORTS = [
  {
    path: "quota/credentials/providers.js",
    names: ["readManagedCredential", "writeManagedCredential"],
  },
  { path: "quota/providers/opendesign.js", names: ["fetchOpenDesignQuota"] },
] as const;

const defaultGitRun: GitRun = (cwd, args) =>
  new Promise((resolveOutput, reject) => {
    execFile("git", args, { cwd, timeout: 10_000, maxBuffer: 1 << 20 }, (err, stdout) => {
      if (err) reject(err);
      else resolveOutput(stdout.trim());
    });
  });

async function importRequiredModule(dir: string, path: string): Promise<Record<string, unknown>> {
  try {
    return (await import(pathToFileURL(resolve(dir, path)).href)) as Record<string, unknown>;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    const missing =
      message.match(/does not provide an export named ['"]([^'"]+)['"]/i) ??
      message.match(/export named ['"]([^'"]+)['"] not found/i);
    if (missing) {
      throw new Error(`missing required export "${missing[1]}" while importing ${path}`);
    }
    throw new Error(`could not import ${path}: ${message}`);
  }
}

async function validateExports(dir: string): Promise<{
  credentials: OpenChamberCredentialsModule;
  provider: OpenChamberProviderModule;
}> {
  const [credentials, provider] = await Promise.all(
    REQUIRED_MODULE_EXPORTS.map(({ path }) => importRequiredModule(dir, path)),
  );
  const modules = [credentials, provider];
  for (let index = 0; index < REQUIRED_MODULE_EXPORTS.length; index += 1) {
    const required = REQUIRED_MODULE_EXPORTS[index];
    const module = modules[index];
    for (const name of required.names) {
      if (typeof module[name] !== "function") {
        throw new Error(`missing required export "${name}" in ${required.path}`);
      }
    }
  }
  return {
    credentials: credentials as unknown as OpenChamberCredentialsModule,
    provider: provider as unknown as OpenChamberProviderModule,
  };
}

async function inspectCandidate(dir: string, gitRun: GitRun): Promise<ResolvedOpenChamberLib> {
  if (!existsSync(resolve(dir, "quota/index.js"))) {
    throw new Error("quota/index.js is missing");
  }

  const root = (await gitRun(dir, ["rev-parse", "--show-toplevel"])).trim();
  if (!root) throw new Error("Git returned an empty checkout root");

  const branch = (await gitRun(root, ["branch", "--show-current"])).trim();
  if (branch !== "main") {
    throw new Error(`checkout is on ${branch ? `branch ${branch}` : "a detached HEAD"}, not main`);
  }

  const changes = (await gitRun(root, ["status", "--porcelain", "--untracked-files=normal"])).trim();
  if (changes) throw new Error("checkout has a dirty working tree");

  const revision = (await gitRun(root, ["rev-parse", "--verify", "HEAD^{commit}"])).trim();
  if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(revision)) {
    throw new Error(`Git returned an invalid HEAD revision ${JSON.stringify(revision)}`);
  }

  const modules = await validateExports(dir);
  return { dir, root, revision, ...modules };
}

/** Pick the first clean main checkout with the credential APIs this gatherer uses. */
export async function resolveLibDir(
  candidates: string[],
  { gitRun = defaultGitRun, log = console.warn }: ResolveLibOptions = {},
): Promise<ResolvedOpenChamberLib> {
  const rejected: string[] = [];
  const seen = new Set<string>();

  for (const candidate of candidates) {
    const dir = resolve(candidate);
    if (seen.has(dir)) continue;
    seen.add(dir);
    try {
      const lib = await inspectCandidate(dir, gitRun);
      log(`quota-gatherer: OpenChamber lib dir=${lib.dir} branch=main rev=${lib.revision}`);
      return lib;
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      rejected.push(`${dir}: ${reason}`);
      log(`quota-gatherer: rejected OpenChamber lib ${dir}: ${reason}`);
    }
  }

  const detail = rejected.length ? rejected.join("; ") : "no candidates were configured";
  throw new Error(`no usable clean main OpenChamber lib checkout found (${detail})`);
}
