// Checkout drift for the fleet-feed collector (#1116 item 3): the
// collector runs from a git checkout of this repo (the sandbox
// pm-feed clone), so a checkout that has fallen behind origin/main
// means the collector itself is running old code — and the feed it
// produces can lie quietly. The collector measures the drift at
// collection time and embeds it in the feed; the aggregator surfaces
// it verbatim in the health view.
//
// Honesty rules (the health.ts style): the state is one of
// "in-sync", "behind" (with the exact count), or "unknown" WITH a
// reason. Nothing here invents a state: a git failure, a missing
// ref, or a checkout with local commits is "unknown" naming why.
import { execFile } from "node:child_process";

export type CheckoutDriftState = "in-sync" | "behind" | "unknown";

export interface CheckoutDrift {
  state: CheckoutDriftState;
  /** HEAD of the collector's checkout, when git could read it. */
  head?: string;
  /** origin/main as the checkout's refs know it, when readable. */
  originMain?: string;
  /** Commits HEAD is behind origin/main; set for in-sync (0) and behind (>0). */
  behindBy?: number;
  /** Why the state is unknown; set iff state === "unknown". */
  reason?: string;
}

/** The minimal git runner shape — injectable for tests. */
export type GitRun = (cwd: string, args: string[]) => Promise<string>;

export const defaultGitRun: GitRun = (cwd, args) =>
  new Promise((resolve, reject) => {
    execFile("git", args, { cwd, timeout: 10_000, maxBuffer: 1 << 20 }, (err, stdout) => {
      if (err) reject(err);
      else resolve(stdout.trim());
    });
  });

async function revParse(run: GitRun, cwd: string, ref: string): Promise<string> {
  const sha = await run(cwd, ["rev-parse", "--verify", `${ref}^{commit}`]);
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error(`git rev-parse ${ref} returned ${JSON.stringify(sha)}`);
  return sha;
}

async function countRange(run: GitRun, cwd: string, range: string): Promise<number> {
  const out = await run(cwd, ["rev-list", "--count", range]);
  const n = Number(out);
  if (!Number.isInteger(n) || n < 0) throw new Error(`git rev-list --count ${range} returned ${JSON.stringify(out)}`);
  return n;
}

/**
 * Measure the drift of the checkout at `cwd` vs its origin/main ref.
 * The ref is whatever the checkout's last fetch left — this function
 * never fetches (collection must not mutate the checkout it runs
 * from), so a stale ref understates drift rather than inventing it;
 * the health view shows both SHAs so a human can check.
 */
export async function computeCheckoutDrift(cwd: string, run: GitRun = defaultGitRun): Promise<CheckoutDrift> {
  let head: string;
  let originMain: string;
  try {
    head = await revParse(run, cwd, "HEAD");
    originMain = await revParse(run, cwd, "origin/main");
  } catch (err) {
    return { state: "unknown", reason: `checkout refs unreadable: ${err instanceof Error ? err.message : String(err)}` };
  }
  let behindBy: number;
  let aheadBy: number;
  try {
    behindBy = await countRange(run, cwd, "HEAD..origin/main");
    aheadBy = await countRange(run, cwd, "origin/main..HEAD");
  } catch (err) {
    return {
      state: "unknown",
      head,
      originMain,
      reason: `drift count failed: ${err instanceof Error ? err.message : String(err)}`,
    };
  }
  if (aheadBy > 0) {
    return {
      state: "unknown",
      head,
      originMain,
      behindBy,
      reason: `checkout has ${aheadBy} local commit(s) not on origin/main — not a clean pull of main`,
    };
  }
  return behindBy > 0
    ? { state: "behind", head, originMain, behindBy }
    : { state: "in-sync", head, originMain, behindBy: 0 };
}

/** The drift a health view reports when the feed carries none
 *  (a feed written before this check existed). */
export function unreportedCheckoutDrift(): CheckoutDrift {
  return { state: "unknown", reason: "collector did not report checkout drift (feed predates the #1116 drift check)" };
}
