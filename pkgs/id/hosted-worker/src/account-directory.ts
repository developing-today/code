// Initial authoritative account-directory record for jurisdiction-aware
// hosted signup (#3399; design doc/2026-11-01_jurisdiction_aware_user_store_design.md).
// This module constructs metadata and plans migration steps (#3404);
// persistence applies them by compare-and-set elsewhere; callers must
// not derive or contact a user object until this validation succeeds.
//
// The module also keeps the legacy inventory: pre-existing accounts whose
// jurisdiction is unknown are recorded as legacy entries, never as
// validated records. Inventorying asserts nothing about residency — a
// legacy account is migrated by a separately reviewed procedure, not
// relabelled `us` (or any other jurisdiction) or silently backfilled.

import { isMigrationId } from "./migration-fence.ts";
import {
  FailClosed,
  parseBindingId,
  parseJurisdiction,
  SUPPORTED_JURISDICTIONS,
  type Jurisdiction,
} from "./routing.ts";

/**
 * Directory migration phases (#3404, design §7). `stable` is the only
 * non-migration phase. `pending`, `frozen` and `verified` still route
 * to the source; `switched` names the target as authoritative while it
 * is not yet serving, so the front door answers it with a retryable
 * refusal. Completion returns the record to `stable` at the target;
 * abort returns it to `stable` at the source.
 */
export const HOSTED_MIGRATION_PHASES = ["stable", "pending", "frozen", "verified", "switched"] as const;
export type HostedMigrationPhase = (typeof HOSTED_MIGRATION_PHASES)[number];

/**
 * #3401 data-minimal allowlist: the only columns the shared directory
 * may hold — pseudonymous subject/binding identifiers plus minimal
 * jurisdiction and migration metadata. No graph content, journals,
 * blob bytes, or private profile data. The D1 migration, the query
 * layer (`COLUMNS` in persistent-account-directory.ts), and the
 * hosted-directory-allowlist suite all pin this one list, so widening
 * the shared residency exception needs its own reviewed slice.
 */
export const HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS = [
  "subject_key",
  "binding_id",
  "jurisdiction",
  "jurisdiction_pending",
  "migration_id",
  "phase",
  "routing_epoch",
] as const;

/** Durable routing metadata required before a hosted user object is derived. */
export interface HostedAccountRecord {
  /** Pseudonymous provider subject key; never an email address. */
  subjectKey: string;
  /** Stable binding ID minted by the hosted binding ceremony. */
  bindingId: string;
  /** The active, validated jurisdiction. */
  jurisdiction: Jurisdiction;
  /** The other jurisdiction of a running migration: its target while
   * pending, frozen or verified, and its source once switched (so a
   * resumed coordinator can still seal the source). Null in `stable`. */
  jurisdictionPending: Jurisdiction | null;
  /** The running migration's id; null only in `stable`. */
  migrationId: string | null;
  /** New accounts start in `stable`; only guarded transitions move it. */
  phase: HostedMigrationPhase;
  /** Incremented by guarded migration transitions, never inferred. */
  routingEpoch: number;
}

export interface NewHostedAccount {
  subjectKey: string;
  bindingId: string;
  jurisdiction?: string | null;
}

/**
 * Inventory entry for a pre-existing hosted account whose jurisdiction
 * is unknown (created before jurisdiction-aware signup, or derived
 * through an unrestricted namespace). It is deliberately NOT a
 * `HostedAccountRecord`: it carries no jurisdiction, no routing epoch,
 * and no migration state, so nothing in the routing path can consume it
 * as a validated record. The only way out of the inventory is the
 * separately reviewed migration procedure; this module provides no
 * conversion from a legacy entry to a record.
 */
export interface LegacyHostedAccount {
  /** Pseudonymous provider subject key; never an email address. */
  subjectKey: string;
  /** Stable binding ID minted by the hosted binding ceremony. */
  bindingId: string;
  /** Always null: a legacy account's jurisdiction is unknown, and the
   * inventory records that fact instead of asserting a residency. */
  jurisdiction: null;
  /** Why this account is in the inventory rather than the directory. */
  classification: "legacy-unrestricted";
}

