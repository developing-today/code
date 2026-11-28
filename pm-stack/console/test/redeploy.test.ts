// redeploy.sh, run for real against a stand-in for GitHub made here.
//
// The script under test is this repository's own file. GitHub is a bare
// git repository on disk, reached over git's own protocol and allowing
// the partial fetches GitHub allows; "main" there is whatever a working
// copy beside it last pushed. Two things are stand-ins: the program
// that starts the console's self-check (a script that records what it
// was given and passes or fails as told) and install.sh (a script that
// records that it ran, and fails in a release written to). Everything
// else, git included, is the real thing.
//
// The oracle for "what is in service" is the commit pushed to main:
// its files, read back from git, against the files of the release the
// script made current.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  rmSync,
  writeFileSync,
} from "node:fs";
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
let origin: string;
let work: string;
let home: string;
let share: string;
let log: string;

function sh(cwd: string, argv: string[], env: Record<string, string> = {}) {
  const done = Bun.spawnSync(argv, {
    cwd,
    env: { ...process.env, HOME: home, GIT_CONFIG_GLOBAL: "/dev/null", ...env },
  });
  return { code: done.exitCode, out: done.stdout.toString(), err: done.stderr.toString() };
}

function git(cwd: string, ...args: string[]) {
  const done = sh(cwd, [
    "git",
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@example.com",
    "-c",
    "commit.gpgsign=false",
    ...args,
  ]);
  if (done.code !== 0) throw new Error(`git ${args.join(" ")}: ${done.err}`);
  return done.out.trim();
}

/** Run the script, from a copy that sits in no checkout at all unless a test says otherwise. */
function run(env: Record<string, string> = {}, script = join(dir, "redeploy.sh"), cwd = dir) {
  return sh(cwd, ["bash", script], {
    PM_CONSOLE_BUN: join(dir, "fake-bun"),
    PM_CONSOLE_REMOTE: `file://${origin}`,
    CHECK: "pass",
    ...env,
  });
}

/** Change files in the working copy, commit, and push to "GitHub"'s main. Answers the commit. */
function push(message: string, files: Record<string, string>) {
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(join(work, path, ".."), { recursive: true });
    writeFileSync(join(work, path), text);
  }
  git(work, "add", "-A");
  git(work, "commit", "-q", "-m", message);
  git(work, "push", "-q", "origin", "HEAD:main");
  return git(work, "rev-parse", "HEAD");
}

