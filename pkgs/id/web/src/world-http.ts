/**
 * Fetch-boundary validation for the world page's HTTP responses (#7833).
 *
 * The server contracts are `HostViewResponse` in
 * `pkgs/id/src/web/world_ws.rs` (snapshot/view/surface nullable,
 * records_sequence a number, records a string-keyed map, next_after a
 * string or null) and the invite handler's `{ capability, display_name }`
 * (capability a non-empty string, per the server test asserting exactly
 * that). A wrong-shape 200 (deploy skew, an error envelope returned with
 * 200, a future contract change) must fail closed through the caller's
 * existing error path — the same discipline the WebSocket paths in
 * world.ts already apply to every frame, and the sibling pages adopted
 * for /api/upload in #7823 (`parseUploadResponse`).
 */

import type { ScreenSurface } from "./world-surface";

export type WorldSnapshotShape = {
  world_id: string;
  current_sequence: number;
  active_module_hash?: string | null;
  participants: Array<{ id: number; display_name: string }>;
};

export type WorldHostViewPage = {
  snapshot: WorldSnapshotShape | null;
  view: string | null;
  surface: ScreenSurface | null;
  records_sequence: number;
  records: Record<string, unknown>;
  next_after: string | null;
};

export type WorldInviteResponse = {
  capability: string;
  display_name: string | null;
};

export type GrantPageWorld = {
  world_id: string;
  current: Array<{
    class: string;
    message: string;
    deniable: boolean;
    load_bearing: boolean;
  }>;
  history: Array<{
    at: string;
    kind: "accept" | "narrow" | "pin" | "grant-page-edit" | "yolo-auto-accept";
    message_set: string[];
  }>;
};

export type GrantPageResponse = { worlds: GrantPageWorld[] };

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const isFiniteNumber = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);
const isStringArray = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === "string");

const isWorldSnapshotShape = (value: unknown): value is WorldSnapshotShape =>
  isRecord(value) &&
  typeof value.world_id === "string" &&
  isFiniteNumber(value.current_sequence) &&
  Array.isArray(value.participants) &&
  value.participants.every(
    (participant) =>
      isRecord(participant) && isFiniteNumber(participant.id) && typeof participant.display_name === "string",
  );

/**
 * Validate one `/api/world/host-view` page before any state is touched.
 * Throws with the page's designed "Host view unavailable" vocabulary so
 * the caller's catch renders that — never a raw engine TypeError, and
 * never partially-rendered bogus records.
 */
export function parseWorldHostViewPage(payload: unknown): WorldHostViewPage {
  if (!isRecord(payload)) {
    throw new Error("Host view unavailable: malformed host-view response (expected an object)");
  }
  const { snapshot, view, surface, records_sequence, records, next_after } = payload;
  if (!isRecord(records)) {
    throw new Error("Host view unavailable: malformed host-view response (records must be an object)");
  }
  if (!isFiniteNumber(records_sequence)) {
    throw new Error("Host view unavailable: malformed host-view response (records_sequence must be a number)");
  }
  if (next_after !== undefined && next_after !== null && typeof next_after !== "string") {
    throw new Error("Host view unavailable: malformed host-view response (next_after must be a string or null)");
  }
  if (snapshot !== undefined && snapshot !== null && !isWorldSnapshotShape(snapshot)) {
    throw new Error("Host view unavailable: malformed host-view response (snapshot failed validation)");
  }
  if (view !== undefined && view !== null && typeof view !== "string") {
    throw new Error("Host view unavailable: malformed host-view response (view must be a string or null)");
  }
  if (surface !== undefined && surface !== null && !isRecord(surface)) {
    throw new Error("Host view unavailable: malformed host-view response (surface must be an object or null)");
  }
  return {
    snapshot: (snapshot ?? null) as WorldSnapshotShape | null,
    view: (view ?? null) as string | null,
    surface: (surface ?? null) as ScreenSurface | null,
    records_sequence,
    records,
    next_after: (next_after ?? null) as string | null,
  };
}

