import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #2865: the kill-collector drill installed its restore trap BEFORE its
// precondition checks, so every refusal (timer inactive, health
// unreadable, feed already stale) still ran restore() — starting the
// collector timer and an immediate collection run, contradicting the
// drill's own "refuse to start, nothing is touched" interlock and
// potentially masking a real incident. These tests run the real drill
// script against stubbed systemctl/sudo/sleep and a stubbed health
// channel (~/tools/laptop-ssh.sh under a fake HOME), record every
// systemctl invocation, and pin:
//   - every refusal path exits non-zero with NO `systemctl start`;
//   - a drill that actually kills the collector still restores exactly
//     (stop timer, stop service, start timer, start service) and
//     reports outcome "passed".

const DRILL = join(import.meta.dir, "..", "scripts", "kill-collector-drill.sh");

interface Scenario {
  host?: string;
  interruptPause?: boolean; // stub touch sends TERM to the drill while creating the pause file
  pauseOverride?: string; // drill-side PM_CORE_FEED_PAUSE_FILE (defaults to the test pause file)
  aggEnvPauseFile?: string; // PM_CORE_FEED_PAUSE_FILE written into the aggregator .env
  noAggEnv?: boolean; // omit the aggregator .env entirely
  timerActive: boolean;
  healthPlan: string[]; // per health-channel call: fresh | stale | fail
  incidentFlag?: boolean;
  holdLock?: boolean;
}

function runDrill(scn: Scenario) {
  const base = mkdtempSync(join(tmpdir(), "drill-test-"));
  const home = join(base, "home");
  const bin = join(base, "bin");
  const state = join(base, "state");
  const pauseFile = join(state, "fleet-feed.pause");
  mkdirSync(join(home, "tools"), { recursive: true });
  mkdirSync(bin, { recursive: true });
  mkdirSync(state, { recursive: true });

  const stubLog = join(base, "systemctl.log");
  writeFileSync(stubLog, "");
  const realTouch = Bun.which("touch") ?? "/usr/bin/touch";
  const writeExe = (path: string, body: string) => {
    writeFileSync(path, body);
    chmodSync(path, 0o755);
  };

  writeExe(join(bin, "hostname"), `#!/bin/sh\nprintf '%s\\n' "${scn.host ?? "amd"}"\n`);

  writeExe(
    join(bin, "systemctl"),
    `#!/bin/sh
echo "systemctl $*" >> "${stubLog}"
if [ "$1" = "is-active" ]; then
  [ "\${STUB_TIMER_ACTIVE:-1}" = "1" ] && exit 0 || exit 1
fi
exit 0
`,
  );
  writeExe(
    join(bin, "sudo"),
    `#!/bin/sh
[ "$1" = "-n" ] && shift
exec "$@"
`,
  );
  // The drill waits minutes in real runs; the stub makes waits instant.
  writeExe(join(bin, "sleep"), `#!/bin/sh\nexit 0\n`);
  writeExe(
    join(bin, "touch"),
    `#!/bin/sh
"${realTouch}" "$@"
if [ "\${STUB_INTERRUPT_PAUSE:-0}" = "1" ]; then
  kill -TERM "$PPID"
  exit 1
fi
`,
  );

  // Health channel: the drill reads it via ~/tools/laptop-ssh.sh. Each
  // invocation consumes the next plan entry (the last one repeats).
  const planFile = join(base, "health-plan");
  const counterFile = join(base, "health-count");
  writeFileSync(planFile, scn.healthPlan.join("\n") + "\n");
  writeFileSync(counterFile, "0");
  writeExe(
    join(home, "tools", "laptop-ssh.sh"),
    `#!/bin/sh
n=$(cat "${counterFile}")
n=$((n + 1))
echo "$n" > "${counterFile}"
mode=$(sed -n "\${n}p" "${planFile}")
[ -z "$mode" ] && mode=$(tail -n 1 "${planFile}")
case "$mode" in
  fail) exit 1 ;;
  stale)
    python3 -c 'import datetime; print((datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=20)).strftime("%Y-%m-%dT%H:%M:%SZ"))' > "${base}/gen-at"
    ;;
  *)
    python3 -c 'import datetime; print(datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))' > "${base}/gen-at"
    ;;
esac
printf '{"feed":{"generatedAt":"%s"},"degraded":false}' "$(cat "${base}/gen-at")"
`,
  );

  if (scn.incidentFlag) writeFileSync(join(state, "incident.flag"), "");

  const aggEnvDir = join(home, ".local", "state", "pm-aggregator");
  if (!scn.noAggEnv) {
    mkdirSync(aggEnvDir, { recursive: true });
    writeFileSync(
      join(aggEnvDir, ".env"),
      `PM_AGG_FEED_FILE=${join(state, "fleet-feed.json")}\n${scn.aggEnvPauseFile ? `PM_CORE_FEED_PAUSE_FILE=${scn.aggEnvPauseFile}\n` : ""}`,
    );
  }

  let lockHolder: ReturnType<typeof Bun.spawn> | null = null;
  if (scn.holdLock) {
    lockHolder = Bun.spawn(["flock", join(state, "pm-drills.lock"), "sleep", "15"], {
      stdout: "ignore",
      stderr: "ignore",
    });
    Bun.sleepSync(300);
  }

  try {
    const r = spawnSync("bash", [DRILL], {
      env: {
        ...process.env,
        HOME: home,
        PATH: `${bin}:${process.env.PATH}`,
        PM_DRILL_STATE_DIR: state,
        PM_DRILL_NO_POST: "1",
        PM_CORE_FEED_PAUSE_FILE: scn.pauseOverride ?? pauseFile,
        STUB_TIMER_ACTIVE: scn.timerActive ? "1" : "0",
        STUB_INTERRUPT_PAUSE: scn.interruptPause ? "1" : "0",
      },
      encoding: "utf8",
      timeout: 30000,
    });
    return {
      status: r.status,
      stdout: r.stdout ?? "",
      stderr: r.stderr ?? "",
      systemctlLog: readFileSync(stubLog, "utf8"),
      resultFile: join(state, "kill-collector-result.json"),
      pauseFile,
    };
  } finally {
    lockHolder?.kill();
  }
}

