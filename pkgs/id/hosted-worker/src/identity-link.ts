/**
 * SSO relying-party identity links, slice H2 (#14260, child of #9086).
 *
 * The link/unlink API over the `identity_link` table (migration
 * tenancy/migrations/0002_identity_link.sql; design
 * `doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086/` §4.3).
 * One row per linked login door: the provider's subject at that door
 * is the key; email is contact-only and never appears here at all.
 *
 * The ceremony (design §4.3), enforced by every mutating function:
 *
 *   - Link: an authenticated session on the account (the caller
 *     resolved the caller through an EXISTING active door and hands
 *     that session in as `session`) PLUS a completed upstream login
 *     on the new door (`proof`: the door + the subject it issued).
 *     First-link / account creation is the provider's gated path
 *     (design §4.4) and is NOT this API: an account row must already
 *     exist, and links are only ever added to the session's own
 *     account — there is no way to name another account as target.
 *   - Unlink: the account's single active link cannot be unlinked
 *     without a replacement linked in the same operation (the
 *     last-door guard — an account with no doors is unrecoverable
 *     except by Drew-level intervention, so the API makes that
 *     state unreachable). With a replacement, the replacement is
 *     linked first and only then is the old door unlinked; if the
 *     replacement link is refused, nothing changes.
 *   - A door-identity belongs to one account EVER (the table's
 *     UNIQUE (door, provider_sub) spans history): a second account
 *     claiming a door-identity the first account once held — even
 *     after unlink — is refused. The owning account re-linking its
 *     own unlinked door reactivates the retained history row.
 *   - Every link and unlink writes an `identity_link_event` audit
 *     row (actor binding + timestamp) in the same operation
 *     (design §§4.3, 4.8: no unaudited identity-state change).
 *     Refusals change nothing and write nothing. "Same operation"
 *     is literal (#15024): the state change and its audit row
 *     commit in one transaction on the SQL seam (see `atomically`
 *     below), so a failed audit insert leaves no unaudited
 *     identity mutation behind and the caller can simply retry
 *     the operation — a retry after a failed link performs the
 *     full audited link instead of reporting `already_linked`
 *     for a change that was never audited.
 *
 * DB continuity (design §4.3): tenant access keys off
 * `account_id`/`binding_id`, never off a door. Nothing in this
 * module reads or writes `binding`, `tenant_db`, `role_grant` or
 * `credential` rows for effect — the only `binding` read is the
 * ceremony check that the actor binding is the account's own active
 * binding. Linking or unlinking therefore cannot touch tenant rows,
 * re-key a database, or invalidate a session's tokens.
 *
 * Shape notes:
 *
 *   - Results mirror the front door (H1): domain refusals are
 *     returned `{ ok: false, reason }`, never thrown and never
 *     partially applied.
 *   - `resolveAccountIdByDoor` is the lookup H1's front door wires
 *     to the central tenancy DB. It takes the door explicitly: the
 *     (door, provider_sub) pair is the key, and the same subject
 *     string at two doors is two identities by design, so no
 *     sub-only lookup is offered.
 *   - The SQL seam is the same minimal surface the TenantOpsDO
 *     uses (the object's SQLite storage locally; the E6 store
 *     adapter is where a Turso-over-HTTP handle plugs in). Like
 *     the TenantOpsDO, this module relies on the caller's
 *     serialisation (the tenancy object's gate) for interleaving
 *     BETWEEN operations: check-then-write sequences here must
 *     run inside that gate. Within one operation, atomicity is
 *     the module's own job (#15024): the mutation and its audit
 *     event commit together or not at all.
 */

export const DOORS = ["github", "google", "microsoft", "devtoday"] as const;
export type Door = (typeof DOORS)[number];

const DOOR_SET: ReadonlySet<string> = new Set(DOORS);