export interface LegacyHostedAccountInput {
  subjectKey: string;
  bindingId: string;
  /**
   * Must be absent, null, or empty. A legacy account by definition has
   * no verified jurisdiction; supplying one here would relabel the
   * account at inventory time, which the design forbids — the value is
   * rejected, never stored.
   */
  jurisdiction?: string | null;
}

/**
 * Record a pre-existing account in the legacy inventory. Validates the
 * identity inputs exactly like account creation, but asserts no
 * jurisdiction: any non-empty jurisdiction value fails closed, because
 * inventory cannot prove residency and must not relabel the account.
 */
export function inventoryLegacyHostedAccount(input: LegacyHostedAccountInput): LegacyHostedAccount {
  if (typeof input.subjectKey !== "string" || input.subjectKey.trim() === "") {
    throw new FailClosed(400, "invalid_subject", "legacy account requires a non-empty pseudonymous subject key");
  }

  const bindingId = parseBindingId(input.bindingId);

  if (input.jurisdiction !== undefined && input.jurisdiction !== null && input.jurisdiction !== "") {
    throw new FailClosed(
      403,
      "legacy_jurisdiction_unverifiable",
      "legacy account jurisdiction is unknown; inventory records it as unknown instead of relabelling the account",
    );
  }

  return {
    subjectKey: input.subjectKey,
    bindingId,
    jurisdiction: null,
    classification: "legacy-unrestricted",
  };
}

/**
 * Profile fields a normal update may carry. Only `jurisdiction` is
 * recognised here, and only to guard it: the design
 * (doc/2026-11-01_jurisdiction_aware_user_store_design.md §1–4) says
 * normal profile updates cannot change `jurisdiction`, even before
 * first DO creation — only the migration state machine may change it.
 * Any other profile content lives outside this record.
 */
export interface HostedAccountProfilePatch {
  jurisdiction?: string | null;
}

/**
 * Apply a normal profile update to a hosted account record. Fails
 * closed (throws `FailClosed`) when the patch attempts to change
 * `jurisdiction` to a different value. A patch with the same
 * jurisdiction, no jurisdiction, or a null jurisdiction returns the
 * record with `jurisdiction`, `jurisdictionPending`, `migrationId`,
 * `phase`, and `routingEpoch` unchanged — only the migration state
 * machine may change those. The input record is never mutated.
 */
export function updateHostedAccountRecord(
  record: HostedAccountRecord,
  patch: HostedAccountProfilePatch,
): HostedAccountRecord {
  if (patch.jurisdiction !== undefined && patch.jurisdiction !== null && patch.jurisdiction !== record.jurisdiction) {
    throw new FailClosed(
      403,
      "jurisdiction_immutable",
      "profile updates cannot change hosted jurisdiction; only the migration state machine may change it",
    );
  }

  return { ...record };
}

/**
 * In-memory hosted account directory keyed by the unique pseudonymous
 * subject key (design doc/2026-11-01_jurisdiction_aware_user_store_design.md:
 * "Duplicate signup reads the existing row; it never overwrites it or
 * resets the policy").
 *
 * The directory also holds the legacy inventory (see
 * `inventoryLegacyHostedAccount`). A subject lives in at most one of
 * the two maps: signup for an inventoried legacy subject fails closed
 * rather than silently creating a second, validated store alongside
 * the legacy one, and a validated subject cannot be inventoried as
 * legacy afterwards.
 *
 * This is the contract the persistent unique-subject transaction slice
 * will implement against durable storage; it holds no production state
 * and enables no signup or migration path on its own.
 */
export class HostedAccountDirectory {
  private readonly rows = new Map<string, HostedAccountRecord>();
  private readonly legacyRows = new Map<string, LegacyHostedAccount>();

  get size(): number {
    return this.rows.size;
  }

  get legacySize(): number {
    return this.legacyRows.size;
  }

  /** The legacy inventory entry for a subject, if one was recorded. */
  legacyInventory(subjectKey: string): LegacyHostedAccount | undefined {
    return this.legacyRows.get(subjectKey);
  }

