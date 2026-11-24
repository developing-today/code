// install.sh, run for real in a made-up home directory.
//
// The script under test is this repository's own file, placed in a
// release laid out the way redeploy.sh lays one out. Two programs are
// stand-ins, found first on its PATH: `systemctl` (records what it was
// asked) and `curl` (answers the console's health check as told). The
// oracle is what a machine must have afterwards: the release's own unit
// files installed, the service restarted, and success only when the
// console answers as that release's commit.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SCRIPT = join(import.meta.dir, "..", "install.sh");
const SYSTEMD = join(import.meta.dir, "..", "..", "systemd");
const UNITS = [
  "pm-console.service",
  "pm-console-expose.service",
  "pm-console-expose.timer",
  "pm-console-redeploy.service",
  "pm-console-redeploy.timer",
  "pm-console-android-release.service",
  "pm-console-android-release.timer",
];
const COMMIT = "0123456789abcdef0123456789abcdef01234567";

let dir: string;
let home: string;
let share: string;
let log: string;

/** A release as redeploy.sh lays one out, holding this repository's install.sh and units. */
function release(commit: string, says = true) {
  const root = join(share, "releases", commit);
  mkdirSync(join(root, "pm-stack/console"), { recursive: true });
  mkdirSync(join(root, "pm-stack/systemd"), { recursive: true });
  cpSync(SCRIPT, join(root, "pm-stack/console/install.sh"));
  for (const unit of UNITS) cpSync(join(SYSTEMD, unit), join(root, "pm-stack/systemd", unit));
  if (says) writeFileSync(join(root, "pm-stack/console/.release"), `commit=${commit}\ntree=${"a".repeat(40)}\n`);
  return root;
}

function serve(commit: string) {
  rmSync(join(share, "current"), { force: true });
  symlinkSync(`releases/${commit}`, join(share, "current"));
}

function run(script: string, env: Record<string, string> = {}) {
  const done = Bun.spawnSync(["bash", script], {
    cwd: dir,
    env: {
      ...process.env,
      HOME: home,
      PATH: `${join(dir, "bin")}:${process.env.PATH}`,
      PM_CONSOLE_WAIT_SECONDS: "2",
      HEALTH: `{"ok":true,"version":"${COMMIT.slice(0, 12)}"}`,
      ...env,
    },
  });
  return { code: done.exitCode, out: done.stdout.toString(), err: done.stderr.toString() };
}

