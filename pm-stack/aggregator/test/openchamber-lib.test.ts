import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "bun:test";
import { resolveLibDir, type GitRun } from "../src/openchamber-lib.ts";

const roots: string[] = [];
const REVISION = "a".repeat(40);

interface FixtureOptions {
  missingExport?: string;
  brokenDependencyExport?: string;
}

async function makeLib({ missingExport, brokenDependencyExport }: FixtureOptions = {}) {
  const root = await mkdtemp(join(tmpdir(), "openchamber-lib-test-"));
  roots.push(root);
  const dir = join(root, "packages/web/server/lib");
  await mkdir(join(dir, "quota/credentials"), { recursive: true });
  await mkdir(join(dir, "quota/providers"), { recursive: true });
  await writeFile(join(dir, "quota/index.js"), "export {};\n");

  const credentialExports = ["readManagedCredential", "writeManagedCredential"].filter((name) => name !== missingExport);
  const imports = brokenDependencyExport ? `import { ${brokenDependencyExport} } from "./auth.js";\nvoid ${brokenDependencyExport};\n` : "";
  await writeFile(
    join(dir, "quota/credentials/providers.js"),
    `${imports}${credentialExports.map((name) => `export function ${name}() { return null; }`).join("\n")}\n`,
  );
  await writeFile(join(dir, "quota/providers/opendesign.js"), "export async function fetchOpenDesignQuota() {}\n");
  await writeFile(join(dir, "quota/credentials/auth.js"), "export {};\n");
  return { root, dir };
}

function fakeGit(states: Array<{ root: string; dir: string; branch?: string; changes?: string }>): GitRun {
  return async (cwd, args) => {
    const state = states.find((candidate) => cwd === candidate.root || cwd === candidate.dir);
    if (!state) throw new Error(`unexpected git cwd: ${cwd}`);
    switch (args.join(" ")) {
      case "rev-parse --show-toplevel":
        return state.root;
      case "branch --show-current":
        return state.branch ?? "main";
      case "status --porcelain --untracked-files=normal":
        return state.changes ?? "";
      case "rev-parse --verify HEAD^{commit}":
        return REVISION;
      default:
        throw new Error(`unexpected git call: ${args.join(" ")}`);
    }
  };
}

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe("resolveLibDir", () => {
  test("skips a dirty candidate and logs the selected path and revision", async () => {
    const dirty = await makeLib();
    const clean = await makeLib();
    const messages: string[] = [];

    const resolved = await resolveLibDir([dirty.dir, clean.dir], {
      gitRun: fakeGit([
        { ...dirty, changes: "?? untracked.txt" },
        clean,
      ]),
      log: (message) => messages.push(message),
    });

    expect(resolved.dir).toBe(clean.dir);
    expect(resolved.revision).toBe(REVISION);
    expect(messages.some((message) => message.includes(`${dirty.dir}: checkout has a dirty working tree`))).toBe(true);
    expect(messages).toContain(`quota-gatherer: OpenChamber lib dir=${clean.dir} branch=main rev=${REVISION}`);
  });

  test("skips a candidate checked out on a non-main branch", async () => {
    const offMain = await makeLib();
    const clean = await makeLib();
    const messages: string[] = [];

    const resolved = await resolveLibDir([offMain.dir, clean.dir], {
      gitRun: fakeGit([{ ...offMain, branch: "queue/8357-224" }, clean]),
      log: (message) => messages.push(message),
    });

    expect(resolved.dir).toBe(clean.dir);
    expect(messages.some((message) => message.includes(`${offMain.dir}: checkout is on branch queue/8357-224, not main`))).toBe(true);
  });

  test("names a missing required export instead of exposing a bare import failure", async () => {
    const incomplete = await makeLib({ missingExport: "readManagedCredential" });
    const messages: string[] = [];

    await expect(
      resolveLibDir([incomplete.dir], {
        gitRun: fakeGit([incomplete]),
        log: (message) => messages.push(message),
      }),
    ).rejects.toThrow('missing required export "readManagedCredential"');

    expect(messages.some((message) => message.includes('missing required export "readManagedCredential"'))).toBe(true);
  });

  test("names a missing export imported by the required module", async () => {
    const incomplete = await makeLib({ brokenDependencyExport: "openCodeLocalCredentialSource" });

    await expect(
      resolveLibDir([incomplete.dir], {
        gitRun: fakeGit([incomplete]),
        log: () => {},
      }),
    ).rejects.toThrow('missing required export "openCodeLocalCredentialSource"');
  });
});
