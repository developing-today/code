// check-provenance-matrix.ts — the #1200 consistency check.
//
// The capability provenance matrix in this directory's README claims
// specific facts about where each aggregator facet reads from and how
// the projector and Pad are deployed. Those claims used to live as
// prose in three places (pm-stack README, this README, /program) and
// silently diverged — #1200 is the record of that failure.
//
// This script derives the load-bearing facts from the configuration
// and code of record and fails (exit 1, naming each divergence) when
// any surface contradicts them:
//
//   - nixos/services/pm-stack/default.nix declares the pm-projector
//     unit and pins the aggregator's PM_PROJECTOR_URL to it.
//   - install.sh renders PM_AGG_SOURCE=live for the deployment.
//   - src/sources/live.ts wires items pool-first (fleet feed Project
//     pool, then projector, then fixtures) and comments via the
//     projector with a fixture fallback.
//   - src/program.ts (/program) must not claim the comment projection
//     is fixture-backed or still to come.
//   - README.md carries the matrix, naming the projector and Pad.
//   - pm-stack/README.md points at the matrix as the source of truth.
//
// Run after changing any facet's source, the projector's deployment,
// or the matrix itself:
//
//   bun run scripts/check-provenance-matrix.ts

import { readFileSync } from "node:fs";
import { join } from "node:path";

const aggDir = join(import.meta.dir, "..");
const repoRoot = join(aggDir, "..", "..");

const read = (rel: string) => readFileSync(join(repoRoot, rel), "utf8");

const failures: string[] = [];
const check = (ok: boolean, message: string) => {
  if (ok) console.log(`ok — ${message}`);
  else failures.push(message);
};

// --- Facts from configuration and code -------------------------------------

const nix = read("nixos/services/pm-stack/default.nix");
check(nix.includes("pm-projector = {"), "NixOS module declares the pm-projector service");
check(
  nix.includes('"PM_PROJECTOR_URL=http://127.0.0.1:'),
  "NixOS module pins the aggregator's PM_PROJECTOR_URL to the local projector (projector is deployed, not off)",
);

const install = read("pm-stack/aggregator/install.sh");
check(install.includes("PM_AGG_SOURCE=live"), "install.sh renders PM_AGG_SOURCE=live for the deployment");

const live = read("pm-stack/aggregator/src/sources/live.ts");
const poolIdx = live.indexOf("poolFromFeed(feed)");
const projectorItemsIdx = live.indexOf("projectorItems(cfg.projector.baseUrl");
check(
  poolIdx >= 0 && projectorItemsIdx > poolIdx,
  "live.ts items() resolves the fleet-feed Project pool before the projector (pool-first)",
);
const commentsStart = live.indexOf("async comments(refs)");
const commentsBody = commentsStart >= 0 ? live.slice(commentsStart, commentsStart + 600) : "";
check(
  commentsBody.includes("projectorComments(cfg.projector.baseUrl"),
  "live.ts comments() reads the GitHub comment projection from the projector",
);
check(!live.includes("projector is dead by config"), "live.ts carries no stale 'projector is dead by config' claim");

// --- Surfaces must agree -----------------------------------------------------

const program = read("pm-stack/aggregator/src/program.ts");
check(
  !program.includes("GitHub comment projection is fixture-backed"),
  "/program (program.ts) does not claim the comment projection is fixture-backed",
);
check(
  !program.includes("GitHub comment projection off fixtures"),
  "/program (program.ts) does not list the live comment projection as future work",
);
check(
  program.includes("live from the projector"),
  "/program (program.ts) states the comment projection reads live from the projector",
);

const aggReadme = read("pm-stack/aggregator/README.md");
check(
  aggReadme.includes("## Capability provenance matrix (#1200)"),
  "aggregator README carries the capability provenance matrix",
);
check(
  aggReadme.includes("**Projector** (service)") && aggReadme.includes("**Pad** (service)"),
  "the matrix names the projector and Pad explicitly",
);
check(aggReadme.includes("Fleet feed's GitHub Project pool FIRST"), "the matrix states items resolve pool-first");
check(
  !aggReadme.includes("Fixture-backed today:** nothing, when the stack is up"),
  "aggregator README carries no unqualified 'nothing is fixture-backed' claim",
);

const stackReadme = read("pm-stack/README.md");
check(
  stackReadme.includes("Capability provenance matrix"),
  "pm-stack README points at the matrix as the provenance source of truth",
);
check(
  stackReadme.includes("the aggregator reads no Pad data"),
  "pm-stack README states the aggregator reads no Pad data",
);

if (failures.length) {
  console.error(`\ncheck-provenance-matrix: ${failures.length} divergence(s):`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("\ncheck-provenance-matrix: matrix, READMEs, /program, code and NixOS module agree.");