const events = () => (existsSync(log) ? readFileSync(log, "utf8").trim().split("\n").filter(Boolean) : []);
const installed = (unit: string) => join(home, ".config/systemd/user", unit);

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "pm-install-"));
  home = join(dir, "home");
  share = join(home, ".local/share/pm-console");
  log = join(dir, "events.log");
  mkdirSync(join(dir, "bin"), { recursive: true });
  mkdirSync(share, { recursive: true });
  writeFileSync(join(dir, "bin/systemctl"), `#!/usr/bin/env bash\necho "systemctl $*" >>"${log}"\n`);
  // The console's health check: answers what the test says, or not at all.
  writeFileSync(
    join(dir, "bin/curl"),
    `#!/usr/bin/env bash\necho "curl \${*: -1}" >>"${log}"\n[ "$HEALTH" = none ] && exit 7\nprintf '%s' "$HEALTH"\n`,
  );
  // No waiting between tries.
  writeFileSync(join(dir, "bin/sleep"), "#!/usr/bin/env bash\nexit 0\n");
  for (const name of ["systemctl", "curl", "sleep"]) chmodSync(join(dir, "bin", name), 0o755);
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe("install.sh", () => {
  test("from the release in service: its own units are installed, the console restarted, and it has come up when it answers as that release", () => {
    const root = release(COMMIT);
    serve(COMMIT);
    const done = run(join(root, "pm-stack/console/install.sh"));
    expect([done.code, done.err]).toEqual([0, ""]);
    expect(done.out).toContain(`${COMMIT.slice(0, 12)} is running on http://127.0.0.1:4872/console/`);
    // Every unit of the console, as this release has it.
    for (const unit of UNITS) {
      expect(readFileSync(installed(unit), "utf8")).toBe(readFileSync(join(SYSTEMD, unit), "utf8"));
    }
    // A settings file is made where there was none, for the owner alone to read.
    expect(readFileSync(join(home, ".local/state/pm-console/.env"), "utf8")).toBe(
      "PM_CONSOLE_PORT=4872\nPM_CONSOLE_OWNER_EMAILS=\n",
    );
    expect(events()).toEqual([
      "systemctl --user daemon-reload",
      "systemctl --user enable pm-console.service",
      "systemctl --user restart pm-console.service",
      "systemctl --user enable --now pm-console-expose.timer pm-console-redeploy.timer pm-console-android-release.timer",
      "curl http://127.0.0.1:4872/console/health",
    ]);
  });

  test("run through the link that says which release is in service, it is the same release", () => {
    release(COMMIT);
    serve(COMMIT);
    expect(run(join(share, "current/pm-stack/console/install.sh")).code).toBe(0);
  });

  test("the units it installs run the console from the release in service, and nothing from a checkout someone works in", () => {
    for (const unit of UNITS.filter((name) => name.endsWith(".service"))) {
      const text = readFileSync(join(SYSTEMD, unit), "utf8");
      expect([unit, /git\/code/.test(text)]).toEqual([unit, false]);
      expect([unit, text.includes("%h/.local/share/pm-console/current/pm-stack/console")]).toEqual([unit, true]);
    }
  });

  test("an existing settings file is never overwritten, and its port is the one asked", () => {
    const root = release(COMMIT);
    serve(COMMIT);
    mkdirSync(join(home, ".local/state/pm-console"), { recursive: true });
    writeFileSync(
      join(home, ".local/state/pm-console/.env"),
      'PM_CONSOLE_PORT="4999"\nPM_CONSOLE_OWNER_EMAILS=o@example.com\n',
    );
    expect(run(join(root, "pm-stack/console/install.sh")).code).toBe(0);
    expect(readFileSync(join(home, ".local/state/pm-console/.env"), "utf8")).toBe(
      'PM_CONSOLE_PORT="4999"\nPM_CONSOLE_OWNER_EMAILS=o@example.com\n',
    );
    expect(events().at(-1)).toBe("curl http://127.0.0.1:4999/console/health");
  });

  test("an answer from other code than this release's is not having come up: it fails, saying which answered", () => {
    const root = release(COMMIT);
    serve(COMMIT);
    const other = run(join(root, "pm-stack/console/install.sh"), { HEALTH: '{"ok":true,"version":"ffffffffffff"}' });
    expect(other.code).toBe(1);
    expect(other.err).toContain(`it answers its health check as ffffffffffff, not as ${COMMIT.slice(0, 12)}`);
    // Asked for as long as it was given, not once.
    expect(events().filter((e) => e.startsWith("curl"))).toHaveLength(2);
    // Code from before the console said which commit it is: also not this release.
    const old = run(join(root, "pm-stack/console/install.sh"), { HEALTH: '{"ok":true}' });
    expect(old.code).toBe(1);
    expect(old.err).toContain("a build that does not say which it is");
  });

  test("no answer at all fails, and says where to look", () => {
    const root = release(COMMIT);
    serve(COMMIT);
    const done = run(join(root, "pm-stack/console/install.sh"), { HEALTH: "none" });
    expect(done.code).toBe(1);
    expect(done.err).toContain("did not answer its health check");
    expect(done.err).toContain("journalctl --user -u pm-console");
  });

  test("a release that does not say which commit it is has come up when the console answers at all", () => {
    const root = release(COMMIT, false);
    serve(COMMIT);
    expect(run(join(root, "pm-stack/console/install.sh"), { HEALTH: '{"ok":true}' }).code).toBe(0);
  });

  test("from anywhere but the release in service it does nothing, and says how it is meant to be run", () => {
    const root = release(COMMIT);
    const other = release("f".repeat(40));
    serve("f".repeat(40));
    // Another release than the one in service, and a copy in no release at all.
    const stray = join(dir, "checkout/pm-stack/console");
    mkdirSync(stray, { recursive: true });
    cpSync(SCRIPT, join(stray, "install.sh"));
    for (const script of [join(root, "pm-stack/console/install.sh"), join(stray, "install.sh")]) {
      const done = run(script);
      expect(done.code).toBe(1);
      expect(done.err).toContain("this is not the release in service");
      expect(done.err).toContain("Run redeploy.sh");
    }
    expect(events()).toEqual([]);
    expect(existsSync(installed("pm-console.service"))).toBe(false);
    expect(existsSync(other)).toBe(true);
    // With nothing in service at all, the same.
    rmSync(join(share, "current"));
    expect(run(join(root, "pm-stack/console/install.sh")).code).toBe(1);
    expect(events()).toEqual([]);
  });

  test("it takes no arguments", () => {
    const root = release(COMMIT);
    serve(COMMIT);
    const done = Bun.spawnSync(["bash", join(root, "pm-stack/console/install.sh"), "--now"], {
      env: { ...process.env, HOME: home, PATH: `${join(dir, "bin")}:${process.env.PATH}` },
    });
    expect(done.exitCode).toBe(2);
    expect(events()).toEqual([]);
  });
});
