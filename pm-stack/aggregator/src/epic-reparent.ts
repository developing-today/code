import type { RefKey } from "./model.ts";

export interface ReparentIssue {
  ref: RefKey;
  parentRef: RefKey | null;
  state: "OPEN" | "CLOSED";
  labels: readonly string[];
  /** Native ancestors, nearest first. */
  primaryParentChain: readonly RefKey[];
  area: string | null;
}

export interface ReparentEpic {
  ref: RefKey;
  state: "OPEN" | "CLOSED";
  labels: readonly string[];
  /** Native ancestors, nearest first. */
  primaryParentChain: readonly RefKey[];
  area: string | null;
  maintenance: boolean;
}

export type ReparentRelation =
  | "shared_design"
  | "shared_primary_parent"
  | "shared_epic"
  | "shared_area"
  | "maintenance_fallback";

export type ReparentSelection =
  | { kind: "selected"; epic: ReparentEpic; reason: ReparentRelation }
  | { kind: "ambiguous"; reason: Exclude<ReparentRelation, "maintenance_fallback">; refs: RefKey[] }
  | { kind: "unavailable"; reason: "no_open_destination_or_maintenance_epic" };

const intersection = (a: readonly string[], b: readonly string[]) =>
  a.filter((value) => b.includes(value));

function sharedLabel(child: ReparentIssue, epic: ReparentEpic, prefix: string): boolean {
  const childLabels = child.labels.filter((label) => label.startsWith(prefix));
  const epicLabels = new Set(epic.labels.filter((label) => label.startsWith(prefix)));
  return childLabels.some((label) => epicLabels.has(label));
}

/** Select a unique highest-ranked open related epic. Ties are surfaced, never guessed. */
export function selectReparentDestination(
  child: ReparentIssue,
  candidates: readonly ReparentEpic[],
): ReparentSelection {
  const rank: Exclude<ReparentRelation, "maintenance_fallback">[] = [
    "shared_design",
    "shared_primary_parent",
    "shared_epic",
    "shared_area",
  ];
  const related = candidates.filter(
    (candidate) => candidate.state === "OPEN"
      && candidate.ref !== child.ref
      && candidate.ref !== child.parentRef
      && !candidate.primaryParentChain.includes(child.ref),
  );
  for (const relation of rank) {
    const matches = related.filter((candidate) => {
      switch (relation) {
        case "shared_design":
          return sharedLabel(child, candidate, "design/");
        case "shared_primary_parent":
          return intersection(child.primaryParentChain, candidate.primaryParentChain).length > 0;
        case "shared_epic":
          return sharedLabel(child, candidate, "epic/");
        case "shared_area":
          return child.area !== null && child.area === candidate.area;
      }
    });
    if (matches.length === 1) return { kind: "selected", epic: matches[0]!, reason: relation };
    if (matches.length > 1)
      return { kind: "ambiguous", reason: relation, refs: matches.map((match) => match.ref).sort() };
  }
  const maintenance = related.filter((candidate) => candidate.maintenance);
  if (maintenance.length === 1)
    return { kind: "selected", epic: maintenance[0]!, reason: "maintenance_fallback" };
  if (maintenance.length > 1)
    return { kind: "ambiguous", reason: "shared_area", refs: maintenance.map((match) => match.ref).sort() };
  return { kind: "unavailable", reason: "no_open_destination_or_maintenance_epic" };
}

export interface EpicReparentPlan {
  ref: RefKey;
  oldParentRef: RefKey | null;
  newParentRef: RefKey;
  expectedLabels: string[];
  labelsToApply: string[];
}

/** Update the canonical issue-body metadata while preserving all unrelated fields and links. */
export function setHistoricalParentMetadata(
  body: string,
  parentRef: RefKey,
  present: boolean,
  recordedAt = new Date().toISOString().slice(0, 10),
): string {
  const markers = /<!--\s*pm-metadata:start\s*-->[\s\S]*?<!--\s*pm-metadata:end\s*-->/;
  const existing = markers.exec(body)?.[0];
  let metadata: Record<string, unknown> = { pm_meta: 1, links: [] };
  if (existing) {
    const json = /```json\s*([\s\S]*?)```/i.exec(existing)?.[1];
    if (!json) throw new Error("existing PM metadata block has no JSON payload");
    const parsed: unknown = JSON.parse(json);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
      throw new Error("existing PM metadata JSON must be an object");
    const links = (parsed as { links?: unknown }).links;
    if (!Array.isArray(links)) throw new Error("existing PM metadata links must be an array");
    metadata = parsed as Record<string, unknown>;
  }
  const links = metadata.links as unknown[];
  const [repo, rawNumber] = parentRef.split("#");
  const number = Number(rawNumber);
  const isParentLink = (link: unknown) => {
    if (!link || typeof link !== "object") return false;
    const record = link as { relation?: unknown; target?: { repo?: unknown; number?: unknown } };
    return record.relation === "parent" && record.target?.repo === repo && record.target.number === number;
  };
  if (present && links.some((link) => isParentLink(link) && (link as { status?: unknown }).status === "historical"))
    return body;
  const nextLinks = links.filter((link) => !(isParentLink(link) && (link as { status?: unknown }).status === "historical"));
  if (present)
    nextLinks.push({
      target: { repo, number },
      relation: "parent",
      status: "historical",
      until: recordedAt,
      reason: "reparent-on-close",
    });
  metadata.links = nextLinks;
  const block = `<!-- pm-metadata:start -->\n\`\`\`json\n${JSON.stringify(metadata, null, 2)}\n\`\`\`\n<!-- pm-metadata:end -->`;
  if (existing) return body.replace(existing, () => block);
  if (!present) return body;
  return `${body}${body ? "\n\n" : ""}${block}\n`;
}

