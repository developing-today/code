/**
 * Wire shapes the PM tenant store holds, and their validators.
 *
 * These types are the conformance contract for #12825: they mirror
 * #9060's design shapes and the laptop collector's landed output
 * types field for field, so a row the store accepts is a row the
 * collector can emit and the hosted read API can serve.
 *
 * - BoardDef        — #9060 §1; pm-stack/aggregator/src/board-registry.ts
 * - LinkRecord      — #9060 §11 (relation / status / provenance records)
 * - PmMetadataBlock — #9060 §12 (the canonical Metadata block)
 * - WriterLease     — component A design §5 (writer-of-record lease)
 *
 * Validators return a list of problems (empty = valid), mirroring
 * validateBoardRegistry in the aggregator: callers decide whether a
 * problem is fatal; nothing here throws on shape.
 */

export type BoardRole = "overall" | "project" | "wip" | "register" | "ideas";

/** #9060 §1 board registry entry, stored as a pm_boards row. */
export interface BoardDef {
  id: string;
  org: string;
  number: number;
  nodeId: string;
  url: string;
  role: BoardRole;
  parentBoardId?: string;
  siblingOrder: number;
  title: string;
}

const BOARD_ROLES: readonly string[] = ["overall", "project", "wip", "register", "ideas"];

export function validateBoardDef(value: unknown): string[] {
  if (!isRecord(value)) return ["board def must be an object"];
  const problems: string[] = [];
  if (typeof value.id !== "string" || !value.id.trim()) problems.push("board id must not be empty");
  if (typeof value.org !== "string" || !value.org.trim()) problems.push("board org must not be empty");
  if (!Number.isInteger(value.number) || (value.number as number) < 1)
    problems.push(`invalid project number: ${String(value.id)}`);
  if (typeof value.nodeId !== "string" || !value.nodeId.startsWith("PVT_"))
    problems.push(`invalid project node id: ${String(value.id)}`);
  if (typeof value.role !== "string" || !BOARD_ROLES.includes(value.role))
    problems.push(`invalid board role: ${String(value.role)}`);
  if (value.parentBoardId !== undefined && (typeof value.parentBoardId !== "string" || !value.parentBoardId.trim()))
    problems.push(`invalid parent board id: ${String(value.id)}`);
  if (!Number.isInteger(value.siblingOrder)) problems.push(`invalid sibling order: ${String(value.id)}`);
  if (typeof value.title !== "string" || !value.title.trim()) problems.push("board title must not be empty");
  if (typeof value.url === "string" && typeof value.org === "string" && Number.isInteger(value.number)) {
    try {
      if (new URL(value.url).pathname !== `/orgs/${value.org}/projects/${value.number as number}`)
        problems.push(`project URL does not match org/number: ${String(value.id)}`);
    } catch {
      problems.push(`invalid project URL: ${String(value.id)}`);
    }
  } else {
    problems.push(`invalid project URL: ${String(value.id)}`);
  }
  return problems;
}

export type LinkStatus = "active" | "historical" | "removed";

/** #9060 §12 link target for item links: {repo, number}. */
export interface LinkTarget {
  repo: string;
  number: number;
}

/**
 * #9060 §11 link record, as stored in pm_links. The source is an item
 * ref ("owner/repo#123") or a board id; the target is an item
 * ({repo, number}) or a board ({boardId}) — links are level-agnostic.
 */
export interface LinkRecord {
  sourceKind: "item" | "board";
  sourceRef: string;
  target: LinkTarget | { boardId: string };
  relation: string;
  status: LinkStatus;
  since?: string | null;
  until?: string | null;
  reason?: string | null;
}

const LINK_STATUSES: readonly string[] = ["active", "historical", "removed"];