  /**
   * Record a pre-existing account in the legacy inventory. Repeating
   * the inventory for the same subject and binding returns the
   * preserved entry; a conflicting binding fails closed, exactly like
   * duplicate signup. Inventorying a subject that already has a
   * validated record also fails closed — the record stays authoritative.
   */
  inventoryLegacy(input: LegacyHostedAccountInput): LegacyHostedAccount {
    // Validate first, including the no-asserted-jurisdiction rule, so a
    // relabel attempt fails closed even on a repeat inventory call.
    const entry = inventoryLegacyHostedAccount(input);
    if (this.rows.has(entry.subjectKey)) {
      throw new FailClosed(
        409,
        "legacy_subject_already_registered",
        "subject already has a validated account record; it cannot be inventoried as legacy",
      );
    }
    const existing = this.legacyRows.get(entry.subjectKey);
    if (existing !== undefined) {
      if (existing.bindingId !== entry.bindingId) {
        throw new FailClosed(
          409,
          "duplicate_subject_conflict",
          "subject key is already bound to a different binding id; failing closed",
        );
      }
      return existing;
    }
    this.legacyRows.set(entry.subjectKey, entry);
    return entry;
  }

  signup(input: NewHostedAccount): HostedAccountRecord {
    if (this.legacyRows.has(input.subjectKey)) {
      throw new FailClosed(
        409,
        "legacy_subject_requires_migration",
        "subject has a legacy account of unknown jurisdiction; signup cannot relabel it — migration is a separately reviewed procedure",
      );
    }
    const existing = this.rows.get(input.subjectKey);
    if (existing !== undefined) {
      if (existing.bindingId !== input.bindingId) {
        throw new FailClosed(
          409,
          "duplicate_subject_conflict",
          "subject key is already bound to a different binding id; failing closed",
        );
      }
      return existing;
    }
    const record = createHostedAccountRecord(input);
    this.rows.set(record.subjectKey, record);
    return record;
  }
}

/**
 * Validate the identity and explicit jurisdiction before constructing the
 * first directory row. There is deliberately no policy default: missing or
 * unsupported jurisdiction cannot create metadata that could later route to
 * an empty or unrestricted object.
 */
export function createHostedAccountRecord(input: NewHostedAccount): HostedAccountRecord {
  if (typeof input.subjectKey !== "string" || input.subjectKey.trim() === "") {
    throw new FailClosed(400, "invalid_subject", "hosted account requires a non-empty pseudonymous subject key");
  }

  const bindingId = parseBindingId(input.bindingId);
  const jurisdiction = parseJurisdiction(input.jurisdiction ?? null);

  return {
    subjectKey: input.subjectKey,
    bindingId,
    jurisdiction,
    jurisdictionPending: null,
    migrationId: null,
    phase: "stable",
    routingEpoch: 0,
  };
}

/**
 * Validate a directory record's migration shape (#3404, design §7):
 * - stable: no pending target, no migration id, epoch >= 0;
 * - pending/frozen/verified: a pending target different from the
 *   active jurisdiction, a migration id, epoch >= 1;
 * - switched: the target is now the active jurisdiction and the
 *   pending field names the source, a migration id, epoch >= 2.
 * Returns an error description, or null when the shape holds.
 */
export function migrationShapeError(record: HostedAccountRecord): string | null {
  if (!(HOSTED_MIGRATION_PHASES as readonly string[]).includes(record.phase)) return "unknown phase";
  if (!Number.isSafeInteger(record.routingEpoch) || record.routingEpoch < 0) return "routing epoch out of range";
  if (!(SUPPORTED_JURISDICTIONS as readonly string[]).includes(record.jurisdiction)) return "unsupported jurisdiction";
  switch (record.phase) {
    case "stable":
      if (record.jurisdictionPending !== null || record.migrationId !== null)
        return "stable record carries migration state";
      return null;
    case "pending":
    case "frozen":
    case "verified":
    case "switched":
      if (
        record.jurisdictionPending === null ||
        !(SUPPORTED_JURISDICTIONS as readonly string[]).includes(record.jurisdictionPending)
      ) {
        return "migrating record lacks its other jurisdiction";
      }
      if (record.jurisdictionPending === record.jurisdiction) return "pending jurisdiction equals the active one";
      if (!isMigrationId(record.migrationId)) return "migrating record lacks a migration id";
      if (record.routingEpoch < (record.phase === "switched" ? 2 : 1)) return "migrating record's epoch is too low";
      return null;
  }
}

