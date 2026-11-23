// The Workers, as one platform.
//
// The repository holds a list of every Cloudflare Worker it contains
// (pm-stack/platform/workers.json): where each lives, what it does,
// whether it is in service and how it gets there. This reads that list
// for the app's Platform screen, wherever the app is served from: a
// machine asks through the fleet binary and the edge asks GitHub
// itself, like every read in reads.ts. A Worker deployed from Actions
// also shows the latest run of its deploy.

/** How a Worker gets into service. */
export const DEPLOYED = ["actions", "by hand", "none"] as const;
export const STATES = ["in service", "not deployed"] as const;
/** What a Worker belongs to, in the order the screen shows them. */
export const GROUPS = ["platform", "fleet", "data", "id", "lab"] as const;

export interface WorkerRow {
  /** Its name on the account (the `name` in its wrangler file). */
  name: string;
  title: string;
  group: (typeof GROUPS)[number];
  /** The directory its wrangler file is in. */
  path: string;
  does: string;
  state: (typeof STATES)[number];
  deploy: (typeof DEPLOYED)[number];
  /** The workflow that deploys it, when one does. */
  workflow: string | null;
  /** Where it answers, when it is in service. */
  address: string | null;
  /** What joining the one platform means for it. */
  next: string;
}

export const WORKERS_FILE = "pm-stack/platform/workers.json";

const NAME = /^[a-z0-9][a-z0-9-]{0,62}$/;
const PATH = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$/;
const WORKFLOW = /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}\.ya?ml$/;
const text = (value: unknown, max: number): string | null =>
  typeof value === "string" && value.trim() !== "" && value.length <= max ? value : null;

/** One Worker as the list may give it, or null. */
export function workerRow(value: unknown): WorkerRow | null {
  const v = value as Record<string, unknown> | null;
  if (!v || typeof v !== "object") return null;
  const title = text(v.title, 80);
  const does = text(v.does, 400);
  const next = text(v.next, 400);
  if (typeof v.name !== "string" || !NAME.test(v.name) || !title || !does || !next) return null;
  if (typeof v.path !== "string" || !PATH.test(v.path) || v.path.includes("..")) return null;
  if (!(GROUPS as readonly unknown[]).includes(v.group)) return null;
  if (!(STATES as readonly unknown[]).includes(v.state) || !(DEPLOYED as readonly unknown[]).includes(v.deploy))
    return null;
  const workflow = v.workflow === null || v.workflow === undefined ? null : v.workflow;
  if (workflow !== null && (typeof workflow !== "string" || !WORKFLOW.test(workflow))) return null;
  // Deployed from Actions means there is a workflow that does it, and
  // nothing that is not in service has a way of getting there.
  if ((v.deploy === "actions") !== (workflow !== null)) return null;
  if ((v.state === "not deployed") !== (v.deploy === "none")) return null;
  const address = v.address === null || v.address === undefined ? null : v.address;
  if (address !== null && (typeof address !== "string" || !/^https:\/\/[a-z0-9.-]+(\/[^\s]*)?$/.test(address)))
    return null;
  if (v.state === "not deployed" && address !== null) return null;
  return {
    name: v.name,
    title,
    group: v.group as WorkerRow["group"],
    path: v.path,
    does,
    state: v.state as WorkerRow["state"],
    deploy: v.deploy as WorkerRow["deploy"],
    workflow: workflow as string | null,
    address: address as string | null,
    next,
  };
}

/** The list as the file gives it: the rows that are well formed, in
 * the order the screen shows them, and how many were not. */
export function workersList(value: unknown): { checked: string | null; workers: WorkerRow[]; malformed: number } {
  const v = value as { checked?: unknown; workers?: unknown } | null;
  const given = Array.isArray(v?.workers) ? v.workers : [];
  const rows = given.map(workerRow);
  const seen = new Set<string>();
  const workers = rows.filter((row): row is WorkerRow => {
    if (!row || seen.has(row.name)) return false;
    seen.add(row.name);
    return true;
  });
  workers.sort((a, b) => GROUPS.indexOf(a.group) - GROUPS.indexOf(b.group));
  return {
    checked: typeof v?.checked === "string" && /^\d{4}-\d{2}-\d{2}$/.test(v.checked) ? v.checked : null,
    workers,
    malformed: given.length - workers.length,
  };
}
