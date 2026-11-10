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

// Second slice of #1283 (first was the kill-collector drill, #1823).
// The age-feed drill must verify the stale-but-alive feed signal — the
// failure mode a dead collector cannot produce — without ever leaving
// the feed aged. These tests run the real script against stubbed
// systemctl/sleep, a stubbed health channel (PM_DRILL_HEALTH_CMD) and a
// real feed file in a temp dir, and pin:
//   - every refusal path exits non-zero and leaves the feed bytes
//     byte-identical (the #2865 lesson from the kill-collector drill);
//   - a drill that ages the feed restores the original bytes, and a
//     collector rewrite mid-drill is re-aged rather than mistaken for
//     freshness;
//   - a fired signal + confirmed restore reports outcome "passed".

const DRILL = join(import.meta.dir, "..", "scripts", "age-feed-drill.sh");

interface Scenario {
  timerActive: boolean;
  healthPlan: string[]; // per health call: fresh | stale | fail
  incidentFlag?: boolean;
  holdLock?: boolean;
  /** Rendered overview page for PM_DRILL_PAGE_CMD (#1116 item 5):
   *  warn = carries the stale-feed banner, silent = readable page
   *  without it, fail = the page cannot be fetched. Unset means the
   *  drill default runs (local curl, unreadable in this sandbox). */
  page?: "warn" | "silent" | "fail";
  /** Every drill sleep rewrites the feed with a fresh stamp — the
   *  collector's own 5-minute run landing mid-drill. */
  rewriteOnSleep?: boolean;
}

function iso(offsetMinutes: number) {
  return new Date(Date.now() + offsetMinutes * 60_000).toISOString();
}

function runDrill(scn: Scenario) {
  const base = mkdtempSync(join(tmpdir(), "age-feed-test-"));
  const bin = join(base, "bin");
  const state = join(base, "state");
  const feedDir = join(base, "feed");
  mkdirSync(bin, { recursive: true });
  mkdirSync(state, { recursive: true });
  mkdirSync(feedDir, { recursive: true });

  const feedFile = join(feedDir, "fleet-feed.json");
  const originalFeed =
    JSON.stringify({ version: 1, generatedAt: iso(-1), attention: [], sections: { keep: true } }, null, 2) + "\n";
  writeFileSync(feedFile, originalFeed);

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
if [ "$1" = "--user" ] && [ "$2" = "is-active" ]; then
  [ "\${STUB_TIMER_ACTIVE:-1}" = "1" ] && exit 0 || exit 3
fi
exit 0
`,
  );
  // The drill waits minutes in real runs; the stub makes waits instant
  // (and, when asked, simulates the collector rewriting the feed).
  writeExe(
    join(bin, "sleep"),
    `#!/bin/sh
if [ "\${STUB_REWRITE_ON_SLEEP:-0}" = "1" ]; then
  printf '{"version":1,"generatedAt":"%s","attention":[],"sections":{"keep":true}}' "$(python3 -c 'import datetime; print(datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00","Z"))')" > "${feedFile}"
fi
exit 0
`,
  );

  const planFile = join(base, "health-plan");
  const counterFile = join(base, "health-count");
  writeFileSync(planFile, scn.healthPlan.join("\n") + "\n");
  writeFileSync(counterFile, "0");
  writeExe(
    join(base, "health"),
    `#!/bin/sh
n=$(cat "${counterFile}")
n=$((n + 1))
echo "$n" > "${counterFile}"
mode=$(sed -n "\${n}p" "${planFile}")
[ -z "$mode" ] && mode=$(tail -n 1 "${planFile}")
case "$mode" in
  fail) exit 1 ;;
  stale) minutes=-20 ;;
  *) minutes=-1 ;;
esac
gen=$(python3 -c "import datetime,sys; print((datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=int(sys.argv[1]))).strftime('%Y-%m-%dT%H:%M:%SZ'))" "$minutes")
printf '{"degraded":%s,"feed":{"generatedAt":"%s","ageMs":0}}' "$([ "$mode" = stale ] && echo true || echo false)" "$gen"
`,
  );

  if (scn.incidentFlag) writeFileSync(join(state, "incident.flag"), "");

  let lockHolder: ReturnType<typeof Bun.spawn> | null = null;
  if (scn.holdLock) {
    lockHolder = Bun.spawn(["flock", join(state, "age-feed.lock"), "sleep", "15"], {
      stdout: "ignore",
      stderr: "ignore",
    });
    Bun.sleepSync(300);
  }

  try {
    const env: Record<string, string | undefined> = {
      ...process.env,
      HOME: base,
      PATH: `${bin}:${process.env.PATH}`,
      PM_DRILL_STATE_DIR: state,
      PM_DRILL_FEED_FILE: feedFile,
      PM_DRILL_HEALTH_CMD: join(base, "health"),
      PM_DRILL_ATTENTION_CMD: `printf '%s' '{"attention":[{"id":"watchdog:feed"}]}'`,
      PM_DRILL_NO_POST: "1",
      PM_DRILL_RESTORE_WAIT_MIN: "1",
      STUB_TIMER_ACTIVE: scn.timerActive ? "1" : "0",
      STUB_REWRITE_ON_SLEEP: scn.rewriteOnSleep ? "1" : "0",
    };
    // #1116 item (5): stub the rendered overview page the drill greps for
    // the stale-feed banner. Unset when the scenario has no page plan, so
    // the drill default (local curl) runs and records unknown.
    if (scn.page === "warn") {
      writeExe(
        join(base, "page"),
        `#!/bin/sh
