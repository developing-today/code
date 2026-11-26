// Edition drift for the laptop PM edition (CF-9, #14949; #14107 M2):
// the laptop aggregator/console/dispatch/projector units are the
// fallback tier of the PM stack, and the fallback tier must never
// silently become the stale tier (#14059: the aggregator once ran
// 4,687 commits behind main with nothing saying so). The edition
// deploys by merge -> nixos-rebuild switch from this repo, and every
// deployed aggregator build is stamped with the commit it was built
// from (PM_AGG_BUILD_VERSION, served as `version` by /api/health).
// This module measures the distance between that deployed commit and
// origin/main, in the primary checkout's own refs.
//
// The stale rule is TIME, not commit count: main can move hundreds of
// commits on a busy day between two healthy flips, so a count alone
// would cry wolf. Drift is stale when main's head commit is newer
// than the deployed commit by more than EDITION_DRIFT_STALE_MS —
// i.e. main has carried newer code for that long without the edition
// flipping to it. Normal merge -> switch latency (minutes, an hour
// on a slow rebuild) never trips it; a missed flip window does.
//
// Honesty rules (the checkout-drift.ts style): the state is one of
// "in-sync", "behind" (with the exact count and both timestamps), or
// "unknown" WITH a reason. Nothing here invents a state: an
// unstamped build, a commit the checkout has never fetched, a
// non-ancestor (local or diverged) build, or any git failure is
// "unknown" naming why. This function never fetches — the refs are
// whatever the checkout's last fetch left (pm-git-sync fetches on
// its timer), so a stale ref understates drift rather than
// inventing it; both SHAs are reported so a human can check.
import { defaultGitRun, type GitRun } from "./checkout-drift.ts";

export type EditionDriftState = "in-sync" | "behind" | "unknown";

export interface EditionDrift {
  state: EditionDriftState;
  /** The deployed build's commit, when it was stamped with one. */
  deployedCommit?: string;
  /** origin/main as the checkout's refs know it, when readable. */
  mainHead?: string;
  /** Commits the deployed commit is behind origin/main; set for in-sync (0) and behind (>0). */
  behindBy?: number;
  /** Commit timestamp (ISO) of the deployed commit, when readable. */
  deployedCommitAt?: string;
  /** Commit timestamp (ISO) of origin/main's head, when readable. */
  mainHeadAt?: string;
  /** mainHeadAt - deployedCommitAt: how long main has carried code
   *  newer than the deployed build. Set whenever both timestamps are. */
  driftMs?: number;
  /** True iff state is "behind" and driftMs exceeds the stale threshold. */
  stale: boolean;
  /** Why the state is unknown; set iff state === "unknown". */
  reason?: string;
}

/** Past this much main-movement time without a flip, the edition is
 *  stale and the drift alarm fires. Six hours: at least one
 *  reasonable rebuild window has been missed, while ordinary
 *  merge -> switch latency stays far below it. */
export const EDITION_DRIFT_STALE_MS = 6 * 3_600_000;

const SHA_RE = /^[0-9a-f]{40}$/;

function unknown(reason: string, deployedCommit?: string, mainHead?: string): EditionDrift {
  return {
    state: "unknown",
    stale: false,
    reason,
    ...(deployedCommit ? { deployedCommit } : {}),
    ...(mainHead ? { mainHead } : {}),
  };
}

async function revParse(run: GitRun, cwd: string, ref: string): Promise<string> {
  const sha = await run(cwd, ["rev-parse", "--verify", `${ref}^{commit}`]);
  if (!SHA_RE.test(sha)) throw new Error(`git rev-parse ${ref} returned ${JSON.stringify(sha)}`);
  return sha;
}

async function commitTimeIso(run: GitRun, cwd: string, ref: string): Promise<string> {
  const out = await run(cwd, ["log", "-1", "--format=%cI", ref]);
  if (!Number.isFinite(Date.parse(out))) throw new Error(`git log ${ref} returned ${JSON.stringify(out)}`);
  return new Date(Date.parse(out)).toISOString();
}

/**
 * Measure the drift of the deployed build (`deployedCommit`, the
 * aggregator's PM_AGG_BUILD_VERSION stamp) vs origin/main in the
 * checkout at `repoDir`. See the header for the stale rule and the
 * honesty contract.
 */
