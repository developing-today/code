// redeploy.sh, run for real against a small checkout made here.
//
// The script under test is this repository's own file, copied into a
// throwaway git repository shaped like the real one. Two things are
// stand-ins: the program that starts the console's self-check (a script
// that records what it was given and passes or fails as told) and
// install.sh (a script that records that it ran). Everything else, git
// included, is the real thing.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SCRIPT = join(import.meta.dir, "..", "redeploy.sh");
const UNITS = [
  "pm-console.service",
  "pm-console-expose.service",
  "pm-console-expose.timer",
  "pm-console-redeploy.service",
  "pm-console-redeploy.timer",
  "pm-console-android-release.service",
  "pm-console-android-release.timer",
];

let dir: string;
let repo: string;
let home: string;
let log: string;

function sh(cwd: string, ...argv: string[]) {
  const done = Bun.spawnSync(argv, { cwd, env: { ...process.env, HOME: home, GIT_CONFIG_GLOBAL: "/dev/null" } });
  return { code: done.exitCode, out: done.stdout.toString(), err: done.stderr.toString() };
}

function git(...args: string[]) {
  const done = sh(
    repo,
    "git",
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@example.com",
    "-c",
    "commit.gpgsign=false",
    ...args,
  );
  if (done.code !== 0) throw new Error(`git ${args.join(" ")}: ${done.err}`);
  return done.out.trim();
}

function run(env: Record<string, string> = {}) {
  const done = Bun.spawnSync(["bash", join(repo, "pm-stack/console/redeploy.sh")], {
    cwd: dir,
    env: {
      ...process.env,
      HOME: home,
      GIT_CONFIG_GLOBAL: "/dev/null",
      PM_CONSOLE_BUN: join(dir, "fake-bun"),
      CHECK: "pass",
      ...env,
    },
  });
  return { code: done.exitCode, out: done.stdout.toString(), err: done.stderr.toString() };
}

