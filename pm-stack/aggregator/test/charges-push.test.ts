import { test, expect } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Explicit timeout: five subprocess push invocations can exceed the bun default 5s per-test timeout on slower hosts (Codex P2 on PR #3167).
test("only a successfully installed feed confirms its charges", async () => {
  const dir = await mkdtemp(join(tmpdir(), "charges-push-"));
  const bin = join(dir, "bin");
  const state = join(dir, "state.json");
  const known = join(dir, "known-hosts");
  const realBun = process.execPath;
  // Resolve the real interpreter to an absolute path BEFORE the wrapper
  // directory leads the child PATH; a bare "python3" wrapper would exec
  // itself in a loop (Codex P2 on PR #3167).
  const pythonOverride = process.env.TEST_PYTHON3;
  const python = pythonOverride?.startsWith("/")
    ? pythonOverride
    : (Bun.which(pythonOverride ?? "python3") ?? pythonOverride ?? "python3");
  const charge = { normalized: "example", amountUsd: 12, at: "2026-10-01" };
  const script = join(import.meta.dir, "../scripts/push-fleet-feed.sh");
  try {
    await mkdir(bin);
    await writeFile(known, "pinned\n");
    // The fake collector selects from confirmed state, like the real collector.
    await writeFile(join(bin, "bun"), `#!/usr/bin/env bash\nif [[ "$2" == scripts/collect-fleet-feed.ts ]]; then\n  exec '${realBun}' '${join(dir, "collect.ts")}' "$4"\nfi\nexec '${realBun}' "$@"\n`, { mode: 0o755 });
    await writeFile(join(dir, "collect.ts"), `import {readFile,writeFile,appendFile} from 'node:fs/promises';\nlet keys=[]; try { keys=JSON.parse(await readFile(process.env.CHARGES_SURFACED_STATE_PATH,'utf8')).surfaced } catch {}\nconst fresh=!keys.includes('example|12.00|2026-10-01');\nawait appendFile(process.env.COLLECTION_LOG, fresh?'fresh\\n':'seen\\n');\nawait writeFile(process.argv[2],JSON.stringify({chargesAttention:fresh?[${JSON.stringify(charge)}]:[]}));\n`);
    await writeFile(join(bin, "python3"), `#!/usr/bin/env bash\nexec '${python}' "$@"\n`, { mode: 0o755 });
    await writeFile(join(bin, "scp"), `#!/usr/bin/env bash\n[[ "$FAIL_STEP" != scp ]]\n`, { mode: 0o755 });
    await writeFile(join(bin, "ssh"), `#!/usr/bin/env bash\n[[ "$FAIL_STEP" != ssh ]]\n`, { mode: 0o755 });
    const run = async (fail: string) => {
      const proc = Bun.spawn(["bash", script], { env: { ...process.env, PATH: `${bin}:${process.env.PATH ?? "/usr/bin:/bin"}`, HOME: dir, HTTPS_PROXY: "http://user:pass@localhost:3128", CHARGES_SURFACED_STATE_PATH: state, COLLECTION_LOG: join(dir, "collections"), PUSH_FLEET_FEED_KNOWN_HOSTS: known, FAIL_STEP: fail }, stdout: "pipe", stderr: "pipe" });
      const output = await new Response(proc.stderr).text();
      return { code: await proc.exited, output };
    };
    for (const step of ["scp", "ssh"]) {
      expect((await run(step)).code).not.toBe(0);
      expect(await Bun.file(state).exists()).toBe(false);
    }
    await writeFile(state, JSON.stringify({ version: 1, surfaced: ["previous"] }));
    const successes = await Promise.all([run(""), run("")]);
    expect(successes).toEqual([{ code: 0, output: "" }, { code: 0, output: "" }]);
    expect((await readFile(join(dir, "collections"), "utf8")).trim().split("\n")).toEqual(["fresh", "fresh", "fresh", "seen"]);
    expect(JSON.parse(await readFile(state, "utf8")).surfaced).toEqual(["previous", "example|12.00|2026-10-01"]);
    expect((await run("")).code).toBe(0);
    expect(JSON.parse(await readFile(state, "utf8")).surfaced).toEqual(["previous", "example|12.00|2026-10-01"]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}, 30000);