/** "owner/repo#123" — the registry's canonical key for an issue or PR. */
export function isRefKey(value: unknown): value is string {
  return typeof value === "string" && /^[^/\s#]+\/[^/\s#]+#\d+$/.test(value);
}

/** Target / relation / status / until / reason checks shared by pm_links rows and §12 block links. */
function validateLinkFields(value: Record<string, unknown>): string[] {
  const problems: string[] = [];
  const target = value.target;
  const targetIsItem = isRecord(target) && typeof target.repo === "string" && Number.isInteger(target.number);
  const targetIsBoard = isRecord(target) && typeof target.boardId === "string" && target.boardId.trim().length > 0;
  if (!targetIsItem && !targetIsBoard) problems.push("link target must be {repo, number} or {boardId}");
  if (typeof value.relation !== "string" || !value.relation.trim()) problems.push("link relation must not be empty");
  if (typeof value.status !== "string" || !LINK_STATUSES.includes(value.status))
    problems.push(`invalid link status: ${String(value.status)}`);
  // #9060 §11: historical links carry until + reason; removed links
  // carry their reason so the removal stays auditable.
  if (value.status === "historical") {
    if (typeof value.until !== "string" || !value.until) problems.push("historical link must carry until");
    if (typeof value.reason !== "string" || !value.reason) problems.push("historical link must carry a reason");
  }
  if (value.status === "removed" && (typeof value.reason !== "string" || !value.reason))
    problems.push("removed link must carry a reason");
  return problems;
}

export function validateLinkRecord(value: unknown): string[] {
  if (!isRecord(value)) return ["link record must be an object"];
  const problems: string[] = [];
  if (value.sourceKind === "item") {
    if (!isRefKey(value.sourceRef)) problems.push(`link source is not an item ref: ${String(value.sourceRef)}`);
  } else if (value.sourceKind === "board") {
    if (typeof value.sourceRef !== "string" || !value.sourceRef.trim())
      problems.push("link source board id must not be empty");
  } else {
    problems.push(`invalid link source kind: ${String(value.sourceKind)}`);
  }
  return [...problems, ...validateLinkFields(value)];
}

/** #9060 §12: the canonical Metadata block every issue and PR carries. */
export interface PmMetadataBlock {
  pm_meta: 1;
  direct_parent: { repo: string; number: number; kind: string } | null;
  links: Array<{
    target: LinkTarget;
    relation: string;
    status: LinkStatus;
    since?: string | null;
    until?: string | null;
    reason?: string | null;
  }>;
  lineage: {
    design: string | null;
    roadmap_item: string | null;
    priority: string | null;
  };
  provenance: Record<string, unknown>;
}

export function validateMetadataBlock(value: unknown): string[] {
  if (!isRecord(value)) return ["metadata block must be an object"];
  const problems: string[] = [];
  if (value.pm_meta !== 1) problems.push(`unsupported pm_meta version: ${String(value.pm_meta)}`);
  const parent = value.direct_parent;
  if (parent !== null) {
    if (
      !isRecord(parent) ||
      typeof parent.repo !== "string" ||
      !Number.isInteger(parent.number) ||
      typeof parent.kind !== "string"
    )
      problems.push("direct_parent must be null or {repo, number, kind}");
  }
  if (!Array.isArray(value.links)) {
    problems.push("links must be an array");
  } else {
    value.links.forEach((link, index) => {
      if (!isRecord(link)) {
        problems.push(`links[${index}] must be an object`);
        return;
      }
      // §12 block links are item → item by construction (the block's
      // own item is the source), so only the shared fields apply.
      for (const problem of validateLinkFields(link)) problems.push(`links[${index}]: ${problem}`);
    });
  }
  if (!isRecord(value.lineage)) {
    problems.push("lineage must be an object");
  } else {
    for (const key of ["design", "roadmap_item", "priority"] as const) {
      const entry = value.lineage[key];
      if (entry !== null && typeof entry !== "string") problems.push(`lineage.${key} must be a string or null`);
    }
  }
  if (!isRecord(value.provenance)) problems.push("provenance must be an object");
  return problems;
}

export type WriterKind = "laptop" | "hosted";

/** Component A design §5: the writer-of-record lease row. */
export interface WriterLease {
  boardId: string;
  writer: WriterKind;
  epoch: number;
  heartbeatAt: string;
}

export function validateWriterLease(value: unknown): string[] {
  if (!isRecord(value)) return ["writer lease must be an object"];
  const problems: string[] = [];
  if (typeof value.boardId !== "string" || !value.boardId.trim()) problems.push("lease board id must not be empty");
  if (value.writer !== "laptop" && value.writer !== "hosted")
    problems.push(`invalid lease writer: ${String(value.writer)}`);
  if (!Number.isInteger(value.epoch) || (value.epoch as number) < 1)
    problems.push(`invalid lease epoch: ${String(value.epoch)}`);
  if (typeof value.heartbeatAt !== "string" || Number.isNaN(Date.parse(value.heartbeatAt)))
    problems.push("lease heartbeat must be an ISO-8601 timestamp");
  return problems;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