/** A directory migration step, applied by compare-and-set (#3404). */
export type DirectoryMigrationEvent =
  | { type: "begin"; migrationId: string; targetJurisdiction: string }
  | { type: "freeze-acknowledged"; migrationId: string }
  | { type: "snapshot-verified"; migrationId: string }
  | { type: "switch"; migrationId: string }
  | { type: "complete"; migrationId: string }
  | { type: "abort"; migrationId: string };

function refuse(message: string): never {
  throw new FailClosed(409, "directory_transition_refused", message);
}

function nextEpoch(epoch: number): number {
  if (epoch >= Number.MAX_SAFE_INTEGER) refuse("routing epoch cannot advance safely");
  return epoch + 1;
}

/**
 * Plan one migration step from an expected directory record. Pure: the
 * persistent directory applies the result with a compare-and-set on the
 * expected record. Epochs follow the migration-control state machine:
 * begin and switch advance the epoch, abort advances it again so a
 * stale router at the pending epoch cannot resurrect the attempt, and
 * freeze, verify and complete keep it. Every non-begin step must name
 * the running migration id.
 */
export function planDirectoryTransition(
  record: HostedAccountRecord,
  event: DirectoryMigrationEvent,
): HostedAccountRecord {
  const shape = migrationShapeError(record);
  if (shape !== null) throw new FailClosed(503, "invalid_directory_record", `directory record is invalid: ${shape}`);
  if (!isMigrationId(event.migrationId)) {
    throw new FailClosed(400, "invalid_migration_id", "migration_id must be 1 to 128 of A-Z a-z 0-9 . _ : -");
  }
  if (event.type === "begin") {
    if (record.phase !== "stable") refuse(`cannot begin a migration during ${record.phase}`);
    const target = parseJurisdiction(event.targetJurisdiction);
    if (target === record.jurisdiction) refuse("target jurisdiction must differ from the active one");
    return {
      ...record,
      jurisdictionPending: target,
      migrationId: event.migrationId,
      phase: "pending",
      routingEpoch: nextEpoch(record.routingEpoch),
    };
  }
  if (record.phase === "stable" || record.migrationId !== event.migrationId) {
    refuse(`no running migration ${event.migrationId} on this account`);
  }
  switch (event.type) {
    case "freeze-acknowledged":
      if (record.phase !== "pending") refuse(`cannot acknowledge the freeze during ${record.phase}`);
      return { ...record, phase: "frozen" };
    case "snapshot-verified":
      if (record.phase !== "frozen") refuse(`cannot verify the snapshot during ${record.phase}`);
      return { ...record, phase: "verified" };
    case "switch":
      if (record.phase !== "verified" || record.jurisdictionPending === null) {
        refuse(`cannot switch during ${record.phase}`);
      }
      return {
        ...record,
        jurisdiction: record.jurisdictionPending,
        jurisdictionPending: record.jurisdiction,
        phase: "switched",
        routingEpoch: nextEpoch(record.routingEpoch),
      };
    case "complete":
      if (record.phase !== "switched") refuse(`cannot complete during ${record.phase}`);
      return { ...record, jurisdictionPending: null, migrationId: null, phase: "stable" };
    case "abort":
      if (record.phase === "switched") refuse("cannot abort after the switch");
      return {
        ...record,
        jurisdictionPending: null,
        migrationId: null,
        phase: "stable",
        routingEpoch: nextEpoch(record.routingEpoch),
      };
  }
}

/** Exact equality of two directory records' routed fields. */
export function sameDirectoryRecord(a: HostedAccountRecord, b: HostedAccountRecord): boolean {
  return (
    a.subjectKey === b.subjectKey &&
    a.bindingId === b.bindingId &&
    a.jurisdiction === b.jurisdiction &&
    a.jurisdictionPending === b.jurisdictionPending &&
    a.migrationId === b.migrationId &&
    a.phase === b.phase &&
    a.routingEpoch === b.routingEpoch
  );
}