const events = () => (existsSync(log) ? readFileSync(log, "utf8").trim().split("\n").filter(Boolean) : []);
const state = (name: string) => {
  const path = join(home, ".local/state/pm-console", name);
  return existsSync(path) ? readFileSync(path, "utf8") : null;
};
/** The release in service, as the link says. */
const current = () => (existsSync(join(share, "current")) ? readlinkSync(join(share, "current")) : null);
const releases = () => (existsSync(join(share, "releases")) ? readdirSync(join(share, "releases")).sort() : []);
const inService = (path: string) => readFileSync(join(share, "current", path), "utf8");
/** A stand-in for install.sh that records the release it ran from, and fails if told to. */
const installer = (fails = false) =>
  `echo "install $(basename "$(cd "$(dirname "$0")/../.." && pwd -P)")" >>"${log}"\n${fails ? "exit 1\n" : ""}`;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "pm-redeploy-"));
  origin = join(dir, "origin.git");
  work = join(dir, "work");
  home = join(dir, "home");
  share = join(home, ".local/share/pm-console");
  log = join(dir, "events.log");
  mkdirSync(join(home, ".local/state/pm-console"), { recursive: true });
  mkdirSync(work);
  cpSync(SCRIPT, join(dir, "redeploy.sh"));
  // "GitHub": bare, and allowing what GitHub allows a clone that takes only part of the tree.
  git(dir, "init", "-q", "--bare", "-b", "main", origin);
  git(origin, "config", "uploadpack.allowFilter", "true");
  git(origin, "config", "uploadpack.allowAnySHA1InWant", "true");
  git(work, "init", "-q", "-b", "main");
  git(work, "remote", "add", "origin", `file://${origin}`);
  // The stand-in for bun: records its arguments, the directory and the
  // release it ran in, and one setting from the service's file; then
  // passes or fails.
  writeFileSync(
    join(dir, "fake-bun"),
    `#!/usr/bin/env bash\necho "check $* in=$(basename "$PWD") of=$(basename "$(cd ../.. && pwd -P)") port=\${PM_CONSOLE_PORT:-} note=\${PM_NOTE:-} plain=\${PM_PLAIN:-}" >>"${log}"\n[ "$CHECK" = pass ] || { echo "boom: cannot start"; exit 1; }\n`,
  );
  chmodSync(join(dir, "fake-bun"), 0o755);
  // As systemd allows: a comment, a blank line, quoted values, a value with a space.
  writeFileSync(
    join(home, ".local/state/pm-console/.env"),
    '# settings\nPM_CONSOLE_PORT="4872"\n\nPM_NOTE=\'two words\'\nPM_PLAIN=it\'s "kept"\n',
  );
  push("one", {
    "pm-stack/console/src/server.ts": "// v1\n",
    "pm-stack/console/public/app.js": "// app v1\n",
    "pm-stack/console/redeploy.sh": readFileSync(SCRIPT, "utf8"),
    "pm-stack/console/install.sh": installer(),
    ...Object.fromEntries(UNITS.map((unit) => [`pm-stack/systemd/${unit}`, "[Unit]\n"])),
    "pm-stack/systemd/pm-other.service": "[Unit]\n",
    "pm-stack/aggregator/src/pages.ts": "export const PAGES = [];\n",
    "pm-stack/aggregator/src/server.ts": "// the dashboard\n",
    "README.md": "the rest of the repository\n",
    "pkgs/id/src/main.rs": "fn main() {}\n",
  });
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe("redeploy.sh", () => {
  test("the first run makes the console's own copy of main, checks it with the service's own settings, and puts it into service; once", () => {
    const sha = git(work, "rev-parse", "HEAD");
    const first = run();
    expect([first.code, first.err]).toEqual([0, ""]);
    expect(first.out).toContain(`${sha.slice(0, 12)} is in service`);
    // What is in service is that commit's console, laid out as a release of its own.
    expect(current()).toBe(`releases/${sha}`);
    expect(inService("pm-stack/console/src/server.ts")).toBe("// v1\n");
    expect(inService("pm-stack/systemd/pm-console.service")).toBe("[Unit]\n");
    expect(inService("pm-stack/aggregator/src/pages.ts")).toBe("export const PAGES = [];\n");
    // The release says which commit it is, and which tree its console is.
    expect(inService("pm-stack/console/.release")).toBe(
      `commit=${sha}\ntree=${git(work, "rev-parse", "HEAD:pm-stack/console")}\n`,
    );
    // The check ran in that release's console directory, with the settings file's values, spaces and all;
    // then that release's own install ran.
    expect(events()).toEqual([
      `check run src/server.ts --check in=console of=${sha} port=4872 note=two words plain=it's "kept"`,
      `install ${sha}`,
    ]);
    expect(state("deployed")).not.toBeNull();
    // Nothing has changed on main: nothing is checked or installed again.
    const again = run();
    expect([again.code, again.out, again.err]).toEqual([0, "", ""]);
    expect(events()).toHaveLength(2);
  });

  test("only the console's part of main is brought to this machine", () => {
    run();
    const listed = sh(join(share, "current"), ["find", ".", "-type", "f"]).out.trim().split("\n").sort();
    expect(listed).toEqual(
      [
        "./pm-stack/aggregator/src/pages.ts",
        "./pm-stack/console/.release",
        "./pm-stack/console/install.sh",
        "./pm-stack/console/public/app.js",
        "./pm-stack/console/redeploy.sh",
        "./pm-stack/console/src/server.ts",
        ...UNITS.map((unit) => `./pm-stack/systemd/${unit}`),
        "./pm-stack/systemd/pm-other.service",
      ].sort(),
    );
    // And the clone it is made from holds no file of the rest either.
    expect(existsSync(join(share, "repo/README.md"))).toBe(false);
    expect(existsSync(join(share, "repo/pkgs"))).toBe(false);
    expect(existsSync(join(share, "repo/pm-stack/aggregator/src/server.ts"))).toBe(false);
  });

  test("what goes into service is main as GitHub has it, whatever the checkout the script is run from holds", () => {
    const main = git(work, "rev-parse", "HEAD");
    // A checkout someone is working in: on another branch, with other
    // console code committed there and more of it half-written on disk.
    const lanes = join(dir, "lanes");
    git(dir, "clone", "-q", `file://${origin}`, lanes);
    git(lanes, "checkout", "-q", "-b", "someone/elses-work");
    writeFileSync(join(lanes, "pm-stack/console/src/server.ts"), "// another branch's console\n");
    git(lanes, "commit", "-q", "-am", "theirs");
    writeFileSync(join(lanes, "pm-stack/console/public/app.js"), "// half written\n");
    const done = run({}, join(lanes, "pm-stack/console/redeploy.sh"), join(lanes, "pm-stack/console"));
    expect(done.code).toBe(0);
    expect(current()).toBe(`releases/${main}`);
    expect(inService("pm-stack/console/src/server.ts")).toBe("// v1\n");
    expect(inService("pm-stack/console/public/app.js")).toBe("// app v1\n");
    // That checkout is as it was: its branch, its commit, its half-written file.
    expect(git(lanes, "rev-parse", "--abbrev-ref", "HEAD")).toBe("someone/elses-work");
    expect(git(lanes, "status", "--porcelain")).toBe("M pm-stack/console/public/app.js");
    expect(readFileSync(join(lanes, "pm-stack/console/public/app.js"), "utf8")).toBe("// half written\n");
  });

  test("a change on main to the console, one of its units or the dashboard's list of pages is new code; a change elsewhere is not", () => {
    run();
    const first = current();
    push("elsewhere", {
      "README.md": "changed\n",
      "pm-stack/aggregator/src/server.ts": "// changed\n",
      "pm-stack/systemd/pm-other.service": "[Unit]\nDescription=changed\n",
      "pkgs/id/src/main.rs": "fn main() { }\n",
    });
    expect(run().code).toBe(0);
    expect([current(), events().length]).toEqual([first, 2]);
    const unit = push("unit", { "pm-stack/systemd/pm-console.service": "[Unit]\nDescription=changed\n" });
    expect(run().code).toBe(0);
    expect([current(), events().length]).toEqual([`releases/${unit}`, 4]);
    expect(inService("pm-stack/systemd/pm-console.service")).toBe("[Unit]\nDescription=changed\n");
    const code = push("code", { "pm-stack/console/src/server.ts": "// v2\n" });
    expect(run().code).toBe(0);
    expect([current(), events().length]).toEqual([`releases/${code}`, 6]);
    expect(inService("pm-stack/console/src/server.ts")).toBe("// v2\n");
    const pages = push("pages", { "pm-stack/aggregator/src/pages.ts": "export const PAGES = [1];\n" });
    expect(run().code).toBe(0);
    expect([current(), events().length]).toEqual([`releases/${pages}`, 8]);
    // A file taken off main is not left behind in the next release.
    git(work, "rm", "-q", "pm-stack/console/public/app.js");
    git(work, "commit", "-q", "-m", "app removed");
    git(work, "push", "-q", "origin", "HEAD:main");
    expect(run().code).toBe(0);
    expect(existsSync(join(share, "current/pm-stack/console/public/app.js"))).toBe(false);
  });

  test("code that does not start is never put into service, and is not tried again until main's code changes", () => {
    run();
    const good = current();
    const was = state("deployed");
    const broken = push("broken", { "pm-stack/console/src/server.ts": "// broken\n" });
    const bad = run({ CHECK: "fail" });
    expect(bad.code).toBe(1);
    expect(bad.err).toContain("the code in service is unchanged");
    expect(bad.err).toContain("boom: cannot start");
    // The release before it is still the one in service, and the one that failed is not kept.
    expect(current()).toBe(good);
    expect(releases()).not.toContain(broken);
    expect(events().filter((e) => e.startsWith("install"))).toHaveLength(1);
    expect(state("deployed")).toBe(was);
    // The next minute: still failed, and the check is not run again.
    expect(run({ CHECK: "fail" }).code).toBe(1);
    expect(events().filter((e) => e.startsWith("check"))).toHaveLength(2);
    // Even if it would pass now: the same code is the same verdict.
    expect(run({ CHECK: "pass" }).code).toBe(1);
    expect(current()).toBe(good);
    // A fix arrives: checked, installed, and the failure is forgotten.
    const fixed = push("fixed", { "pm-stack/console/src/server.ts": "// fixed\n" });
    expect(run().code).toBe(0);
    expect(current()).toBe(`releases/${fixed}`);
    expect(state("deploy-failed")).toBeNull();
    expect(state("deployed")).not.toBe(was);
  });

  test("a release that does not come up in service is taken out again, and the one before it is put back", () => {
    run();
    const good = current();
    const goodSha = (good as string).replace("releases/", "");
    const was = state("deployed");
    // It passes its self-check, and then its install fails: the service did not come up as it.
    const bad = push("does not come up", { "pm-stack/console/install.sh": installer(true) });
    const done = run();
    expect(done.code).toBe(1);
    expect(done.err).toContain("did not come up in service");
    expect(done.err).toContain("is back in service");
    expect(current()).toBe(good);
    // Its install ran, failed, and the one before it was installed again.
    expect(events().slice(2)).toEqual([
      `check run src/server.ts --check in=console of=${bad} port=4872 note=two words plain=it's "kept"`,
      `install ${bad}`,
      `install ${goodSha}`,
    ]);
    expect(state("deployed")).toBe(was);
    // It is not tried again while main's code is the same ...
    expect(run().code).toBe(1);
    expect(events()).toHaveLength(5);
    // Main goes back to exactly the code in service: there is nothing
    // to put into service, and nothing is failing any more.
    push("reverted", { "pm-stack/console/install.sh": installer() });
    const back = run();
    expect([back.code, back.out, back.err]).toEqual([0, "", ""]);
    expect([current(), events().length]).toEqual([good, 5]);
    // And when main's code is new again, it is tried again.
    const fixed = push("comes up", { "pm-stack/console/src/server.ts": "// v2\n" });
    expect(run().code).toBe(0);
    expect(current()).toBe(`releases/${fixed}`);
    expect(state("deploy-failed")).toBeNull();
  });

  test("the very first release not coming up leaves nothing marked as in service", () => {
    const sha = push("does not come up", { "pm-stack/console/install.sh": installer(true) });
    const done = run();
    expect(done.code).toBe(1);
    expect(done.err).toContain("there is no release before it to put back");
    expect(state("deployed")).toBeNull();
    expect(events()).toEqual([
      `check run src/server.ts --check in=console of=${sha} port=4872 note=two words plain=it's "kept"`,
      `install ${sha}`,
    ]);
  });

  test("with GitHub out of reach nothing changes: quietly at first, and as a failure once it has been half an hour", () => {
    run();
    const good = current();
    const nowhere = { PM_CONSOLE_REMOTE: `file://${join(dir, "no-such.git")}` };
    const quiet = run(nowhere);
    expect([quiet.code, quiet.out]).toEqual([0, ""]);
    expect(current()).toBe(good);
    expect(state("unreached")).not.toBeNull();
    expect(events()).toHaveLength(2);
    // Twenty-nine minutes of it: still quiet.
    const ago = (minutes: number) => `${Math.floor(Date.now() / 1000) - minutes * 60}\n`;
    writeFileSync(join(home, ".local/state/pm-console/unreached"), ago(29));
    expect(run(nowhere).code).toBe(0);
    // Thirty-one: the run fails, which the Services screen and the notifications show.
    writeFileSync(join(home, ".local/state/pm-console/unreached"), ago(31));
    const loud = run(nowhere);
    expect(loud.code).toBe(1);
    expect(loud.err).toContain("GitHub has not answered for 31 minutes");
    expect(loud.err).toContain("the code in service is unchanged");
    expect(current()).toBe(good);
    // Reached again: forgotten, and main is followed as before.
    const next = push("after", { "pm-stack/console/src/server.ts": "// after\n" });
    expect(run().code).toBe(0);
    expect(state("unreached")).toBeNull();
    expect(current()).toBe(`releases/${next}`);
  });

  test("old releases are cleared away; the one in service and the one before it are always kept", () => {
    const made: string[] = [git(work, "rev-parse", "HEAD")];
    run();
    for (let n = 2; n <= 6; n++) {
      made.push(push(`v${n}`, { "pm-stack/console/src/server.ts": `// v${n}\n` }));
      expect(run().code).toBe(0);
    }
    expect(current()).toBe(`releases/${made[5]}`);
    expect(releases()).toEqual([made[3], made[4], made[5]].sort());
    // Something that is not a release is not touched: neither a name
    // that is no commit's, nor one that only looks like the start of one.
    mkdirSync(join(share, "releases/keep-me"));
    mkdirSync(join(share, "releases/deadbeef"));
    made.push(push("v7", { "pm-stack/console/src/server.ts": "// v7\n" }));
    expect(run().code).toBe(0);
    expect(releases()).toEqual([made[4], made[5], made[6], "deadbeef", "keep-me"].sort());
  });

  test("a copy that has been damaged is made again from GitHub", () => {
    run();
    const sha = git(work, "rev-parse", "HEAD");
    // The link gone, the clone's settings changed by hand, a stray file in it.
    rmSync(join(share, "current"));
    git(join(share, "repo"), "config", "remote.origin.url", "file:///somewhere/else.git");
    writeFileSync(join(share, "repo/pm-stack/console/stray.txt"), "left behind\n");
    writeFileSync(join(share, "repo/pm-stack/console/src/server.ts"), "// edited in place\n");
    expect(run().code).toBe(0);
    expect(current()).toBe(`releases/${sha}`);
    expect(inService("pm-stack/console/src/server.ts")).toBe("// v1\n");
    expect(existsSync(join(share, "current/pm-stack/console/stray.txt"))).toBe(false);
  });

  test("it takes no arguments", () => {
    const done = sh(dir, ["bash", join(dir, "redeploy.sh"), "--force"]);
    expect(done.code).toBe(2);
    expect(events()).toEqual([]);
    expect(existsSync(share)).toBe(false);
  });
});
