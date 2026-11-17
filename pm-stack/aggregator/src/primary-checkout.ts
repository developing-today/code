// Primary-checkout guard (#9849, recurrence of #8170): the laptop's
// primary clone (~/git/code) is a deploy surface, not a workspace —
// pm-git-sync keeps it on a clean main and the PM services run from
// it. Twice (2026-11-08→10 under #8170, again 2026-11-13/14) a lane
// worked directly in the primary, and the dirt silently blocked
// deploys until a human rescue pass. Prose rules did not stop it.
//
// This module is the surfacing half of the guard: a local check the
// stack sentinel runs on its normal cadence, so a primary that is
// off main or dirty raises ONE stack-health incident naming the
// dirt — through the same damped transition machinery as every
// other service — before it blocks a deploy, instead of the block
// being discovered by the deploy that trips over it. (The launcher
// half — refusing/redirecting a session started in the primary —
// belongs to the session launcher, which does not live in this
// repo; see #9849.)

export interface PrimaryCheckoutStatus {
  /** Current branch, or null when HEAD is detached or unparseable. */
  branch: string | null;
  /** Paths with uncommitted changes (tracked or untracked). */
  dirtyPaths: string[];
}

/**
 * Parse `git status --porcelain=v1 --branch` output. The branch line
 * is `## <branch>[...upstream info]` (or `## HEAD (no branch)` when
 * detached); every other line is one dirty path in XY format.
 */
export function parseGitStatusPorcelain(output: string): PrimaryCheckoutStatus {
  let branch: string | null = null;
  const dirtyPaths: string[] = [];
  for (const line of output.split("\n")) {
    if (line.startsWith("## ")) {
      const rest = line.slice(3).trim();
      if (rest.startsWith("HEAD")) {
        branch = null;
      } else {
        // Strip upstream tracking suffixes: "main...origin/main [ahead 1]".
        branch = rest.split(/[.]{3}|\s/)[0] || null;
      }
      continue;
    }
    if (line.length > 3) {
      // XY + space + path; renames carry "old -> new" — name the new path.
      const path = line.slice(3);
      const arrow = path.indexOf(" -> ");
      dirtyPaths.push(arrow >= 0 ? path.slice(arrow + 4) : path);
    }
  }
  return { branch, dirtyPaths };
}

export interface PrimaryCheckoutVerdict {
  state: "up" | "down";
  detail: string;
}

const MAX_NAMED_PATHS = 5;

/**
 * The guard's verdict: the primary is healthy only on a clean main.
 * Anything else is the deploy-blocking condition, and the detail
 * names the dirt so the incident is actionable without a login.
 */
export function assessPrimaryCheckout(status: PrimaryCheckoutStatus): PrimaryCheckoutVerdict {
  const problems: string[] = [];
  if (status.branch !== "main") {
    problems.push(status.branch === null ? "HEAD is detached (not on main)" : `on branch ${status.branch}, not main`);
  }
  if (status.dirtyPaths.length > 0) {
    const named = status.dirtyPaths.slice(0, MAX_NAMED_PATHS).join(", ");
    const more =
      status.dirtyPaths.length > MAX_NAMED_PATHS ? `, +${status.dirtyPaths.length - MAX_NAMED_PATHS} more` : "";
    problems.push(`${status.dirtyPaths.length} uncommitted path(s): ${named}${more}`);
  }
  if (problems.length === 0) {
    return { state: "up", detail: "primary checkout on clean main" };
  }
  return { state: "down", detail: `primary checkout blocked: ${problems.join("; ")}` };
}

/**
 * Run the check against a real checkout directory. A directory that
 * cannot be read as a git checkout at all is also a down: from the
 * deploy pipeline's side it blocks in exactly the same way.
 */
export function checkPrimaryCheckout(dir: string): PrimaryCheckoutVerdict {
  let output: string;
  try {
    const proc = Bun.spawnSync(["git", "-C", dir, "status", "--porcelain=v1", "--branch"], {
      stdout: "pipe",
      stderr: "pipe",
    });
    if (!proc.success) {
      const stderr = new TextDecoder().decode(proc.stderr).trim().split("\n")[0] ?? "";
      return {
        state: "down",
        detail: `primary checkout blocked: git status failed in ${dir} (${stderr || "unknown error"})`,
      };
    }
    output = new TextDecoder().decode(proc.stdout);
  } catch (err) {
    return {
      state: "down",
      detail: `primary checkout blocked: git status could not run in ${dir} (${err instanceof Error ? err.message : String(err)})`,
    };
  }
  return assessPrimaryCheckout(parseGitStatusPorcelain(output));
}
