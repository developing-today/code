// Durable unique-subject directory operations for #3399. This module is an
// adapter contract only: production binding/provisioning and signup wiring
// remain gated separately (#2285 / #3403).

import {
  createHostedAccountRecord,
  inventoryLegacyHostedAccount,
  type HostedAccountRecord,
  type LegacyHostedAccount,
  type LegacyHostedAccountInput,
  type NewHostedAccount,
} from "./account-directory.ts";
import { FailClosed, parseBindingId, parseJurisdiction } from "./routing.ts";

/** Small structural subset shared by Cloudflare D1 and the SQLite test adapter. */
export interface HostedDirectoryStatement {
  bind(...values: Array<string | number | null>): HostedDirectoryStatement;
  first<T extends Record<string, unknown>>(): Promise<T | null>;
  /** All result rows, in the statement's own order. D1 returns rows under
   * `results`; its adapter unwraps that here so the directory sees one shape. */
  all<T extends Record<string, unknown>>(): Promise<T[]>;
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

const LEGACY_COLUMNS = "subject_key, binding_id, classification";

type StoredLegacyAccountRow = Record<string, unknown> & {
  subject_key: string;
  binding_id: string;
  classification: string;
};

function fromLegacyRow(row: StoredLegacyAccountRow): LegacyHostedAccount {
  try {
    if (
      typeof row.subject_key !== "string" ||
      row.subject_key.trim() === "" ||
      typeof row.classification !== "string" ||
      row.classification !== "legacy-unrestricted"
    ) {
      throw new Error("stored legacy inventory entry is outside the legacy contract");
    }
    return {
      subjectKey: row.subject_key,
      bindingId: parseBindingId(row.binding_id),
      jurisdiction: null,
      classification: "legacy-unrestricted",
    };
  } catch {
    throw new FailClosed(
      503,
      "invalid_legacy_inventory_record",
      "stored legacy inventory entry is invalid; failing closed",
    );
  }
}

function fromRow(row: StoredAccountRow): HostedAccountRecord {
  try {
    if (
      typeof row.subject_key !== "string" ||
      row.subject_key.trim() === "" ||
      typeof row.binding_id !== "string" ||
      row.jurisdiction_pending !== null ||
      row.migration_id !== null ||
      row.phase !== "stable" ||
      row.routing_epoch !== 0
    ) {
      throw new Error("stored directory metadata is outside the stable account contract");
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
  } catch {
    throw new FailClosed(
      503,
      "invalid_directory_record",
      "stored hosted account record is invalid; failing closed",
    );
  }
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

  private async getByBindingId(bindingId: string): Promise<HostedAccountRecord | undefined> {
    const row = await this.database
      .prepare(`SELECT ${COLUMNS} FROM hosted_accounts WHERE binding_id = ?`)
      .bind(bindingId)
      .first<StoredAccountRow>();
    return row ? fromRow(row) : undefined;
  }

  async signup(input: NewHostedAccount): Promise<HostedAccountRecord> {
    // A legacy-inventoried subject must never gain a second, validated
    // store beside its legacy entry: signup fails closed and the
    // separately reviewed migration procedure owns the way out.
    const legacy = await this.legacyInventory(input.subjectKey);
    if (legacy) {
      throw new FailClosed(
        409,
        "legacy_subject_requires_migration",
        "subject has a legacy account of unknown jurisdiction; signup cannot relabel it — migration is a separately reviewed procedure",
      );
    }

    // Match the directory contract: a duplicate subject reads its authoritative
    // row and never replaces its jurisdiction with a new signup's value.
    const existing = await this.get(input.subjectKey);
    if (existing) return this.existingSubject(existing, input.bindingId);

    const candidate = createHostedAccountRecord(input);
    let inserted: StoredAccountRow | null;
    try {
      inserted = await this.database
        .prepare(
          `INSERT INTO hosted_accounts (${COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?) ` +
            "ON CONFLICT(subject_key) DO NOTHING " +
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
    } catch (error) {
      const winner = await this.get(candidate.subjectKey);
      if (winner) return this.existingSubject(winner, candidate.bindingId);

      const bindingOwner = await this.getByBindingId(candidate.bindingId);
      if (bindingOwner) {
        throw new FailClosed(
          409,
          "binding_conflict",
          "binding id is already assigned to another subject",
        );
      }
      throw error;
    }
    if (inserted) return fromRow(inserted);

    // A competing signup may have won after the initial read. Re-read the
    // authoritative subject row; never fall back to the candidate snapshot.
    const winner = await this.get(candidate.subjectKey);
    if (winner) return this.existingSubject(winner, candidate.bindingId);

    const bindingOwner = await this.getByBindingId(candidate.bindingId);
    if (bindingOwner) {
      throw new FailClosed(
        409,
        "binding_conflict",
        "binding id is already assigned to another subject",
      );
    }

    throw new FailClosed(
      503,
      "directory_write_conflict",
      "directory write did not produce a row or an authoritative conflict; failing closed",
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

  /**
   * Record a pre-existing account in the durable legacy inventory.
   * Validation runs before any read or write, so a relabel attempt
   * fails closed even on a repeat call. An already-validated subject
   * cannot be inventoried as legacy; its record stays authoritative.
   */
  async inventoryLegacy(input: LegacyHostedAccountInput): Promise<LegacyHostedAccount> {
    const entry = inventoryLegacyHostedAccount(input);
    if (await this.get(entry.subjectKey)) {
      throw new FailClosed(
        409,
        "legacy_subject_already_registered",
        "subject already has a validated account record; it cannot be inventoried as legacy",
      );
    }

    let inserted: StoredLegacyAccountRow | null;
    try {
      inserted = await this.database
        .prepare(
          `INSERT INTO legacy_hosted_accounts (${LEGACY_COLUMNS}) VALUES (?, ?, ?) ` +
            "ON CONFLICT(subject_key) DO NOTHING " +
            `RETURNING ${LEGACY_COLUMNS}`,
        )
        .bind(entry.subjectKey, entry.bindingId, entry.classification)
        .first<StoredLegacyAccountRow>();
    } catch (error) {
      const winner = await this.legacyInventory(entry.subjectKey);
      if (winner) return this.existingLegacy(winner, entry.bindingId);
      throw error;
    }
    if (inserted) return fromLegacyRow(inserted);

    // A competing inventory may have won after the validated-row read.
    // Re-read the authoritative entry; a relabel attempt still fails
    // closed against the winner's binding.
    const winner = await this.legacyInventory(entry.subjectKey);
    if (winner) return this.existingLegacy(winner, entry.bindingId);

    throw new FailClosed(
      503,
      "legacy_inventory_write_conflict",
      "legacy inventory write did not produce an entry or an authoritative conflict; failing closed",
    );
  }

  /** The durable legacy inventory entry for a subject, if one was recorded. */
  async legacyInventory(subjectKey: string): Promise<LegacyHostedAccount | undefined> {
    const row = await this.database
      .prepare(`SELECT ${LEGACY_COLUMNS} FROM legacy_hosted_accounts WHERE subject_key = ?`)
      .bind(subjectKey)
      .first<StoredLegacyAccountRow>();
    return row ? fromLegacyRow(row) : undefined;
  }

  /**
   * Every inventoried legacy account, ordered by subject key. The
   * packet's "legacy inventory lists unrestricted objects" read:
   * entries carry no jurisdiction, so a listing can never relabel them.
   */
  async listLegacyInventory(): Promise<LegacyHostedAccount[]> {
    const rows = await this.database
      .prepare(`SELECT ${LEGACY_COLUMNS} FROM legacy_hosted_accounts ORDER BY subject_key`)
      .all<StoredLegacyAccountRow>();
    return rows.map(fromLegacyRow);
  }

  private existingLegacy(
    row: LegacyHostedAccount,
    bindingId: string,
  ): LegacyHostedAccount {
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