/** Build the exact label set for a move; only the primary-parent label is replaced. */
export function planEpicReparent(
  child: ReparentIssue,
  destination: ReparentEpic,
): EpicReparentPlan {
  if (destination.state !== "OPEN") throw new Error(`destination epic is not open: ${destination.ref}`);
  if (destination.ref === child.ref) throw new Error(`issue cannot be re-parented to itself: ${child.ref}`);
  if (destination.ref === child.parentRef) throw new Error(`destination is already the child's parent: ${destination.ref}`);
  if (destination.primaryParentChain.includes(child.ref))
    throw new Error(`destination is a descendant of the child and would create a native-parent cycle: ${destination.ref}`);
  const slugs = destination.labels
    .filter((label) => label.startsWith("epic/"))
    .map((label) => label.slice("epic/".length))
    .filter(Boolean);
  if (slugs.length !== 1)
    throw new Error(`destination must have exactly one epic/<slug> label: ${destination.ref}`);
  const labelsToApply = new Set(child.labels.filter((label) => !label.startsWith("primary-parent/")));
  labelsToApply.add(`primary-parent/${slugs[0]}`);
  return {
    ref: child.ref,
    oldParentRef: child.parentRef,
    newParentRef: destination.ref,
    expectedLabels: [...new Set(child.labels)].sort(),
    labelsToApply: [...labelsToApply].sort(),
  };
}

export interface ReparentGateway {
  /** Live reads, not values reused from destination planning. */
  readChildLive(ref: RefKey): Promise<ReparentIssue>;
  isOpenEpicLive(ref: RefKey): Promise<boolean>;
  /** Atomically re-point the native sub-issue edge (GitHub replaceParent). */
  replaceNativeParent(child: RefKey, oldParent: RefKey | null, newParent: RefKey): Promise<void>;
  /** Remove a newly-created native edge when compensating a child that had no parent. */
  removeNativeParent(child: RefKey, parent: RefKey): Promise<void>;
  /** Set the exact planned labels after verifying the plan's expected label set. */
  replaceLabels(child: RefKey, labels: readonly string[]): Promise<void>;
  /** Returns true only when this operation created a new historical link. */
  recordHistoricalParent(child: RefKey, parent: RefKey): Promise<boolean>;
  removeHistoricalParent(child: RefKey, parent: RefKey): Promise<void>;
  addComment(ref: RefKey, body: string): Promise<void>;
}

/** Apply a previously reviewed plan only if live child state still matches it. */
export async function applyEpicReparent(
  plan: EpicReparentPlan,
  closingEpicRef: RefKey,
  gateway: ReparentGateway,
): Promise<void> {
  const child = await gateway.readChildLive(plan.ref);
  if (child.state !== "OPEN") throw new Error(`child is not open: ${plan.ref}`);
  if (child.parentRef !== plan.oldParentRef)
    throw new Error(`child parent changed since plan was made: ${plan.ref}`);
  const labels = [...new Set(child.labels)].sort();
  if (JSON.stringify(labels) !== JSON.stringify(plan.expectedLabels))
    throw new Error(`child labels changed since plan was made: ${plan.ref}`);
  if (!(await gateway.isOpenEpicLive(plan.newParentRef)))
    throw new Error(`destination is not an open epic: ${plan.newParentRef}`);

  let historicalParentRecorded = false;
  try {
    await gateway.replaceNativeParent(plan.ref, plan.oldParentRef, plan.newParentRef);
    await gateway.replaceLabels(plan.ref, plan.labelsToApply);
    if (plan.oldParentRef)
      historicalParentRecorded = await gateway.recordHistoricalParent(plan.ref, plan.oldParentRef);
  } catch (cause) {
    const compensationErrors: unknown[] = [];
    if (plan.oldParentRef && historicalParentRecorded) {
      try {
        await gateway.removeHistoricalParent(plan.ref, plan.oldParentRef);
      } catch (error) {
        compensationErrors.push(error);
      }
    }
    try {
      await gateway.replaceLabels(plan.ref, plan.expectedLabels);
    } catch (error) {
      compensationErrors.push(error);
    }
    try {
      if (plan.oldParentRef)
        await gateway.replaceNativeParent(plan.ref, plan.newParentRef, plan.oldParentRef);
      else
        await gateway.removeNativeParent(plan.ref, plan.newParentRef);
    } catch (error) {
      compensationErrors.push(error);
    }
    if (compensationErrors.length)
      throw new AggregateError([cause, ...compensationErrors], "re-parent failed and compensation was incomplete");
    throw cause;
  }
  const note = `Re-parented from ${plan.oldParentRef ?? "(no parent)"} to ${plan.newParentRef} as part of closing ${closingEpicRef}; labels were preserved except for the primary-parent marker.`;
  await gateway.addComment(plan.ref, note);
  await gateway.addComment(closingEpicRef, note);
}