export async function computeEditionDrift(
  deployedCommit: string,
  repoDir: string,
  run: GitRun = defaultGitRun,
  thresholdMs: number = EDITION_DRIFT_STALE_MS,
): Promise<EditionDrift> {
  const deployed = deployedCommit.trim();
  if (!SHA_RE.test(deployed)) {
    return unknown(
      `deployed build marker is not a commit sha (${deployed ? JSON.stringify(deployed.slice(0, 24)) : "empty"}) — the build was not stamped, so its drift cannot be measured`,
    );
  }
  let mainHead: string;
  try {
    mainHead = await revParse(run, repoDir, "origin/main");
  } catch (err) {
    return unknown(
      `origin/main is unreadable in the edition checkout: ${err instanceof Error ? err.message : String(err)}`,
      deployed,
    );
  }
  if (deployed === mainHead) {
    let at: string | undefined;
    try {
      at = await commitTimeIso(run, repoDir, mainHead);
    } catch {
      at = undefined;
    }
    return {
      state: "in-sync",
      deployedCommit: deployed,
      mainHead,
      behindBy: 0,
      ...(at ? { deployedCommitAt: at, mainHeadAt: at } : {}),
      driftMs: 0,
      stale: false,
    };
  }
  try {
    // One call answers both existence and ancestry: merge-base exits
    // non-zero when the commit is absent from this checkout and when
    // it is present but not an ancestor; both are the same honest
    // bucket — this checkout cannot place the deployed build on main.
    await run(repoDir, ["merge-base", "--is-ancestor", deployed, "origin/main"]);
  } catch {
    return unknown(
      `deployed commit ${deployed.slice(0, 8)} is not an ancestor of origin/main in this checkout (never fetched, local-only, or diverged build)`,
      deployed,
      mainHead,
    );
  }
  let behindBy: number;
  let deployedCommitAt: string;
  let mainHeadAt: string;
  try {
    const count = await run(repoDir, ["rev-list", "--count", `${deployed}..origin/main`]);
    behindBy = Number(count);
    if (!Number.isInteger(behindBy) || behindBy < 0) {
      throw new Error(`git rev-list --count returned ${JSON.stringify(count)}`);
    }
    deployedCommitAt = await commitTimeIso(run, repoDir, deployed);
    mainHeadAt = await commitTimeIso(run, repoDir, "origin/main");
  } catch (err) {
    return unknown(`drift measurement failed: ${err instanceof Error ? err.message : String(err)}`, deployed, mainHead);
  }
  if (behindBy === 0) {
    return {
      state: "in-sync",
      deployedCommit: deployed,
      mainHead,
      behindBy: 0,
      deployedCommitAt,
      mainHeadAt,
      driftMs: 0,
      stale: false,
    };
  }
  const driftMs = Math.max(0, Date.parse(mainHeadAt) - Date.parse(deployedCommitAt));
  return {
    state: "behind",
    deployedCommit: deployed,
    mainHead,
    behindBy,
    deployedCommitAt,
    mainHeadAt,
    driftMs,
    stale: driftMs > thresholdMs,
  };
}

function hoursText(ms: number): string {
  const h = ms / 3_600_000;
  return h < 10 ? `${h.toFixed(1)}h` : `${Math.round(h)}h`;
}

/**
 * The stack-sentinel verdict for one drift measurement (#14949):
 * stale is the incident; behind-within-threshold and in-sync are up;
 * unknown is up with the reason in the detail — the alarm never
 * fires on ignorance (an unreachable aggregator is the aggregator
 * probe's incident, not this check's).
 */
export function editionDriftVerdict(
  drift: EditionDrift,
  thresholdMs: number = EDITION_DRIFT_STALE_MS,
): { state: "up" | "down"; detail: string } {
  if (drift.state === "unknown") {
    return { state: "up", detail: `edition drift unknown: ${drift.reason ?? "not measured"}` };
  }
  if (drift.state === "in-sync") {
    return { state: "up", detail: `laptop edition deployed at main head ${drift.mainHead?.slice(0, 8) ?? "?"}` };
  }
  const base =
    `laptop edition is ${drift.behindBy ?? "?"} commits behind main ` +
    `(deployed ${drift.deployedCommit?.slice(0, 8) ?? "?"} at ${drift.deployedCommitAt ?? "?"}, ` +
    `main head ${drift.mainHead?.slice(0, 8) ?? "?"} at ${drift.mainHeadAt ?? "?"}; ` +
    `main has carried newer code for ${hoursText(drift.driftMs ?? 0)})`;
  if (drift.stale) {
    return {
      state: "down",
      detail: `${base} — past the ${hoursText(thresholdMs)} drift threshold; the merge -> switch flip is overdue`,
    };
  }
  return { state: "up", detail: `${base} — within the ${hoursText(thresholdMs)} drift threshold` };
}