const events = () => (existsSync(log) ? readFileSync(log, "utf8").trim().split("\n").filter(Boolean) : []);
const state = (name: string) => {
  const path = join(home, ".local/state/pm-console", name);
  return existsSync(path) ? readFileSync(path, "utf8") : null;
};

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "pm-redeploy-"));
  repo = join(dir, "repo");
  home = join(dir, "home");
  log = join(dir, "events.log");
  mkdirSync(join(repo, "pm-stack/console/src"), { recursive: true });
  mkdirSync(join(repo, "pm-stack/systemd"), { recursive: true });
  mkdirSync(join(home, ".local/state/pm-console"), { recursive: true });
  cpSync(SCRIPT, join(repo, "pm-stack/console/redeploy.sh"));
  writeFileSync(join(repo, "pm-stack/console/install.sh"), `echo "install" >>"${log}"\n`);
  writeFileSync(join(repo, "pm-stack/console/src/server.ts"), "// v1\n");
  for (const unit of UNITS) writeFileSync(join(repo, "pm-stack/systemd", unit), "[Unit]\n");
  // The stand-in for bun: records its arguments, the directory it ran
  // in and one setting from the service's file, then passes or fails.
  writeFileSync(
    join(dir, "fake-bun"),
    `#!/usr/bin/env bash\necho "check $* in=$(basename "$PWD") port=\${PM_CONSOLE_PORT:-} note=\${PM_NOTE:-} plain=\${PM_PLAIN:-}" >>"${log}"\n[ "$CHECK" = pass ] || { echo "boom: cannot start" >&2; exit 1; }\necho "pm-console: check passed"\n`,
  );
  chmodSync(join(dir, "fake-bun"), 0o755);
  // As systemd allows: a comment, a blank line, quoted values, a value with a space.
  writeFileSync(
    join(home, ".local/state/pm-console/.env"),
    '# settings\nPM_CONSOLE_PORT="4872"\n\nPM_NOTE=\'two words\'\nPM_PLAIN=it\'s "kept"\n',
  );
  git("init", "-q", "-b", "main");
  git("add", "-A");
  git("commit", "-q", "-m", "one");
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe("redeploy.sh", () => {
  test("new code is checked with the service's own settings, then put into service, once", () => {
    const first = run();
    expect(first.code).toBe(0);
    expect(first.out).toContain("is in service");
    // The check ran in the console's directory, with the settings file's values, spaces and all.
    expect(events()).toEqual([
      'check run src/server.ts --check in=console port=4872 note=two words plain=it\'s "kept"',
      "install",
    ]);
    expect(state("deployed")).not.toBeNull();
    // Nothing has changed: nothing is checked or installed again.
    expect(run().code).toBe(0);
    expect(events()).toHaveLength(2);
  });

  test("a change to the console or to one of its units is new code; a change elsewhere is not", () => {
    run();
    writeFileSync(join(repo, "README.md"), "elsewhere\n");
    mkdirSync(join(repo, "pm-stack/aggregator"), { recursive: true });
    writeFileSync(join(repo, "pm-stack/aggregator/x.ts"), "x\n");
    writeFileSync(join(repo, "pm-stack/systemd/pm-other.service"), "[Unit]\n");
    git("add", "-A");
    git("commit", "-q", "-m", "elsewhere");
    run();
    expect(events()).toHaveLength(2);
    writeFileSync(join(repo, "pm-stack/systemd/pm-console.service"), "[Unit]\nDescription=changed\n");
    git("commit", "-q", "-am", "unit");
    run();
    expect(events()).toHaveLength(4);
    writeFileSync(join(repo, "pm-stack/console/src/server.ts"), "// v2\n");
    git("commit", "-q", "-am", "code");
    run();
    expect(events()).toHaveLength(6);
  });

  test("code that does not start is never put into service, and is not tried again until the code changes", () => {
    run();
    const inService = state("deployed");
    writeFileSync(join(repo, "pm-stack/console/src/server.ts"), "// broken\n");
    git("commit", "-q", "-am", "broken");
    const bad = run({ CHECK: "fail" });
    expect(bad.code).toBe(1);
    expect(bad.err).toContain("the code in service is unchanged");
    expect(bad.err).toContain("boom: cannot start");
    expect(events().filter((e) => e === "install")).toHaveLength(1);
    expect(state("deployed")).toBe(inService);
    // The next minute: still failed, and the check is not run again.
    const again = run({ CHECK: "fail" });
    expect(again.code).toBe(1);
    expect(events().filter((e) => e.startsWith("check"))).toHaveLength(2);
    // Even if it would pass now: the same code is the same verdict.
    expect(run({ CHECK: "pass" }).code).toBe(1);
    // A fix arrives: checked, installed, and the failure is forgotten.
    writeFileSync(join(repo, "pm-stack/console/src/server.ts"), "// fixed\n");
    git("commit", "-q", "-am", "fixed");
    expect(run().code).toBe(0);
    expect(events().filter((e) => e === "install")).toHaveLength(2);
    expect(state("deploy-failed")).toBeNull();
    expect(state("deployed")).not.toBe(inService);
  });

  test("a checkout part-way through an update is left for the next run", () => {
    run();
    // A file changed on disk that its commit does not have yet.
    writeFileSync(join(repo, "pm-stack/console/src/server.ts"), "// half written\n");
    const mid = run();
    expect(mid.code).toBe(0);
    expect(events()).toHaveLength(2);
  });

  test("an install that fails leaves the code marked as not in service", () => {
    writeFileSync(join(repo, "pm-stack/console/install.sh"), `echo "install" >>"${log}"\nexit 1\n`);
    git("commit", "-q", "-am", "install fails");
    expect(run().code).toBe(1);
    expect(state("deployed")).toBeNull();
    // It is tried again, since the code itself passed its check.
    expect(run().code).toBe(1);
    expect(events().filter((e) => e === "install")).toHaveLength(2);
  });

  test("it takes no arguments", () => {
    const done = Bun.spawnSync(["bash", join(repo, "pm-stack/console/redeploy.sh"), "--force"], {
      env: { ...process.env, HOME: home },
    });
    expect(done.exitCode).toBe(2);
    expect(events()).toEqual([]);
  });
});
