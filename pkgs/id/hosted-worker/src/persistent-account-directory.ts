// Durable unique-subject directory operations for #3399. This module is an
// adapter contract only: production binding/provisioning and signup wiring
// remain gated separately (#2285 / #3403).

import {
  createHostedAccountRecord,
  HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS,
  inventoryLegacyHostedAccount,
  migrationShapeError,
  planDirectoryTransition,
  sameDirectoryRecord,
  type DirectoryMigrationEvent,
  type HostedAccountRecord,
  type HostedMigrationPhase,
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

/** A coordinator's hold on one account's migration (#3404). */
export interface MigrationLease {
  owner: string;
  /** Rises every time the lease changes hands: the fencing token. */
  token: number;
  expiresMs: number;
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

const COLUMNS = HOSTED_DIRECTORY_ALLOWLISTED_COLUMNS.join(", ");

const LEGACY_COLUMNS = "subject_key, binding_id, classification";

/** A legacy-inventoried subject cannot sign up beside its legacy entry. */
function legacySubjectRequiresMigration(): FailClosed {
  return new FailClosed(
    409,
    "legacy_subject_requires_migration",
    "subject has a legacy account of unknown jurisdiction; signup cannot relabel it — migration is a separately reviewed procedure",
  );
}

/** A validated subject cannot also be inventoried as legacy. */
function legacySubjectAlreadyRegistered(): FailClosed {
  return new FailClosed(
    409,
    "legacy_subject_already_registered",
    "subject already has a validated account record; it cannot be inventoried as legacy",
  );
}

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
      typeof row.phase !== "string" ||
      typeof row.routing_epoch !== "number" ||
      (row.migration_id !== null && typeof row.migration_id !== "string")
    ) {
      throw new Error("stored directory metadata has the wrong column types");
    }

    const record: HostedAccountRecord = {
      subjectKey: row.subject_key,
      bindingId: parseBindingId(row.binding_id),
      jurisdiction: parseJurisdiction(row.jurisdiction),
      jurisdictionPending: row.jurisdiction_pending === null ? null : parseJurisdiction(row.jurisdiction_pending),
      migrationId: row.migration_id,
      phase: row.phase as HostedMigrationPhase,
      routingEpoch: row.routing_epoch,
    };
    // Each phase has one valid shape (#3404); anything else fails closed.
    if (migrationShapeError(record) !== null) {
      throw new Error("stored directory metadata is outside the account contract");
    }
    return record;
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

  /** Read a validated stable account by its minted routing key. */
  async getByBindingId(bindingId: string): Promise<HostedAccountRecord | undefined> {
    return this.findByBindingId(bindingId);
  }

  /**
   * Read-only routing lookup for #3399: the front door resolves an
   * already-authenticated binding id to its validated stable record.
   * Only reads; never signs up, inventories, provisions, or migrates.
   * Unknown bindings return undefined; corrupt rows fail closed via
   * fromRow (`invalid_directory_record`).
   */
  async findByBindingId(bindingId: string): Promise<HostedAccountRecord | undefined> {
    const validatedBindingId = parseBindingId(bindingId);
    const row = await this.database
      .prepare(`SELECT ${COLUMNS} FROM hosted_accounts WHERE binding_id = ?`)
      .bind(validatedBindingId)
      .first<StoredAccountRow>();
    if (!row) return undefined;
    const record = fromRow(row);
    if (record.bindingId !== validatedBindingId) {
      throw new FailClosed(
        503,
        "invalid_directory_record",
        "stored hosted account binding does not match its lookup key; failing closed",
      );
    }
    return record;
  }

  async signup(input: NewHostedAccount): Promise<HostedAccountRecord> {
    // A legacy-inventoried subject must never gain a second, validated
    // store beside its legacy entry: signup fails closed and the
    // separately reviewed migration procedure owns the way out.
    const legacy = await this.legacyInventory(input.subjectKey);
    if (legacy) throw legacySubjectRequiresMigration();

    // Match the directory contract: a duplicate subject reads its authoritative
    // row and never replaces its jurisdiction with a new signup's value.
    const existing = await this.get(input.subjectKey);
    if (existing) return this.existingSubject(existing, input.bindingId);

    const candidate = createHostedAccountRecord(input);
    let inserted: StoredAccountRow | null;
    try {
      // The legacy check above is advisory: an inventory can land between
      // it and this write. The insert re-checks inside the one statement,
      // so a subject never lands in both tables (#15661).
      inserted = await this.database
        .prepare(
          `INSERT INTO hosted_accounts (${COLUMNS}) SELECT ?, ?, ?, ?, ?, ?, ? ` +
            "WHERE NOT EXISTS (SELECT 1 FROM legacy_hosted_accounts WHERE subject_key = ?) " +
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
          candidate.subjectKey,
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

    // Or a legacy inventory of the same subject landed first.
    if (await this.legacyInventory(candidate.subjectKey)) {
      throw legacySubjectRequiresMigration();
    }

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

  /**
   * Apply one migration step by compare-and-set (#3404, design §7). The
   * caller passes the record it read; the step is planned from it and
   * written only if the stored row still equals it in every routed
   * field, so two coordinators racing on one account cannot both win.
   * A lost race rereads: if the stored row already equals the planned
   * result (a retried step), that row is returned; otherwise the step
   * fails with 409 `directory_cas_conflict`. A missing subject is 404.
   */
  async transition(
    expected: HostedAccountRecord,
    event: DirectoryMigrationEvent,
    lease?: { lease: MigrationLease; nowMs: number },
  ): Promise<HostedAccountRecord> {
    const next = planDirectoryTransition(expected, event);
    // With a lease, the same statement checks the lease is still held:
    // a coordinator that lost it cannot move the record (#3404).
    const leaseClause = lease
      ? " AND EXISTS (SELECT 1 FROM migration_leases WHERE subject_key = ? AND owner = ? AND token = ? AND expires_ms > ?)"
      : "";
    const leaseValues = lease
      ? [expected.subjectKey, lease.lease.owner, lease.lease.token, lease.nowMs]
      : [];
    const updated = await this.database
      .prepare(
        "UPDATE hosted_accounts SET jurisdiction = ?, jurisdiction_pending = ?, migration_id = ?, phase = ?, routing_epoch = ? " +
          "WHERE subject_key = ? AND binding_id = ? AND jurisdiction = ? AND jurisdiction_pending IS ? " +
          "AND migration_id IS ? AND phase = ? AND routing_epoch = ?" +
          leaseClause +
          ` RETURNING ${COLUMNS}`,
      )
      .bind(
        next.jurisdiction,
        next.jurisdictionPending,
        next.migrationId,
        next.phase,
        next.routingEpoch,
        expected.subjectKey,
        expected.bindingId,
        expected.jurisdiction,
        expected.jurisdictionPending,
        expected.migrationId,
        expected.phase,
        expected.routingEpoch,
        ...leaseValues,
      )
      .first<StoredAccountRow>();
    if (updated) return fromRow(updated);

    const current = await this.get(expected.subjectKey);
    if (!current) {
      throw new FailClosed(404, "directory_account_missing", "no hosted account record for this subject");
    }
    if (sameDirectoryRecord(current, next)) return current;
    if (lease && !(await this.leaseHeld(expected.subjectKey, lease.lease, lease.nowMs))) {
      throw new FailClosed(409, "lease_lost", "this coordinator no longer holds the migration lease");
    }
    throw new FailClosed(
      409,
      "directory_cas_conflict",
      "the account record changed since it was read; reread it and retry the step",
    );
  }

  /**
   * Take or renew the account's migration lease (#3404) until
   * `nowMs + ttlMs`. The holder renews and keeps its token; anyone else
   * takes it only once it has expired, and the token rises. A lease
   * held by another coordinator gives 409 `lease_held`.
   */
  async acquireLease(subjectKey: string, owner: string, nowMs: number, ttlMs: number): Promise<MigrationLease> {
    if (typeof owner !== "string" || owner.trim() === "" || owner.length > 128) {
      throw new FailClosed(400, "invalid_lease_owner", "a lease owner is 1 to 128 characters");
    }
    if (!Number.isSafeInteger(nowMs) || !Number.isSafeInteger(ttlMs) || ttlMs <= 0) {
      throw new FailClosed(400, "invalid_lease_ttl", "lease times are positive integer milliseconds");
    }
    const row = await this.database
      .prepare(
        "INSERT INTO migration_leases (subject_key, owner, token, expires_ms) VALUES (?, ?, 1, ?) " +
          "ON CONFLICT(subject_key) DO UPDATE SET " +
          "token = CASE WHEN migration_leases.owner = excluded.owner THEN migration_leases.token ELSE migration_leases.token + 1 END, " +
          "owner = excluded.owner, expires_ms = excluded.expires_ms " +
          "WHERE migration_leases.owner = excluded.owner OR migration_leases.expires_ms <= ? " +
          "RETURNING owner, token, expires_ms",
      )
      .bind(subjectKey, owner, nowMs + ttlMs, nowMs)
      .first<{ owner: string; token: number; expires_ms: number }>();
    if (!row) {
      throw new FailClosed(409, "lease_held", "another coordinator holds this account's migration lease");
    }
    return { owner: row.owner, token: Number(row.token), expiresMs: Number(row.expires_ms) };
  }

  /** Give the lease up early; a lease already taken over is left alone. */
  async releaseLease(subjectKey: string, lease: MigrationLease): Promise<void> {
    await this.database
      .prepare(
        "UPDATE migration_leases SET expires_ms = 0 WHERE subject_key = ? AND owner = ? AND token = ? RETURNING token",
      )
      .bind(subjectKey, lease.owner, lease.token)
      .first();
  }

  /** Whether `lease` is still the account's live lease at `nowMs`. */
  async leaseHeld(subjectKey: string, lease: MigrationLease, nowMs: number): Promise<boolean> {
    const row = await this.database
      .prepare("SELECT token FROM migration_leases WHERE subject_key = ? AND owner = ? AND token = ? AND expires_ms > ?")
      .bind(subjectKey, lease.owner, lease.token, nowMs)
      .first();
    return row !== null;
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
    if (await this.get(entry.subjectKey)) throw legacySubjectAlreadyRegistered();

    let inserted: StoredLegacyAccountRow | null;
    try {
      // As in signup: the validated-row check above is advisory, so the
      // insert re-checks in the same statement (#15661).
      inserted = await this.database
        .prepare(
          `INSERT INTO legacy_hosted_accounts (${LEGACY_COLUMNS}) SELECT ?, ?, ? ` +
            "WHERE NOT EXISTS (SELECT 1 FROM hosted_accounts WHERE subject_key = ?) " +
            "ON CONFLICT(subject_key) DO NOTHING " +
            `RETURNING ${LEGACY_COLUMNS}`,
        )
        .bind(entry.subjectKey, entry.bindingId, entry.classification, entry.subjectKey)
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

    // Or a signup of the same subject landed first.
    if (await this.get(entry.subjectKey)) throw legacySubjectAlreadyRegistered();

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
   * Read-only routing lookup for #3399: the front door distinguishes a
   * legacy-only binding from a truly unknown one. Only reads; corrupt
   * rows fail closed via fromLegacyRow
   * (`invalid_legacy_inventory_record`).
   */
  async findLegacyByBindingId(bindingId: string): Promise<LegacyHostedAccount | undefined> {
    const validatedBindingId = parseBindingId(bindingId);
    const row = await this.database
      .prepare(`SELECT ${LEGACY_COLUMNS} FROM legacy_hosted_accounts WHERE binding_id = ?`)
      .bind(validatedBindingId)
      .first<StoredLegacyAccountRow>();
    if (!row) return undefined;
    const record = fromLegacyRow(row);
    if (record.bindingId !== validatedBindingId) {
      throw new FailClosed(
        503,
        "invalid_legacy_inventory_record",
        "stored legacy binding does not match its lookup key; failing closed",
      );
    }
    return record;
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
