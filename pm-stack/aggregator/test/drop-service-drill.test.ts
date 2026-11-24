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

// The drop-service drill follows the #2865 contract proven by the
// kill-collector drill: every refusal path touches nothing (in
// particular it never runs `systemctl start`), restoration is armed
// BEFORE the destructive stop so an interrupting TERM still
// restores, and a real drop still restores and reports "passed".
// These tests run the real script against stubbed systemctl/sudo/
// sleep and a stubbed health channel (~/tools/laptop-ssh.sh under a
// fake HOME), recording every systemctl invocation.

const DRILL = join(import.meta.dir, "..", "scripts", "drop-service-drill.sh");
const UNIT = "pm-projector.service";

interface Scenario {
  host?: string;
  interruptStop?: boolean; // stub systemctl stop sends TERM to the drill, then fails
  unitActive: boolean;
  healthPlan: string[]; // per health-channel call: up | down | degraded-down | fail
  incidentFlag?: boolean;
  holdLock?: boolean; // hold the shared pm-drill lock
  holdLegacyLock?: boolean; // hold the legacy kill-collector lock
}

function runDrill(scn: Scenario) {
  const base = mkdtempSync(join(tmpdir(), "drop-drill-test-"));
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

  writeExe(join(bin, "hostname"), `#!/bin/sh\nprintf '%s\\n' "${scn.host ?? "amd"}"\n`);

  // The drill drives the laptop's systemd USER manager.
  writeExe(
    join(bin, "systemctl"),
    `#!/bin/sh
echo "systemctl $*" >> "${stubLog}"
[ "$1" = "--user" ] && shift
if [ "$1" = "is-active" ]; then
  [ "\${STUB_UNIT_ACTIVE:-1}" = "1" ] && exit 0 || exit 1
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
    `#!/bin/sh\n[ "$1" = "-n" ] && shift\nexec "$@"\n`,
  );
  // The drill waits minutes in real runs; the stub makes waits instant.
  writeExe(join(bin, "sleep"), `#!/bin/sh\nexit 0\n`);

  // Health channel: the drill reads it via ~/tools/laptop-ssh.sh.
  // Each invocation consumes the next plan entry (the last repeats).
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
  down) printf '%s' '{"services":[{"def":{"id":"projector"},"ok":false,"state":"down"}],"degraded":false}' ;;
  degraded-down) printf '%s' '{"services":[{"def":{"id":"projector"},"ok":false,"state":"down"}],"degraded":true}' ;;
  *) printf '%s' '{"services":[{"def":{"id":"projector"},"ok":true,"state":"up"}],"degraded":false}' ;;
esac
`,
  );

  if (scn.incidentFlag) writeFileSync(join(state, "incident.flag"), "");

  let lockHolder: ReturnType<typeof Bun.spawn> | null = null;
  if (scn.holdLock || scn.holdLegacyLock) {
    const lockName = scn.holdLegacyLock ? "kill-collector.lock" : "pm-drill.lock";
    lockHolder = Bun.spawn(["flock", join(state, lockName), "sleep", "15"], {
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
        STUB_UNIT_ACTIVE: scn.unitActive ? "1" : "0",
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
      resultFile: join(state, "drop-service-result.json"),
    };
  } finally {
    lockHolder?.kill();
  }
}

function expectNoStart(log: string) {
  const starts = log.split("\n").filter((l) => l.includes(" start "));
  expect(starts).toEqual([]);
}

describe("drop-service drill: refusals touch nothing (#2865 contract)", () => {
  test("non-laptop host -> refuse before any systemctl call", () => {
    const r = runDrill({ host: "sandbox", unitActive: true, healthPlan: ["up"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("must run on laptop host 'amd'");
    expect(r.systemctlLog.trim()).toBe("");
  });

  test("target unit inactive -> refuse, no systemctl start", () => {
    const r = runDrill({ unitActive: false, healthPlan: ["up"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("is not active");
    expectNoStart(r.systemctlLog);
  });

  test("incident flag present -> refuse before any systemctl call", () => {
    const r = runDrill({ unitActive: true, healthPlan: ["up"], incidentFlag: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("incident flag present");
    expect(r.systemctlLog.trim()).toBe("");
  });

  test("shared drill lock held -> refuse, no systemctl start", () => {
    const r = runDrill({ unitActive: true, healthPlan: ["up"], holdLock: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("shared drill lock");
    expectNoStart(r.systemctlLog);
  });

  test("legacy kill-collector lock held -> refuse, no systemctl start", () => {
    const r = runDrill({ unitActive: true, healthPlan: ["up"], holdLegacyLock: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("kill-collector drill holds its lock");
    expectNoStart(r.systemctlLog);
  });

  test("aggregator health unreadable -> refuse, no systemctl start", () => {
    const r = runDrill({ unitActive: true, healthPlan: ["fail"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("refusing to drill blind");
    expectNoStart(r.systemctlLog);
  });

  test("target already down -> refuse (real incident), no systemctl start", () => {
    const r = runDrill({ unitActive: true, healthPlan: ["down"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("already down");
    expectNoStart(r.systemctlLog);
  });
});

describe("drop-service drill: an interrupted stop still restores", () => {
  test("TERM lands while systemctl stop is in flight -> restore starts the unit", () => {
    const r = runDrill({ unitActive: true, healthPlan: ["up"], interruptStop: true });
    expect(r.status).not.toBe(0);
    const starts = r.systemctlLog.split("\n").filter((l) => l.includes(" start "));
    expect(starts.filter((l) => l.includes(`start ${UNIT}`))).toHaveLength(1);
  });
});

describe("drop-service drill: a real drop still restores", () => {
  test("drop -> signals fire -> restore starts unit, outcome passed", () => {
    // Health reads in order: baseline (up), wait-poll (degraded-down
    // -> both signals), restore confirmation (up).
    const r = runDrill({
      unitActive: true,
      healthPlan: ["up", "degraded-down", "up"],
    });
    expect(r.status).toBe(0);
    const calls = r.systemctlLog.split("\n").filter(Boolean);
    const stop = calls.findIndex((l) => l.includes(`stop ${UNIT}`));
    const start = calls.findIndex((l) => l.includes(`start ${UNIT}`));
    expect(stop).toBeGreaterThanOrEqual(0);
    expect(start).toBeGreaterThan(stop);
    expect(existsSync(r.resultFile)).toBe(true);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("passed");
    expect(result.signal_service_down).toBe(true);
    expect(result.signal_degraded).toBe(true);
    expect(result.restoration_confirmed).toBe(true);
  });

  test("down without degraded -> outcome failed, but the unit is still restored", () => {
    const r = runDrill({
      unitActive: true,
      healthPlan: ["up", "down", "up"],
    });
    expect(r.status).toBe(1);
    const starts = r.systemctlLog.split("\n").filter((l) => l.includes(`start ${UNIT}`));
    expect(starts.length).toBeGreaterThanOrEqual(1);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("failed");
    expect(result.signal_service_down).toBe(true);
    expect(result.signal_degraded).toBe(false);
    expect(result.restoration_confirmed).toBe(true);
  });
});