/** The minimal SQL surface (DO SqlStorage shape; see header). */
export interface IdentityLinkSql {
  exec(
    query: string,
    ...params: (string | null)[]
  ): { toArray(): Record<string, unknown>[] };
  /**
   * Optional native atomic block (a Durable Object caller backs
   * this with `storage.transactionSync`; a Turso adapter with its
   * own transaction call). When a seam provides it, `atomically`
   * prefers it; otherwise the module drives
   * BEGIN IMMEDIATE / COMMIT / ROLLBACK through `exec`, which is
   * the plain-SQLite path the tests exercise.
   */
  transaction?<T>(fn: () => T): T;
}

/** Seams with an atomic block already open. The mark makes
 * `atomically` re-entrant: `unlinkDoor`'s replacement swap calls
 * `linkDoor` inside the swap's block, and the inner call must
 * join the outer transaction, never open a nested one. */
const seamsInTransaction = new WeakSet<object>();

/**
 * Run `fn` atomically on the seam (#15024): every write it makes
 * commits, or — if any write throws, in particular the audit
 * event insert — none of them does, and the error propagates.
 * This is what makes the ceremony's "no unaudited identity-state
 * change" rule survive persistence failures, for a whole
 * replacement swap as much as for a single link or unlink.
 */
function atomically<T>(sql: IdentityLinkSql, fn: () => T): T {
  if (seamsInTransaction.has(sql)) return fn();
  seamsInTransaction.add(sql);
  try {
    if (typeof sql.transaction === "function") return sql.transaction(fn);
    sql.exec("BEGIN IMMEDIATE");
    try {
      const out = fn();
      sql.exec("COMMIT");
      return out;
    } catch (err) {
      // A failed rollback must not mask the original error.
      try {
        sql.exec("ROLLBACK");
      } catch {
        // The original error is the one the caller needs.
      }
      throw err;
    }
  } finally {
    seamsInTransaction.delete(sql);
  }
}

/** An authenticated session on an account, resolved by the caller
 * through an existing active door (the front door's output). */
export interface LinkSession {
  accountId: string;
  /** The session's binding — must be the account's active binding. */
  bindingId: string;
}

/** A completed upstream login on a door: the door + its subject. */
export interface DoorProof {
  door: string;
  providerSub: string;
}

export interface IdentityLinkRow {
  linkId: string;
  accountId: string;
  door: string;
  providerSub: string;
  linkedAt: string;
  linkedBy: string;
  unlinkedAt: string | null;
}

export type LinkDenyReason =
  | "bad_door"
  | "account_not_found"
  | "actor_not_account_binding"
  | "door_identity_claimed"
  | "already_linked"
  | "link_not_active"
  | "last_door_guard";

export type LinkResult =
  | { ok: true; link: IdentityLinkRow }
  | { ok: false; reason: LinkDenyReason };

export type UnlinkResult =
  | { ok: true; unlinked: IdentityLinkRow; replacement: IdentityLinkRow | null }
  | { ok: false; reason: LinkDenyReason };

export interface IdentityLinkDeps {
  /** Seconds-epoch ISO clock; defaults to the wall clock. */
  now?: () => string;
  /** Opaque id mint for link/event rows; defaults to randomUUID. */
  newId?: () => string;
}

function nowOf(deps: IdentityLinkDeps): string {
  return (deps.now ?? (() => new Date().toISOString()))();
}

function idOf(deps: IdentityLinkDeps): string {
  return (deps.newId ?? (() => crypto.randomUUID()))();
}

interface LinkRowRaw {
  link_id: string;
  account_id: string;
  door: string;
  provider_sub: string;
  linked_at: string;
  linked_by: string;
  unlinked_at: string | null;
}

function toRow(raw: LinkRowRaw): IdentityLinkRow {
  return {
    linkId: raw.link_id,
    accountId: raw.account_id,
    door: raw.door,
    providerSub: raw.provider_sub,
    linkedAt: raw.linked_at,
    linkedBy: raw.linked_by,
    unlinkedAt: raw.unlinked_at,
  };
}

function findLinkByDoorIdentity(
  sql: IdentityLinkSql,
  door: string,
  providerSub: string,
): IdentityLinkRow | null {
  const rows = sql
    .exec(
      "SELECT link_id, account_id, door, provider_sub, linked_at, linked_by, unlinked_at FROM identity_link WHERE door = ? AND provider_sub = ?",
      door,
      providerSub,
    )
    .toArray() as unknown as LinkRowRaw[];
  return rows.length === 0 ? null : toRow(rows[0]!);
}