/**
 * The host-view page loop's pagination guard (#6622 contract, #9043).
 *
 * The loop follows `next_after` until null. The cursor's type is
 * already validated by `parseWorldHostViewPage` and the server pins
 * its emission to string-or-null, but the loop itself must still
 * terminate against a buggy or compromised host: a cursor that does
 * not advance past the one just requested would re-request the same
 * page forever, and a cursor stream that never ends would fetch
 * forever. Both end here with the page's designed error vocabulary.
 * Each server page is a full budget-bounded records page, so
 * HOST_VIEW_MAX_PAGES is far beyond any view the page can render.
 */
export const HOST_VIEW_MAX_PAGES = 256;

export function advanceHostViewCursor(input: {
  readonly previous: string | null;
  readonly next: string | null;
  readonly page: number;
}): string | null {
  if (input.next === null) return null;
  if (input.next === input.previous) {
    throw new Error("Host view unavailable: records cursor did not advance; refresh the host view");
  }
  if (input.page >= HOST_VIEW_MAX_PAGES) {
    throw new Error(
      `Host view unavailable: records paging exceeded ${HOST_VIEW_MAX_PAGES} pages; refresh the host view`,
    );
  }
  return input.next;
}

/**
 * Validate an `/api/world/invite` response at the fetch boundary. The
 * capability is a credential: a missing, non-string, or empty one must
 * never reach `showCapability` (which persists it to sessionStorage).
 * `display_name` is cosmetic and falls back gracefully (null) when
 * absent or wrong-typed.
 */
export function parseWorldInviteResponse(payload: unknown): WorldInviteResponse {
  if (!isRecord(payload)) {
    throw new Error("Invite failed: malformed /api/world/invite response (expected an object)");
  }
  const { capability, display_name } = payload;
  if (typeof capability !== "string" || capability.length === 0) {
    throw new Error("Invite failed: malformed /api/world/invite response (capability must be a non-empty string)");
  }
  return {
    capability,
    display_name: typeof display_name === "string" ? display_name : null,
  };
}

const GRANT_HISTORY_KINDS = new Set([
  "accept",
  "narrow",
  "pin",
  "grant-page-edit",
  "yolo-auto-accept",
]);

/** Validate the Rust grant-page projection before rendering any client state. */
export function parseGrantPageResponse(payload: unknown): GrantPageResponse {
  if (!isRecord(payload) || !Array.isArray(payload.worlds)) {
    throw new Error("Grant page unavailable: malformed listing response");
  }
  const worlds = payload.worlds.map((world, index): GrantPageWorld => {
    if (!isRecord(world) || typeof world.world_id !== "string" || !Array.isArray(world.current) || !Array.isArray(world.history)) {
      throw new Error(`Grant page unavailable: malformed world row ${index + 1}`);
    }
    const current = world.current.map((line, lineIndex) => {
      if (
        !isRecord(line) ||
        typeof line.class !== "string" ||
        typeof line.message !== "string" ||
        typeof line.deniable !== "boolean" ||
        typeof line.load_bearing !== "boolean"
      ) {
        throw new Error(`Grant page unavailable: malformed permission line ${lineIndex + 1}`);
      }
      return {
        class: line.class,
        message: line.message,
        deniable: line.deniable,
        load_bearing: line.load_bearing,
      };
    });
    const history = world.history.map((entry, entryIndex) => {
      if (
        !isRecord(entry) ||
        typeof entry.at !== "string" ||
        typeof entry.kind !== "string" ||
        !GRANT_HISTORY_KINDS.has(entry.kind) ||
        !isStringArray(entry.message_set)
      ) {
        throw new Error(`Grant page unavailable: malformed history entry ${entryIndex + 1}`);
      }
      return {
        at: entry.at,
        kind: entry.kind as GrantPageWorld["history"][number]["kind"],
        message_set: entry.message_set,
      };
    });
    return { world_id: world.world_id, current, history };
  });
  return { worlds };
}
