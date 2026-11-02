import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
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
  interruptStop?: boolean; // stub systemctl stop sends TERM to the drill, then fails
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
  mkdirSync(join(home, "tools"), { recursive: true });
  mkdirSync(bin, { recursive: true });
  mkdirSync(state, { recursive: true });

  const stubLog = join(base, "systemctl.log");
  writeFileSync(stubLog, "");
  const writeExe = (path: string, body: string) => {
    writeFileSync(path, body);
    chmodSync(path, 0o755);
  };

  writeExe(
    join(bin, "systemctl"),
    `#!/bin/sh
echo "systemctl $*" >> "${stubLog}"
if [ "$1" = "is-active" ]; then
  [ "\${STUB_TIMER_ACTIVE:-1}" = "1" ] && exit 0 || exit 1
fi
if [ "$1" = "stop" ] && [ "\${STUB_INTERRUPT_STOP:-0}" = "1" ]; then
  kill -TERM "$PPID"
  exit 1
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

  let lockHolder: ReturnType<typeof Bun.spawn> | null = null;
  if (scn.holdLock) {
    lockHolder = Bun.spawn(["flock", join(state, "kill-collector.lock"), "sleep", "15"], {
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
        STUB_TIMER_ACTIVE: scn.timerActive ? "1" : "0",
        STUB_INTERRUPT_STOP: scn.interruptStop ? "1" : "0",
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
    };
  } finally {
    lockHolder?.kill();
  }
}

function expectNoStart(log: string) {
  const starts = log.split("\n").filter((l) => l.includes(" start "));
  expect(starts).toEqual([]);
}

describe("kill-collector drill: refusals touch nothing (#2865)", () => {
  test("collector timer inactive -> refuse, no systemctl start", () => {
    const r = runDrill({ timerActive: false, healthPlan: ["fresh"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("is not active");
    expectNoStart(r.systemctlLog);
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
    const r = runDrill({ timerActive: true, healthPlan: ["fail"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("refusing to drill blind");
    expectNoStart(r.systemctlLog);
  });

  test("feed already stale -> refuse (real incident), no systemctl start", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["stale"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("already stale");
    expectNoStart(r.systemctlLog);
  });
});

describe("kill-collector drill: an interrupted stop still restores (#2865, Codex P2 on #3027)", () => {
  test("TERM lands while systemctl stop is in flight -> restore starts timer + service", () => {
    // The stub stop signals the drill (TERM, as systemd cancellation
    // does) after the stop is already in flight, then fails. Restore
    // must be armed BEFORE the stop, or both traps skip it and the
    // collector timer stays stopped indefinitely.
    const r = runDrill({ timerActive: true, healthPlan: ["fresh"], interruptStop: true });
    expect(r.status).not.toBe(0);
    const calls = r.systemctlLog.split("\n").filter(Boolean);
    const starts = calls.filter((l) => l.includes(" start "));
    expect(starts.filter((l) => l.includes("start fleet-feed-collector.timer"))).toHaveLength(1);
    expect(starts.filter((l) => l.includes("start fleet-feed-collector.service"))).toHaveLength(1);
  });
});

describe("kill-collector drill: a real kill still restores (#2865)", () => {
  test("kill -> signal fires -> restore starts timer + service, outcome passed", () => {
    // Health reads in order: baseline (fresh), wait-poll (stale ->
    // signal), evidence capture (stale), restore confirmation (fresh).
    const r = runDrill({
      timerActive: true,
      healthPlan: ["fresh", "stale", "stale", "fresh"],
    });
    expect(r.status).toBe(0);
    const calls = r.systemctlLog.split("\n").filter(Boolean);
    const stopTimer = calls.findIndex((l) => l.includes("stop fleet-feed-collector.timer"));
    const stopSvc = calls.findIndex((l) => l.includes("stop fleet-feed-collector.service"));
    const startTimer = calls.findIndex((l) => l.includes("start fleet-feed-collector.timer"));
    const startSvc = calls.findIndex((l) => l.includes("start fleet-feed-collector.service"));
    expect(stopTimer).toBeGreaterThanOrEqual(0);
    expect(stopSvc).toBeGreaterThan(stopTimer);
    expect(startTimer).toBeGreaterThan(stopSvc);
    expect(startSvc).toBeGreaterThan(startTimer);
    expect(existsSync(r.resultFile)).toBe(true);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("passed");
    expect(result.signal_fired).toBe(true);
    expect(result.restoration_confirmed).toBe(true);
  });
});
