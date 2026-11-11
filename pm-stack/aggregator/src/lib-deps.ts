/**
 * OpenChamber lib dependency preflight (#8759).
 *
 * The quota gatherer's OpenDesign keeper dynamically imports modules
 * from the OpenChamber lib checkout named by OPENCHAMBER_LIB_DIR.
 * That checkout is a plain git clone: when it is (re)created or
 * refreshed without an install step, its declared dependencies are
 * absent (no node_modules) and every import fails with
 * "Cannot find package '<name>'" — the keeper then failed-soft on
 * every tick while the tick itself reported healthy, so the
 * OpenDesign renewal door was silently dead from 2026-11-09 until
 * #8759 was filed.
 *
 * Why a preflight, not import-retry: Bun caches a failed package
 * resolution for the life of the process (verified on bun 1.4.4 —
 * a retry import after installing still throws the cached
 * ResolveMessage), so install-then-retry inside one tick cannot
 * work. Instead, BEFORE the first import: walk the manifests from
 * the lib dir up to the checkout root, check every declared package
 * resolves from the lib dir, and if any are missing install the
 * checkout's declared dependencies once (bun install at the root)
 * and re-check. A dependency absence is then a logged recovery on
 * the tick that fixes it — or, when the install cannot fix it, an
 * explicit error naming the still-missing packages, never a silent
 * failed-soft behind a healthy tick.
 */
import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

const LOCKFILES = ["bun.lock", "bun.lockb", "package-lock.json", "pnpm-lock.yaml", "yarn.lock"];
const DEPENDENCY_FIELDS = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"];

/** Walk up from libDir to the checkout root: the first ancestor that
 * carries both a package.json and a lockfile. Null when no ancestor
 * qualifies (libDir is not inside an installable checkout). */
export function findLibCheckoutRoot(libDir: string, exists: (path: string) => boolean = existsSync): string | null {
  let dir = libDir;
  for (let depth = 0; depth < 10; depth++) {
    if (exists(join(dir, "package.json")) && LOCKFILES.some((lock) => exists(join(dir, lock)))) return dir;
    const parent = dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
  return null;
}

/** Directory chain from libDir up to and including root. */
function chainToRoot(libDir: string, root: string): string[] {
  const chain: string[] = [];
  let dir = libDir;
  for (let depth = 0; depth < 12; depth++) {
    chain.push(dir);
    if (dir === root) break;
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return chain;
}

function declaredInManifest(path: string, readFile: (path: string) => string): string[] {
  try {
    const manifest = JSON.parse(readFile(path)) as Record<string, unknown>;
    const names: string[] = [];
    for (const field of DEPENDENCY_FIELDS) {
      const deps = manifest[field];
      if (deps && typeof deps === "object") names.push(...Object.keys(deps as Record<string, unknown>));
    }
    return names;
  } catch {
    return [];
  }
}

export interface LibDependencyCheck {
  root: string | null;
  /** Declared packages (root + workspace manifests on the lib path)
   * that do not resolve from libDir. Empty when the root is unknown:
   * the preflight does not apply and imports proceed as before. */
  missing: string[];
}

/** Which declared packages are unresolvable from libDir right now.
 * Oracle: a package resolves iff node_modules/<name> exists in some
 * directory on the libDir-to-root chain (the same walk Node/Bun
 * resolution performs for these checkouts). */
export function checkLibDependencies(
  libDir: string,
  io: { exists?: (path: string) => boolean; readFile?: (path: string) => string } = {},
): LibDependencyCheck {
  const exists = io.exists ?? existsSync;
  const readFile = io.readFile ?? ((path: string) => readFileSync(path, "utf8"));
  const root = findLibCheckoutRoot(libDir, exists);
  if (!root) return { root: null, missing: [] };
  const chain = chainToRoot(libDir, root);
  const declared = new Set<string>();
  for (const dir of chain) {
    for (const name of declaredInManifest(join(dir, "package.json"), readFile)) declared.add(name);
  }
  const missing = [...declared].filter((name) => !chain.some((dir) => exists(join(dir, "node_modules", name))));
  return { root, missing };
}

export interface EnsureLibDependenciesOptions {
  libDir: string;
  /** Install the checkout root's declared dependencies. Default:
   *  at the root (the gatherer runs under bun). */
  install?: (root: string) => Promise<void>;
  log?: (line: string) => void;
  exists?: (path: string) => boolean;
  readFile?: (path: string) => string;
}

const INSTALL_TIMEOUT_MS = 240_000;

async function defaultInstall(root: string): Promise<void> {
  if (!basename(process.execPath).includes("bun")) {
    throw new Error("lib dependency preflight needs bun on the exec path to install");
  }
  await new Promise<void>((resolve, reject) => {
    execFile(process.execPath, ["install"], { cwd: root, timeout: INSTALL_TIMEOUT_MS, maxBuffer: 16 * 1024 * 1024 }, (err) => {
      if (err) reject(new Error("bun install failed in the OpenChamber lib checkout"));
      else resolve();
    });
  });
}

/**
 * Make the lib checkout's declared dependencies resolvable, or throw
 * an explicit error naming what is still missing. Installs at most
 * once, and only when something declared is actually missing.
 */
export async function ensureLibDependencies(options: EnsureLibDependenciesOptions): Promise<LibDependencyCheck> {
  const io = { exists: options.exists, readFile: options.readFile };
  const before = checkLibDependencies(options.libDir, io);
  if (!before.root || before.missing.length === 0) return before;
  options.log?.(
    "opendesign-renewal: lib dependencies missing in " + before.root + " (" + before.missing.join(", ") +
      "); installing declared dependencies and re-checking",
  );
  await (options.install ?? defaultInstall)(before.root);
  const after = checkLibDependencies(options.libDir, io);
  if (after.missing.length > 0) {
    throw new Error(
      "OpenChamber lib dependencies still missing after install in " + before.root + ": " + after.missing.join(", "),
    );
  }
  return after;
}