function findActiveLink(
  sql: IdentityLinkSql,
  accountId: string,
  door: string,
): IdentityLinkRow | null {
  const rows = sql
    .exec(
      "SELECT link_id, account_id, door, provider_sub, linked_at, linked_by, unlinked_at FROM identity_link WHERE account_id = ? AND door = ? AND unlinked_at IS NULL",
      accountId,
      door,
    )
    .toArray() as unknown as LinkRowRaw[];
  return rows.length === 0 ? null : toRow(rows[0]!);
}

function countActiveLinks(sql: IdentityLinkSql, accountId: string): number {
  const rows = sql
    .exec(
      "SELECT COUNT(*) AS n FROM identity_link WHERE account_id = ? AND unlinked_at IS NULL",
      accountId,
    )
    .toArray() as { n: number }[];
  return rows[0]!.n;
}

function accountExists(sql: IdentityLinkSql, accountId: string): boolean {
  const rows = sql
    .exec("SELECT account_id FROM account WHERE account_id = ?", accountId)
    .toArray();
  return rows.length > 0;
}

/** The ceremony's actor check: the session's binding must be the
 * account's own ACTIVE binding (a superseded or foreign binding
 * performs no identity-state change). */
function actorIsAccountBinding(
  sql: IdentityLinkSql,
  session: LinkSession,
): boolean {
  const rows = sql
    .exec(
      "SELECT binding_id FROM binding WHERE binding_id = ? AND account_id = ? AND status = 'active'",
      session.bindingId,
      session.accountId,
    )
    .toArray();
  return rows.length > 0;
}

function writeEvent(
  sql: IdentityLinkSql,
  deps: IdentityLinkDeps,
  link: IdentityLinkRow,
  action: "linked" | "unlinked",
  actorBindingId: string,
  at: string,
): void {
  sql.exec(
    "INSERT INTO identity_link_event (event_id, link_id, account_id, door, action, actor_binding_id, at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    idOf(deps),
    link.linkId,
    link.accountId,
    link.door,
    action,
    actorBindingId,
    at,
  );
}

/**
 * Resolve a door identity to its account — the front door's
 * `identity_link` lookup. Only ACTIVE links resolve: an unlinked
 * door stops opening the account immediately (design §4.3).
 */
export function resolveAccountIdByDoor(
  sql: IdentityLinkSql,
  door: string,
  providerSub: string,
): string | null {
  if (!DOOR_SET.has(door)) return null;
  const link = findLinkByDoorIdentity(sql, door, providerSub);
  if (link === null || link.unlinkedAt !== null) return null;
  return link.accountId;
}

/** All of an account's link rows, active and historical, oldest
 * first. History rows are retained while the account lives. */
export function listIdentityLinks(
  sql: IdentityLinkSql,
  accountId: string,
): IdentityLinkRow[] {
  const rows = sql
    .exec(
      "SELECT link_id, account_id, door, provider_sub, linked_at, linked_by, unlinked_at FROM identity_link WHERE account_id = ? ORDER BY linked_at, link_id",
      accountId,
    )
    .toArray() as unknown as LinkRowRaw[];
  return rows.map(toRow);
}

/**
 * Link a new door to the session's account (ceremony in the
 * header). Re-linking a door-identity the account previously
 * unlinked reactivates its retained history row.
 */
export function linkDoor(
  sql: IdentityLinkSql,
  session: LinkSession,
  proof: DoorProof,
  deps: IdentityLinkDeps = {},
): LinkResult {
  return atomically(sql, () => linkDoorWithin(sql, session, proof, deps));
}

