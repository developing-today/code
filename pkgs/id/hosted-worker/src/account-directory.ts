// Initial authoritative account-directory record for jurisdiction-aware
// hosted signup (#3399; design doc/2026-11-01_jurisdiction_aware_user_store_design.md).
// This module constructs metadata only. Persistence, unique-subject
// transactions, and migration transitions are separate slices; callers must
// not derive or contact a user object until this validation succeeds.

import { FailClosed, parseBindingId, parseJurisdiction, type Jurisdiction } from "./routing.ts";

export type HostedMigrationPhase = "stable";

/** Durable routing metadata required before a hosted user object is derived. */
export interface HostedAccountRecord {
  /** Pseudonymous provider subject key; never an email address. */
  subjectKey: string;
  /** Stable binding ID minted by the hosted binding ceremony. */
  bindingId: string;
  /** The active, validated jurisdiction. */
  jurisdiction: Jurisdiction;
  /** Null until a migration transition is explicitly started. */
  jurisdictionPending: null;
  /** Null until a migration transition is explicitly started. */
  migrationId: null;
  /** New accounts start in the only non-migration phase. */
  phase: HostedMigrationPhase;
  /** Incremented by guarded migration transitions, never inferred. */
  routingEpoch: 0;
}

export interface NewHostedAccount {
  subjectKey: string;
  bindingId: string;
  jurisdiction?: string | null;
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
