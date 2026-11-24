// TS CLI backfill verbs (#14312) — the CLI side of the ingestion
// door's backfill + divergence surface. The verbs name a repo
// scope exactly as the routes accept it ({ "repos": [...] } in
// the body; no --repo at all → the deployment's configured
// scope, the routes' own fallback), and render the service's
// documents JSON-first, like every verb. This module is the
// parser; src/cli.ts wires it to argv/stdout and to the doors.

import { FailClosed } from "./errors.ts";

const REPO_FORM = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

function usage(message: string): FailClosed {
  return new FailClosed(400, "invalid_cli", message);
}

/** Parse the [--repo <owner/name>]... scope shared by the
 * `backfill` and `divergence` verbs. Every flag takes exactly
 * one value in owner/name form; an unknown flag, a missing
 * value, or a malformed repo refuses rather than being silently
 * dropped — a dropped repo is a scope the operator did not ask
 * for, not an empty one. */
export function parseRepoScopeArgs(argv: string[], verb: string): string[] {
  const repos: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (flag !== "--repo") {
      throw usage(`unknown ${verb} flag: ${flag}`);
    }
    const value = argv[i + 1];
    if (value === undefined || value.startsWith("--")) {
      throw usage(`${verb} flag --repo requires a value`);
    }
    if (!REPO_FORM.test(value)) {
      throw usage(`${verb} --repo takes <owner/name>, got ${value}`);
    }
    repos.push(value);
    i++;
  }
  return repos;
}

/** `backfill runs` takes no arguments at all: the recorded runs
 * are the service's record, read whole. Anything on the line
 * refuses. */
export function parseBackfillRunsArgs(argv: string[]): void {
  if (argv.length > 0) {
    throw usage(`backfill runs takes no arguments, got: ${argv.join(" ")}`);
  }
}