function linkDoorWithin(
  sql: IdentityLinkSql,
  session: LinkSession,
  proof: DoorProof,
  deps: IdentityLinkDeps,
): LinkResult {
  if (!DOOR_SET.has(proof.door)) return { ok: false, reason: "bad_door" };
  if (!accountExists(sql, session.accountId))
    return { ok: false, reason: "account_not_found" };
  if (!actorIsAccountBinding(sql, session))
    return { ok: false, reason: "actor_not_account_binding" };

  const existing = findLinkByDoorIdentity(sql, proof.door, proof.providerSub);
  if (existing !== null) {
    if (existing.accountId !== session.accountId)
      return { ok: false, reason: "door_identity_claimed" };
    if (existing.unlinkedAt === null) return { ok: false, reason: "already_linked" };
    // Re-link of the account's own door: reactivate the retained
    // history row (the UNIQUE spans history; no second row exists).
    const at = nowOf(deps);
    sql.exec(
      "UPDATE identity_link SET unlinked_at = NULL, linked_at = ?, linked_by = ? WHERE link_id = ?",
      at,
      session.bindingId,
      existing.linkId,
    );
    const reactivated: IdentityLinkRow = {
      ...existing,
      unlinkedAt: null,
      linkedAt: at,
      linkedBy: session.bindingId,
    };
    writeEvent(sql, deps, reactivated, "linked", session.bindingId, at);
    return { ok: true, link: reactivated };
  }

  const at = nowOf(deps);
  const link: IdentityLinkRow = {
    linkId: idOf(deps),
    accountId: session.accountId,
    door: proof.door,
    providerSub: proof.providerSub,
    linkedAt: at,
    linkedBy: session.bindingId,
    unlinkedAt: null,
  };
  sql.exec(
    "INSERT INTO identity_link (link_id, account_id, door, provider_sub, linked_at, linked_by, unlinked_at) VALUES (?, ?, ?, ?, ?, ?, NULL)",
    link.linkId,
    link.accountId,
    link.door,
    link.providerSub,
    link.linkedAt,
    link.linkedBy,
  );
  writeEvent(sql, deps, link, "linked", session.bindingId, at);
  return { ok: true, link };
}

/**
 * Unlink a door from the session's account. The last-door guard
 * refuses to unlink the account's single active link unless
 * `replacement` (a completed upstream login on a new door) is
 * supplied in the same operation; the replacement is linked first
 * and a refused replacement aborts the unlink untouched.
 */
export function unlinkDoor(
  sql: IdentityLinkSql,
  session: LinkSession,
  door: string,
  deps: IdentityLinkDeps = {},
  replacement?: DoorProof,
): UnlinkResult {
  return atomically(sql, () =>
    unlinkDoorWithin(sql, session, door, deps, replacement),
  );
}

function unlinkDoorWithin(
  sql: IdentityLinkSql,
  session: LinkSession,
  door: string,
  deps: IdentityLinkDeps,
  replacement?: DoorProof,
): UnlinkResult {
  if (!DOOR_SET.has(door)) return { ok: false, reason: "bad_door" };
  if (!accountExists(sql, session.accountId))
    return { ok: false, reason: "account_not_found" };
  if (!actorIsAccountBinding(sql, session))
    return { ok: false, reason: "actor_not_account_binding" };

  const active = findActiveLink(sql, session.accountId, door);
  if (active === null) return { ok: false, reason: "link_not_active" };

  let replacementLink: IdentityLinkRow | null = null;
  if (replacement === undefined) {
    if (countActiveLinks(sql, session.accountId) <= 1)
      return { ok: false, reason: "last_door_guard" };
  } else {
    // A supplied replacement is linked first, whatever the active
    // count: the caller asked for one atomic swap, and a refused
    // replacement aborts the unlink untouched.
    const linked = linkDoor(sql, session, replacement, deps);
    if (!linked.ok) return linked;
    replacementLink = linked.link;
  }

  const at = nowOf(deps);
  sql.exec(
    "UPDATE identity_link SET unlinked_at = ? WHERE link_id = ?",
    at,
    active.linkId,
  );
  const unlinked: IdentityLinkRow = { ...active, unlinkedAt: at };
  writeEvent(sql, deps, unlinked, "unlinked", session.bindingId, at);
  return { ok: true, unlinked, replacement: replacementLink };
}
