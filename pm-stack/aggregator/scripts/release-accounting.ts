// Retroactive release-milestone accounting (#9067; design section 4):
// computes close-out snapshots for the existing program milestones of
// developing-today/code from live GitHub state — counts only, no
// closures, no milestone edits. Classification rides issue labels
// (epic marker labels, design/<name>, kind/design — see releases.ts);
// PRs assigned to a milestone are not milestone coverage.
//
// Usage: bun run scripts/release-accounting.ts [--out <file>]
// Reads go through the gh CLI only. Output is the same snapshot shape
// /api/releases.json serves, so the retroactive record and the live
// API can be diffed directly.
import { computeMilestoneSnapshot, type MilestoneItemLike } from "../src/releases.ts";
import type { MilestoneRef } from "../src/model.ts";

const REPO = "developing-today/code";

interface GhMilestone {
  number: number;
  title: string;
  due_on: string | null;
  closed_at: string | null;
}

interface GhIssue {
  number: number;
  state: string;
  labels?: { name?: string }[];
  pull_request?: unknown;
}

function ghApi(path: string): unknown {
  const proc = Bun.spawnSync(["gh", "api", path], { stdout: "pipe", stderr: "pipe" });
  if (proc.exitCode !== 0) {
    throw new Error(`gh api ${path} failed (${proc.exitCode}): ${new TextDecoder().decode(proc.stderr).slice(0, 400)}`);
  }
  return JSON.parse(new TextDecoder().decode(proc.stdout));
}

function milestoneIssues(milestoneNumber: number): MilestoneItemLike[] {
  const items: MilestoneItemLike[] = [];
  for (let page = 1; ; page++) {
    const batch = ghApi(
      `repos/${REPO}/issues?milestone=${milestoneNumber}&state=all&per_page=100&page=${page}`,
    ) as GhIssue[];
    for (const issue of batch) {
      if (issue.pull_request) continue;
      items.push({
        ref: `${REPO}#${issue.number}`,
        state: issue.state === "closed" ? "closed" : "open",
        contentKind: "issue",
        labels: (issue.labels ?? []).map((l) => l.name).filter((x): x is string => typeof x === "string"),
      });
    }
    if (batch.length < 100) return items;
  }
}

function main(): void {
  const outIdx = process.argv.indexOf("--out");
  const outPath = outIdx >= 0 ? process.argv[outIdx + 1] : undefined;
  const milestones = ghApi(`repos/${REPO}/milestones?state=all&per_page=100`) as GhMilestone[];
  const snapshots = milestones
    .map((m) => {
      const ref: MilestoneRef = { title: m.title, dueOn: m.due_on, closedAt: m.closed_at };
      return { number: m.number, ...computeMilestoneSnapshot(ref, milestoneIssues(m.number)) };
    })
    .sort((a, b) => a.milestone.title.localeCompare(b.milestone.title));
  const doc = {
    generatedAt: new Date().toISOString(),
    repo: REPO,
    source: "scripts/release-accounting.ts via gh api (issues by milestone, labels classification) — #9067 retroactive pass",
    snapshots,
  };
  const text = `${JSON.stringify(doc, null, 2)}\n`;
  if (outPath) {
    Bun.write(outPath, text);
    console.log(`wrote ${outPath}: ${snapshots.length} milestone snapshots`);
  } else {
    console.log(text);
  }
}

main();
