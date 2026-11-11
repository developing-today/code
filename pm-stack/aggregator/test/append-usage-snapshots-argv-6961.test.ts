// Pinning test for #6961: append-usage-snapshots must reject unknown or
// misspelled arguments with a usage error (exit 2) BEFORE any file is
// read or written — a typo'd --from-provider-usage used to fall through
// to the default derivation mode and lose (or replace) the whole run's
// records with exit 0, an unknown flag was silently ignored, and a
// typo'd --byy silently discarded the requested observedBy. Valid
// invocations are unchanged (see also the #3295 --by pinning test).
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/append-usage-snapshots.ts");

async function runScript(dir: string, args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: {
      ...process.env,
      QUOTA_VIEW_PATH: join(dir, "quota-view.md"),
      QUOTA_REGISTRY_PATH: join(dir, "registry.md"),
      PROVIDER_USAGE_SNAPSHOT_PATH: join(dir, "provider-usage.json"),
      USAGE_SNAPSHOT_OUTBOX_PATH: join(dir, "outbox.jsonl"),
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("append-usage-snapshots unknown arguments (#6961)", () => {
  test("unknown or misspelled arguments exit 2, name the token, and leave the outbox untouched", async () => {
    const dir = await mkdtemp(join(tmpdir(), "append-argv-6961-"));
    try {
      const outbox = join(dir, "outbox.jsonl");
      await writeFile(outbox, "sentinel\n");
      // A provider-usage snapshot that WOULD append a record if the mode
      // flag were honored: the typo'd run must refuse, not silently
      // derive from the (missing) quota-view instead.
      await writeFile(
        join(dir, "provider-usage.json"),
        JSON.stringify({ generated_at: "2026-11-07T00:00:00Z", sessions: [{}, {}] }),
      );
      const badArgSets: { args: string[]; token: string }[] = [
        { args: ["--from-provider-usag"], token: "--from-provider-usag" },
        { args: ["--bogus-flag"], token: "--bogus-flag" },
        { args: ["--from-provider-usage", "--bogus-flag"], token: "--bogus-flag" },
        { args: ["--byy", "alice"], token: "--byy" },
        { args: ["--from-provider-usage", "--byy", "alice"], token: "--byy" },
        { args: ["stray-positional"], token: "stray-positional" },
      ];
      for (const { args, token } of badArgSets) {
        const { code, stderr } = await runScript(dir, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr).toContain(`unknown argument '${token}'`);
        expect(await readFile(outbox, "utf8")).toBe("sentinel\n");
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 30000);

  test("the #3295 --by errors still win for the token consumed as --by's value", async () => {
    const dir = await mkdtemp(join(tmpdir(), "append-argv-6961-by-"));
    try {
      // "--from-provider-usage" in the --by value slot is judged by
      // resolveObservedBy, not re-interpreted as the mode flag and not
      // reported as an unknown argument.
      const { code, stderr } = await runScript(dir, ["--by", "--from-provider-usage"]);
      expect(code).toBe(2);
      expect(stderr).toContain("--by requires a non-empty name");
      expect(stderr).not.toContain("unknown argument");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 30000);
});