function expectNoStart(log: string) {
  const starts = log.split("\n").filter((l) => l.includes(" start "));
  expect(starts).toEqual([]);
}

describe("kill-collector drill: pause path agrees with the aggregator (#8570)", () => {
  test("drill override that disagrees with the aggregator's pause file -> refuse, touch nothing", () => {
    const elsewhere = join(tmpdir(), `elsewhere-${Date.now()}.pause`);
    const r = runDrill({ timerActive: false, healthPlan: ["fresh"], pauseOverride: elsewhere });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("disagrees with the aggregator's pause file");
    expect(r.systemctlLog.trim()).toBe("");
    expect(existsSync(elsewhere)).toBe(false);
    expect(existsSync(r.pauseFile)).toBe(false);
  });

  test("aggregator .env pins a pause file the drill does not match -> refuse", () => {
    const pinned = join(tmpdir(), `pinned-${Date.now()}.pause`);
    const r = runDrill({ timerActive: false, healthPlan: ["fresh"], aggEnvPauseFile: pinned, pauseOverride: undefined });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("disagrees with the aggregator's pause file");
    expect(existsSync(r.pauseFile)).toBe(false);
  });

  test("unreadable aggregator .env -> refuse before creating any pause file", () => {
    const r = runDrill({ timerActive: false, healthPlan: ["fresh"], noAggEnv: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("cannot read aggregator config");
    expect(r.systemctlLog.trim()).toBe("");
    expect(existsSync(r.pauseFile)).toBe(false);
  });
});

describe("kill-collector drill: refusals touch nothing (#2865)", () => {
  test("non-laptop host -> refuse before any systemctl call", () => {
    const r = runDrill({ host: "sandbox", timerActive: true, healthPlan: ["fresh"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("must run on laptop host 'amd'");
    expect(r.systemctlLog.trim()).toBe("");
  });

  test("legacy collector timer still active -> refuse without creating pause file", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("legacy timer to be retired");
    expectNoStart(r.systemctlLog);
    expect(existsSync(r.pauseFile)).toBe(false);
  });

  test("incident flag present -> refuse before any systemctl call", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh"], incidentFlag: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("incident flag present");
    expect(r.systemctlLog.trim()).toBe("");
  });

  test("drill lock held -> refuse, no systemctl start", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh"], holdLock: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("holds the lock");
    expectNoStart(r.systemctlLog);
  });

  test("aggregator health unreadable -> refuse, no systemctl start", () => {
    const r = runDrill({ timerActive: false, healthPlan: ["fail"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("refusing to drill blind");
    expectNoStart(r.systemctlLog);
  });

  test("feed already stale -> refuse (real incident), no systemctl start", () => {
    const r = runDrill({ timerActive: false, healthPlan: ["stale"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("already stale");
    expectNoStart(r.systemctlLog);
  });
});

describe("kill-collector drill: interrupted pause creation restores the collector", () => {
  test("TERM lands while pause file is being created -> restore removes it", () => {
    const r = runDrill({ timerActive: false, healthPlan: ["fresh"], interruptPause: true });
    expect(r.status).not.toBe(0);
    expect(r.systemctlLog).not.toContain("stop fleet-feed-collector");
    expect(r.systemctlLog).not.toContain("start fleet-feed-collector");
    expect(existsSync(r.pauseFile)).toBe(false);
  });
});

describe("kill-collector drill: a real kill still restores (#2865)", () => {
  test("pause -> signal fires -> resume in-process collector, outcome passed", () => {
    // Health reads in order: baseline (fresh), wait-poll (stale ->
    // signal), evidence capture (stale), restore confirmation (fresh).
    const r = runDrill({
      timerActive: false,
      healthPlan: ["fresh", "stale", "stale", "fresh"],
    });
    expect(r.status).toBe(0);
    const calls = r.systemctlLog.split("\n").filter(Boolean);
    expect(calls).toEqual(["systemctl is-active --quiet fleet-feed-collector.timer"]);
    expect(existsSync(r.pauseFile)).toBe(false);
    expect(existsSync(r.resultFile)).toBe(true);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("passed");
    expect(result.signal_fired).toBe(true);
    expect(result.restoration_confirmed).toBe(true);
  });
});
