// Durable unique-subject directory operations for #3399. This module is an
// adapter contract only: production binding/provisioning and signup wiring
// remain gated separately (#2285 / #3403).

import {
  createHostedAccountRecord,
  type HostedAccountRecord,
  type NewHostedAccount,
} from "./account-directory.ts";
import { FailClosed, parseBindingId, parseJurisdiction } from "./routing.ts";

/** Small structural subset shared by Cloudflare D1 and the SQLite test adapter. */
export interface HostedDirectoryStatement {
  bind(...values: Array<string | number | null>): HostedDirectoryStatement;
  first<T extends Record<string, unknown>>(): Promise<T | null>;
}

export interface HostedDirectoryDatabase {
  prepare(query: string): HostedDirectoryStatement;
}

type StoredAccountRow = Record<string, unknown> & {
  subject_key: string;
  binding_id: string;
  jurisdiction: string;
  jurisdiction_pending: string | null;
  migration_id: string | null;
  phase: string;
  routing_epoch: number;
};

const COLUMNS =
  "subject_key, binding_id, jurisdiction, jurisdiction_pending, migration_id, phase, routing_epoch";

function fromRow(row: StoredAccountRow): HostedAccountRecord {
  if (
    typeof row.subject_key !== "string" ||
    row.subject_key.trim() === "" ||
    typeof row.binding_id !== "string" ||
    row.jurisdiction_pending !== null ||
    row.migration_id !== null ||
    row.phase !== "stable" ||
    row.routing_epoch !== 0
  ) {
    throw new FailClosed(
      503,
      "invalid_directory_record",
      "stored hosted account record is invalid; failing closed",
    );
  }
  return {
    subjectKey: row.subject_key,
    bindingId: parseBindingId(row.binding_id),
    jurisdiction: parseJurisdiction(row.jurisdiction),
    jurisdictionPending: null,
    migrationId: null,
    phase: "stable",
    routingEpoch: 0,
  };
}

/**
 * D1-backed signup record operations. The unique indexes are the serialization
 * point: concurrent insertions cannot replace an existing subject or binding.
 */
export class PersistentHostedAccountDirectory {
  constructor(private readonly database: HostedDirectoryDatabase) {}

  async get(subjectKey: string): Promise<HostedAccountRecord | undefined> {
    const row = await this.database
      .prepare(`SELECT ${COLUMNS} FROM hosted_accounts WHERE subject_key = ?`)
      .bind(subjectKey)
      .first<StoredAccountRow>();
    return row ? fromRow(row) : undefined;
  }

  async signup(input: NewHostedAccount): Promise<HostedAccountRecord> {
    // Match the directory contract: a duplicate subject reads its authoritative
    // row and never replaces its jurisdiction with a new signup's value.
    const existing = await this.get(input.subjectKey);
    if (existing) return this.existingSubject(existing, input.bindingId);

    const candidate = createHostedAccountRecord(input);
    const inserted = await this.database
      .prepare(
        `INSERT INTO hosted_accounts (${COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?) ` +
          "ON CONFLICT DO NOTHING " +
          `RETURNING ${COLUMNS}`,
      )
      .bind(
        candidate.subjectKey,
        candidate.bindingId,
        candidate.jurisdiction,
        candidate.jurisdictionPending,
        candidate.migrationId,
        candidate.phase,
        candidate.routingEpoch,
      )
      .first<StoredAccountRow>();
    if (inserted) return fromRow(inserted);

    // A competing signup may have won after the initial read. Re-read the
    // authoritative subject row; never fall back to the candidate snapshot.
    const winner = await this.get(candidate.subjectKey);
    if (winner) return this.existingSubject(winner, candidate.bindingId);

    // No row for this subject means another subject owns the unique binding.
    throw new FailClosed(
      409,
      "binding_conflict",
      "binding id is already assigned to another subject",
    );
  }

  private existingSubject(
    row: HostedAccountRecord,
    bindingId: string,
  ): HostedAccountRecord {
    if (row.bindingId !== bindingId) {
      throw new FailClosed(
        409,
        "duplicate_subject_conflict",
        "subject key is already bound to a different binding id; failing closed",
      );
    }
    return row;
  }
}