printf '%s' '<div class="banner feed-stale" data-signal="feed-stale">Feed stale</div>'`,
      );
      env.PM_DRILL_PAGE_CMD = join(base, "page");
    } else if (scn.page === "silent") {
      writeExe(
        join(base, "page"),
        `#!/bin/sh
printf '%s' '<div class="card">3 of 3 services up</div>'`,
      );
      env.PM_DRILL_PAGE_CMD = join(base, "page");
    } else if (scn.page === "fail") {
      writeExe(join(base, "page"), "#!/bin/sh\nexit 1\n");
      env.PM_DRILL_PAGE_CMD = join(base, "page");
    }
    const r = spawnSync("bash", [DRILL], {
      env,
      encoding: "utf8",
      timeout: 30000,
    });
    return {
      status: r.status,
      stdout: r.stdout ?? "",
      stderr: r.stderr ?? "",
      systemctlLog: readFileSync(stubLog, "utf8"),
      originalFeed,
      feedBytes: readFileSync(feedFile, "utf8"),
      resultFile: join(state, "age-feed-result.json"),
      evidenceFile: join(state, "age-feed-health-at-signal.json"),
      originalFile: join(state, "age-feed-original.json"),
    };
  } finally {
    lockHolder?.kill();
  }
}

describe("age-feed drill: refusals touch nothing (#2865 lesson)", () => {
  test("collector timer inactive -> refuse, feed unchanged", () => {
    const r = runDrill({ timerActive: false, healthPlan: ["fresh"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("is not active");
    expect(r.feedBytes).toBe(r.originalFeed);
    expect(existsSync(r.resultFile)).toBe(false);
  });

  test("incident flag present -> refuse before any systemctl call", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh"], incidentFlag: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("incident flag present");
    expect(r.systemctlLog.trim()).toBe("");
    expect(r.feedBytes).toBe(r.originalFeed);
  });

  test("drill lock held -> refuse, feed unchanged", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh"], holdLock: true });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("holds the lock");
    expect(r.feedBytes).toBe(r.originalFeed);
  });

  test("aggregator health unreadable -> refuse, feed unchanged", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fail"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("refusing to drill blind");
    expect(r.feedBytes).toBe(r.originalFeed);
  });

  test("feed already stale -> refuse (real incident), feed unchanged", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["stale"] });
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("already stale");
    expect(r.feedBytes).toBe(r.originalFeed);
  });
});

describe("age-feed drill: a real run ages, notices, and restores", () => {
  test("fresh -> aged -> signal fires -> restore confirmed, outcome passed", () => {
    // Health reads in order: baseline (fresh), wait-poll (stale ->
    // signal), restore confirmation (fresh).
    const r = runDrill({ timerActive: true, healthPlan: ["fresh", "stale", "fresh"] });
    expect(r.status).toBe(0);
    expect(r.feedBytes).toBe(r.originalFeed);
    expect(existsSync(r.resultFile)).toBe(true);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.drill).toBe("age-feed");
    expect(result.outcome).toBe("passed");
    expect(result.signal_fired).toBe(true);
    expect(result.health_degraded).toBe(true);
    expect(result.watchdog_item_seen).toBe(true);
    expect(result.restoration_confirmed).toBe(true);
    expect(result.aged_ms).toBeGreaterThan(result.stale_threshold_ms);
    const evidence = JSON.parse(readFileSync(r.evidenceFile, "utf8"));
    expect(evidence.degraded).toBe(true);
  });

  test("collector rewrites the feed mid-drill -> re-aged, still fires and restores", () => {    // health reads: baseline fresh, wait-poll fresh (collector's write
    // landed during the sleep), wait-poll stale -> signal, restore fresh.
    const r = runDrill({
      timerActive: true,
      healthPlan: ["fresh", "fresh", "stale", "fresh"],
      rewriteOnSleep: true,
    });
    expect(r.status).toBe(0);
    expect(r.stdout).toContain("SIGNAL FIRED");
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("passed");
    expect(result.signal_fired).toBe(true);
    expect(result.restoration_confirmed).toBe(true);
    // The drill re-aged the collector's fresh write (it presented a
    // stale-but-alive feed), then restored the saved bytes on exit.
    expect(r.feedBytes).toBe(r.originalFeed);
  });
});

describe("age-feed drill: rendered-page stale warning (#1116 item 5)", () => {
  test("page carrying the stale banner is recorded as seen", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh", "stale", "fresh"], page: "warn" });
    expect(r.status).toBe(0);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("passed");
    expect(result.page_warn_seen).toBe(true);
  });

  test("readable page without the banner is recorded as not seen", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh", "stale", "fresh"], page: "silent" });
    expect(r.status).toBe(0);
    expect(r.stdout).toContain("has no stale-feed warning");
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.page_warn_seen).toBe(false);
  });

  test("unreadable page is recorded as unknown, drill still passes on the health signal", () => {
    const r = runDrill({ timerActive: true, healthPlan: ["fresh", "stale", "fresh"], page: "fail" });
    expect(r.status).toBe(0);
    const result = JSON.parse(readFileSync(r.resultFile, "utf8"));
    expect(result.outcome).toBe("passed");
    expect(result.page_warn_seen).toBe(null);
  });
});
