// TS CLI sync verbs (#12591; design §3.2, §7.2) — the CLI side
// of the sync door. §7.2 answers #9016's "small library surface"
// candidate for the hosted world with the service itself: the TS
// CLI + the service's route modules are the shared surface, and
// these verbs are the CLI half. The thin CLI owns rendering: it
// takes the flags an agent workflow types, builds the same
// request documents the REST door validates (src/sync.ts's
// parsers validate the built shape — one validation path for
// both shapes), and renders the door's response document
// JSON-first, exactly as the service answered it. This module is
// the verb logic; a CLI entrypoint wires it to argv/stdout and to
// an authenticated fetch of the door (the session carriage is
// src/session-resolution.ts).

import { FailClosed } from "./errors.ts";
import {
  parseFlowSyncRequest,
  parseProjectSyncRequest,
  type FlowSyncRequest,
  type FlowSyncResponse,
  type ProjectSyncRequest,
  type ProjectSyncResponse,
} from "./sync.ts";

/** Take the value after flag `flag` at argv[i]; a missing value
 * refuses rather than being silently dropped. */
function flagValue(argv: string[], i: number, flag: string): string {
  const value = argv[i + 1];
  if (value === undefined || value.startsWith("--")) {
    throw new FailClosed(400, "invalid_sync", `sync flag ${flag} requires a value`);
  }
  return value;
}

function unknownFlag(flag: string): FailClosed {
  return new FailClosed(400, "invalid_sync", `unknown sync flag: ${flag}`);
}

/** Parse `sync project` verb arguments into the door's request
 * document:
 *   --owner <login> --project <number>
 *   --add-issue <owner/repo#number> [--add-status <Status>]
 *   --set-status <owner/repo#number>=<Status>
 *   --set-item-status <item-id>=<Status>
 * Adds carry the script's add_and_set shape: every added item is
 * set to --add-status (default "Todo", the script's initial
 * status) in the same request. A status value is everything
 * after the first `=` (status names carry spaces: "In Review"). */
export function parseProjectSyncCliArgs(argv: string[]): ProjectSyncRequest {
  let owner: string | undefined;
  let project: string | undefined;
  let addStatus = "Todo";
  const ops: Record<string, unknown>[] = [];
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    switch (flag) {
      case "--owner":
        owner = flagValue(argv, i, flag);
        i++;
        break;
      case "--project":
        project = flagValue(argv, i, flag);
        i++;
        break;
      case "--add-status":
        addStatus = flagValue(argv, i, flag);
        i++;
        break;
      case "--add-issue": {
        const ref = flagValue(argv, i, flag);
        i++;
        const match = /^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#(\d+)$/.exec(ref);
        if (!match) {
          throw new FailClosed(400, "invalid_sync", `--add-issue takes <owner/repo#number>, got ${ref}`);
        }
        // Ops stay in argv order: execution is serial, and a set
        // naming an item an earlier add in the same request
        // created must see it. --add-status is read at the end,
        // so it applies to every add wherever it appears.
        ops.push({ op: "add-issue", repo: match[1], number: Number(match[2]), kind: "issue" });
        break;
      }
      case "--set-status": {
        const value = flagValue(argv, i, flag);
        i++;
        const eq = value.indexOf("=");
        if (eq <= 0) {
          throw new FailClosed(400, "invalid_sync", "--set-status takes <owner/repo#number>=<Status>");
        }
        ops.push({ op: "set-status", issue: value.slice(0, eq), to: value.slice(eq + 1) });
        break;
      }
      case "--set-item-status": {
        const value = flagValue(argv, i, flag);
        i++;
        const eq = value.indexOf("=");
        if (eq <= 0) {
          throw new FailClosed(400, "invalid_sync", "--set-item-status takes <item-id>=<Status>");
        }
        ops.push({ op: "set-status", item_id: value.slice(0, eq), to: value.slice(eq + 1) });
        break;
      }
      default:
        throw unknownFlag(flag);
    }
  }
  if (owner === undefined || project === undefined) {
    throw new FailClosed(400, "invalid_sync", "sync project requires --owner and --project");
  }
  for (const op of ops) {
    if (op["op"] === "add-issue") op["status"] = addStatus;
  }
  if (ops.length === 0) {
    throw new FailClosed(
      400,
      "invalid_sync",
      "sync project requires at least one --add-issue / --set-status / --set-item-status",
    );
  }
  // One validation path: the built document goes through the
  // door's own parser, so a CLI refusal and a REST refusal are
  // the same refusal.
  return parseProjectSyncRequest({
    project: { owner, number: Number(project) },
    ops,
  });
}

/** Parse `sync flow` verb arguments into the door's request
 * document:
 *   --flow <verdict|tend|review> --repo <owner/name>
 *   --number <n> [--kind issue|pr] --marker <token>
 *   --body <text> [--add-label <name>]... [--remove-label <name>]... */
export function parseFlowSyncCliArgs(argv: string[]): FlowSyncRequest {
  const raw: Record<string, unknown> = {};
  const add: string[] = [];
  const remove: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    switch (flag) {
      case "--flow":
        raw["flow"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--repo":
        raw["repo"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--number":
        raw["number"] = Number(flagValue(argv, i, flag));
        i++;
        break;
      case "--kind":
        raw["kind"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--marker":
        raw["marker"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--body":
        raw["body"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--add-label":
        add.push(flagValue(argv, i, flag));
        i++;
        break;
      case "--remove-label":
        remove.push(flagValue(argv, i, flag));
        i++;
        break;
      default:
        throw unknownFlag(flag);
    }
  }
  for (const field of ["flow", "repo", "number", "marker", "body"]) {
    if (raw[field] === undefined) {
      throw new FailClosed(400, "invalid_sync", `sync flow requires --${field.replace("_", "-")}`);
    }
  }
  if (add.length > 0 || remove.length > 0) {
    raw["labels"] = { add, remove };
  }
  return parseFlowSyncRequest(raw);
}

/** Render a sync response the CLI way: JSON first (design §3.2) —
 * the response document itself, pretty-printed, nothing else. */
export function renderSyncResponse(response: ProjectSyncResponse | FlowSyncResponse): string {
  return `${JSON.stringify(response, null, 2)}\n`;
}
