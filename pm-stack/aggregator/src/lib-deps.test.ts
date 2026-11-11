import { expect, test } from "bun:test";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkLibDependencies, ensureLibDependencies, findLibCheckoutRoot } from "./lib-deps.ts";

async function fakeCheckout(opts: { installed?: boolean } = {}): Promise<{ root: string; libDir: string }> {
  const root = await mkdtemp(join(tmpdir(), "lib-deps-checkout-"));
  const libDir = join(root, "packages/web/server/lib");
  await mkdir(libDir, { recursive: true });
  await writeFile(join(root, "package.json"), JSON.stringify({ dependencies: { yaml: "^2.8.1", "@scope/pkg": "1.0.0" } }), "utf8");
  await writeFile(join(root, "bun.lock"), "", "utf8");
  if (opts.installed) {
    await mkdir(join(root, "node_modules/yaml"), { recursive: true });
    await mkdir(join(root, "node_modules/@scope/pkg"), { recursive: true });
  }
  return { root, libDir };
}

test("findLibCheckoutRoot walks up to the manifest+lockfile ancestor", async () => {
  const { root, libDir } = await fakeCheckout();
  try {
    expect(findLibCheckoutRoot(libDir)).toBe(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("checkLibDependencies reports declared-but-uninstalled packages, scoped included", async () => {
  const { root, libDir } = await fakeCheckout();
  try {
    const check = checkLibDependencies(libDir);
    expect(check.root).toBe(root);
    expect(check.missing.sort()).toEqual(["@scope/pkg", "yaml"]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("checkLibDependencies is clean when node_modules carries the declared packages", async () => {
  const { root, libDir } = await fakeCheckout({ installed: true });
  try {
    expect(checkLibDependencies(libDir).missing).toEqual([]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("ensure installs exactly once when packages are missing and re-checks clean", async () => {
  const { root, libDir } = await fakeCheckout();
  try {
    const installs: string[] = [];
    const logs: string[] = [];
    const result = await ensureLibDependencies({
      libDir,
      log: (line) => logs.push(line),
      install: async (r) => {
        installs.push(r);
        await mkdir(join(r, "node_modules/yaml"), { recursive: true });
        await mkdir(join(r, "node_modules/@scope/pkg"), { recursive: true });
      },
    });
    expect(installs).toEqual([root]);
    expect(result.missing).toEqual([]);
    expect(logs.join(" ")).toContain("yaml");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("ensure does not install when everything already resolves", async () => {
  const { root, libDir } = await fakeCheckout({ installed: true });
  try {
    let installs = 0;
    const result = await ensureLibDependencies({ libDir, install: async () => { installs++; } });
    expect(installs).toBe(0);
    expect(result.missing).toEqual([]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("ensure throws an explicit error naming packages still missing after install", async () => {
  const { root, libDir } = await fakeCheckout();
  try {
    await expect(ensureLibDependencies({ libDir, install: async () => {} })).rejects.toThrow("still missing after install");
    await expect(ensureLibDependencies({ libDir, install: async () => {} })).rejects.toThrow("yaml");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("ensure is a no-op outside an installable checkout", async () => {
  const dir = await mkdtemp(join(tmpdir(), "lib-deps-nocheckout-"));
  try {
    let installs = 0;
    const result = await ensureLibDependencies({ libDir: dir, install: async () => { installs++; } });
    expect(result.root).toBeNull();
    expect(installs).toBe(0);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
