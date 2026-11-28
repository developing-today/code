// Pinning test for #7048: check-contract-corpus must report a wrong-shape
// or non-JSON corpus file as a FAIL check naming the file and still finish
// with its "contract check FAILED" summary, instead of dying on a raw
// TypeError/SyntaxError that aborts every remaining section. Pre-fix, each
// mutation below crashed the script with a stack trace and no summary.
import { describe, expect, test } from "bun:test";
import { cp, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/check-contract-corpus.ts");
const shippedCorpus = join(import.meta.dir, "contract-corpus", "v1");

async function runScript(args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { PATH: process.env.PATH ?? "/run/current-system/sw/bin:/usr/bin:/bin" },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

async function mutatedCorpus(mutate: (dir: string) => Promise<void>): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "contract-corpus-7048-"));
  await cp(shippedCorpus, dir, { recursive: true });
  await mutate(dir);
  return dir;
}

async function writeJson(dir: string, name: string, value: unknown): Promise<void> {
  await writeFile(join(dir, name), JSON.stringify(value));
}

describe("check-contract-corpus corpus shape handling (#7048)", () => {
  test("wrong-shape corpus files are FAIL checks, the run completes with its summary", async () => {
    const cases: { name: string; mutate: (dir: string) => Promise<void>; failNeedle: string }[] = [
      { name: "projects-object", mutate: (d) => writeJson(d, "projects.json", {}), failNeedle: "corpus projects" },
      { name: "registry-object", mutate: (d) => writeJson(d, "registry.json", {}), failNeedle: "corpus registry" },
      { name: "quota-null", mutate: (d) => writeJson(d, "quota.json", null), failNeedle: "corpus quota" },
      { name: "quota-object", mutate: (d) => writeJson(d, "quota.json", {}), failNeedle: "corpus quota" },
      { name: "feed-null", mutate: (d) => writeJson(d, "feed.json", null), failNeedle: "corpus feed.json" },
      {
        name: "orchestration-null",
        mutate: (d) => writeJson(d, "orchestration.json", null),
        failNeedle: "corpus orchestration",
      },
      {
        name: "dns-not-json",
        mutate: async (d) => writeFile(join(d, "dns.json"), "not json{"),
        failNeedle: "corpus dns.json parses as JSON",
      },
    ];
    for (const c of cases) {
      const dir = await mutatedCorpus(c.mutate);
      const { code, stdout, stderr } = await runScript(["--corpus", dir]);
      const combined = stdout + stderr;
      expect(code, c.name).toBe(1);
      expect(combined, c.name).toContain("FAIL");
      expect(combined, c.name).toContain(c.failNeedle);
      expect(combined, c.name).toContain("contract check FAILED");
      // A caught error's name may appear inside a FAIL detail; what must
      // never appear is the uncaught-crash stack / Bun crash banner.
      expect(combined, c.name).not.toContain("at main (");
      expect(combined, c.name).not.toContain("Bun v");
      // Sections after the mutated file still ran: the failure summary is
      // only printed at the very end of main().
      expect(combined, c.name).toContain("contract check FAILED:");
    }
  }, 120000);

  test("two mutated files are both reported in one run", async () => {
    const dir = await mutatedCorpus(async (d) => {
      await writeJson(d, "projects.json", {});
      await writeJson(d, "quota.json", null);
    });
    const { code, stdout, stderr } = await runScript(["--corpus", dir]);
    const combined = stdout + stderr;
    expect(code).toBe(1);
    expect(combined).toContain("corpus projects");
    expect(combined).toContain("corpus quota");
    expect(combined).toContain("contract check FAILED");
    expect(combined).not.toContain("at main (");
    expect(combined).not.toContain("Bun v");
  }, 60000);

  test("the shipped corpus still passes end-to-end (control)", async () => {
    const { code, stdout } = await runScript(["--corpus", shippedCorpus]);
    expect(code).toBe(0);
    expect(stdout).toContain("contract check passed");
    // The fixture corpus used above is the real shipped one.
    expect(await readFile(join(shippedCorpus, "manifest.json"), "utf8")).toContain("contractVersion");
  }, 60000);
});
