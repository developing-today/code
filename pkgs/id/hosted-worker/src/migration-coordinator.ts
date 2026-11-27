// The jurisdiction migration coordinator (#3404 slice 4).
//
// Drives one account from its source jurisdiction to a target:
//
//   stable --begin--> pending --freeze--> frozen --snapshot, copy
//   blobs, import, check--> verified --switch--> switched --activate
//   target, seal source--> stable at the target.
//
// Every directory step is a compare-and-set that also checks the
// coordinator's lease, and every user-object step names the routing
// epoch, so a coordinator that lost its lease or a stale retry is
// refused by both the directory and the objects themselves. The
// coordinator keeps no state of its own: it reads the directory record
// and resumes from whatever phase it finds, so a crash, a lost
// response or a takeover by another coordinator is handled by running
// it again. Abort is the reverse before the switch; after the switch
// the only way on is forward (or a fresh migration back).

import type { HostedAccountRecord } from "./account-directory.ts";
import { HDR_MIGRATION } from "./migration-fence.ts";
import type { MigrationSnapshot, StagedTarget } from "./migration-snapshot.ts";
import type { MigrationLease, PersistentHostedAccountDirectory } from "./persistent-account-directory.ts";
import {
  FailClosed,
  blobsBindingFor,
  parseJurisdiction,
  userStub,
  type BlobsEnv,
  type Jurisdiction,
  type ObjectNamespaceLike,
} from "./routing.ts";
import { HDR_BINDING, HDR_CSE, HDR_JURISDICTION, HDR_ROUTED, blobObjectKey } from "./user-do.ts";

/** What the coordinator needs from the world; production wiring is
 * `workerMigrationPorts`, tests pass fakes. */
export interface MigrationPorts {
  directory: PersistentHostedAccountDirectory;
  /** Call a migration route on the account's user object in one
   * jurisdiction. `cse` binds a fresh object's encryption choice. */
  control(
    record: { bindingId: string },
    jurisdiction: Jurisdiction,
    method: "GET" | "POST",
    path: string,
    body?: unknown,
    cse?: boolean,
  ): Promise<Response>;
  /** Copy one blob's bytes between the jurisdictions' buckets.
   * Idempotent: copying a blob already there is fine. */
  copyBlob(bindingId: string, hash: string, cse: boolean, from: Jurisdiction, to: Jurisdiction): Promise<void>;
  now(): number;
}

export interface MigrationRun {
  subjectKey: string;
  migrationId: string;
  targetJurisdiction: string;
  /** This coordinator's lease owner name. */
  owner: string;
  leaseTtlMs: number;
}

/** Most phase steps one run takes; the happy path needs six. */
const MAX_STEPS = 16;

async function expectOk(response: Response, step: string, tolerate: string[] = []): Promise<Record<string, unknown>> {
  const body = (await response.json().catch(() => ({}))) as Record<string, unknown>;
  if (response.ok) return body;
  if (typeof body.error === "string" && tolerate.includes(body.error)) return body;
  throw new FailClosed(
    response.status >= 500 ? 503 : 409,
    "migration_step_failed",
    `${step} failed: ${String(body.error ?? response.status)}`,
  );
}

async function current(ports: MigrationPorts, subjectKey: string): Promise<HostedAccountRecord> {
  const record = await ports.directory.get(subjectKey);
  if (!record) throw new FailClosed(404, "directory_account_missing", "no hosted account record for this subject");
  return record;
}

/**
 * Run (or resume) a migration to completion. Returns the final stable
 * record at the target. Throws on a refusal; running it again resumes
 * from the directory's phase.
 */
export async function driveMigration(ports: MigrationPorts, run: MigrationRun): Promise<HostedAccountRecord> {
  const target = parseJurisdiction(run.targetJurisdiction);
  let lease = await ports.directory.acquireLease(run.subjectKey, run.owner, ports.now(), run.leaseTtlMs);

  // #15828: a failed or refused run must not keep the lease for the
  // full TTL — the operator route mints a fresh owner per request, so
  // a held lease wedges every retry with `lease_held` even though no
  // driver is running. Release on every throw; releaseLease is a
  // no-op when another coordinator has already taken the lease over
  // (owner/token no longer match), so this cannot free a live lease.
  try {
    return await driveMigrationLoop(
      ports,
      run,
      target,
      () => lease,
      (next) => {
        lease = next;
      },
    );
  } catch (error) {
    await ports.directory.releaseLease(run.subjectKey, lease).catch(() => {});
    throw error;
  }
}

async function driveMigrationLoop(
  ports: MigrationPorts,
  run: MigrationRun,
  target: Jurisdiction,
  getLease: () => MigrationLease,
  setLease: (lease: MigrationLease) => void,
): Promise<HostedAccountRecord> {
  let lease = getLease();
  const step = async (
    record: HostedAccountRecord,
    event: Parameters<PersistentHostedAccountDirectory["transition"]>[1],
  ) => {
    lease = await ports.directory.acquireLease(run.subjectKey, run.owner, ports.now(), run.leaseTtlMs);
    setLease(lease);
    return ports.directory.transition(record, event, { lease, nowMs: ports.now() });
  };
  const mid = { migration_id: run.migrationId };

  for (let i = 0; i < MAX_STEPS; i++) {
    const record = await current(ports, run.subjectKey);
    if (record.phase === "stable") {
      if (record.jurisdiction === target) {
        await ports.directory.releaseLease(run.subjectKey, lease);
        return record;
      }
      await step(record, { type: "begin", migrationId: run.migrationId, targetJurisdiction: target });
      continue;
    }
    if (record.migrationId !== run.migrationId) {
      throw new FailClosed(409, "migration_in_progress", "another migration is running on this account");
    }
    const other = record.jurisdictionPending as Jurisdiction;
    const epoch = { ...mid, routing_epoch: record.routingEpoch };
    switch (record.phase) {
      case "pending": {
        await expectOk(
          await ports.control(record, record.jurisdiction, "POST", "/v1/migration/freeze", epoch),
          "source freeze",
        );
        await step(record, { type: "freeze-acknowledged", migrationId: run.migrationId });
        break;
      }
      case "frozen": {
        const snapshot = (await expectOk(
          await ports.control(record, record.jurisdiction, "POST", "/v1/migration/snapshot", epoch),
          "source snapshot",
        )) as unknown as MigrationSnapshot;
        for (const row of snapshot.tables.blobs) {
          await ports.copyBlob(
            record.bindingId,
            String(row.hash),
            snapshot.client_side_encryption,
            record.jurisdiction,
            other,
          );
        }
        const imported = await ports.control(
          record,
          other,
          "POST",
          "/v1/migration/import",
          snapshot,
          snapshot.client_side_encryption,
        );
        const importBody = await expectOk(imported, "target import", ["target_staged_other"]);
        if (importBody.error === "target_staged_other") {
          // A stale coordinator's import from an older epoch: clear it
          // and import again.
          await expectOk(await ports.control(record, other, "POST", "/v1/migration/discard", epoch), "target discard");
          await expectOk(
            await ports.control(
              record,
              other,
              "POST",
              "/v1/migration/import",
              snapshot,
              snapshot.client_side_encryption,
            ),
            "target import",
          );
        }
        const status = await expectOk(
          await ports.control(record, other, "GET", "/v1/migration/target"),
          "target status",
        );
        const staged = status.staged as StagedTarget | null;
        if (staged === null || staged.migration_id !== run.migrationId || staged.digest !== snapshot.digest) {
          throw new FailClosed(
            409,
            "migration_step_failed",
            "target check failed: the staged data does not match the source",
          );
        }
        await step(record, { type: "snapshot-verified", migrationId: run.migrationId });
        break;
      }
      case "verified":
        await step(record, { type: "switch", migrationId: run.migrationId });
        break;
      case "switched": {
        await expectOk(
          await ports.control(record, record.jurisdiction, "POST", "/v1/migration/activate", epoch),
          "target activate",
        );
        await expectOk(await ports.control(record, other, "POST", "/v1/migration/seal", epoch), "source seal");
        await step(record, { type: "complete", migrationId: run.migrationId });
        break;
      }
    }
  }
  throw new FailClosed(503, "migration_did_not_settle", "the migration did not reach a stable phase");
}

/**
 * Abort a migration before its switch: clear the target's staged copy,
 * move the directory back to the source at a fresh epoch, then reopen
 * the source with that epoch (which also refuses any late freeze from
 * a stale coordinator). Resumable like `driveMigration`; aborting after
 * the switch is refused.
 */
export async function abortMigration(
  ports: MigrationPorts,
  run: Omit<MigrationRun, "targetJurisdiction">,
): Promise<HostedAccountRecord> {
  let lease: MigrationLease = await ports.directory.acquireLease(
    run.subjectKey,
    run.owner,
    ports.now(),
    run.leaseTtlMs,
  );
  // #15828: like driveMigration, a refused or failed abort (e.g. the
  // after-switch refusal below) must release the lease it just took,
  // or it poisons the account for the full TTL.
  try {
    return await abortMigrationBody(ports, run, lease, (next) => {
      lease = next;
    });
  } catch (error) {
    await ports.directory.releaseLease(run.subjectKey, lease).catch(() => {});
    throw error;
  }
}

async function abortMigrationBody(
  ports: MigrationPorts,
  run: Omit<MigrationRun, "targetJurisdiction">,
  initialLease: MigrationLease,
  setLease: (lease: MigrationLease) => void,
): Promise<HostedAccountRecord> {
  let lease: MigrationLease = initialLease;
  let record = await current(ports, run.subjectKey);
  if (record.phase === "switched") {
    throw new FailClosed(
      409,
      "directory_transition_refused",
      "cannot abort after the switch; finish the migration instead",
    );
  }
  if (record.phase !== "stable") {
    if (record.migrationId !== run.migrationId) {
      throw new FailClosed(409, "migration_in_progress", "another migration is running on this account");
    }
    const target = record.jurisdictionPending as Jurisdiction;
    await expectOk(
      await ports.control(record, target, "POST", "/v1/migration/discard", {
        migration_id: run.migrationId,
        routing_epoch: record.routingEpoch,
      }),
      "target discard",
      ["not_staged"],
    );
    lease = await ports.directory.acquireLease(run.subjectKey, run.owner, ports.now(), run.leaseTtlMs);
    setLease(lease);
    record = await ports.directory.transition(
      record,
      { type: "abort", migrationId: run.migrationId },
      { lease, nowMs: ports.now() },
    );
  }
  // Stable at the source: reopen it at the current epoch. Repeating
  // this is harmless, and an unfenced source just raises its floor.
  await expectOk(
    await ports.control(record, record.jurisdiction, "POST", "/v1/migration/unfreeze", {
      migration_id: run.migrationId,
      routing_epoch: record.routingEpoch,
    }),
    "source unfreeze",
  );
  await ports.directory.releaseLease(run.subjectKey, lease);
  return record;
}

/** Production ports: user objects only through `userStub` on the
 * restricted namespace (the caller passes the front door's binding, so
 * the #3400 boundary holds), blob bytes through each jurisdiction's
 * bucket binding. */
export function workerMigrationPorts(
  namespace: ObjectNamespaceLike,
  env: BlobsEnv,
  directory: PersistentHostedAccountDirectory,
): MigrationPorts {
  return {
    directory,
    async control(record, jurisdiction, method, path, body, cse) {
      const headers = new Headers({ "content-type": "application/json", [HDR_MIGRATION]: "1" });
      headers.set(HDR_BINDING, record.bindingId);
      headers.set(HDR_JURISDICTION, jurisdiction);
      headers.set(HDR_ROUTED, "restricted");
      if (cse !== undefined) headers.set(HDR_CSE, cse ? "1" : "0");
      const stub = userStub(namespace, record.bindingId, jurisdiction);
      return stub.fetch(
        new Request(`https://hosted.internal${path}`, {
          method,
          headers,
          body: method === "POST" ? JSON.stringify(body ?? {}) : undefined,
        }),
      );
    },
    async copyBlob(bindingId, hash, cse, from, to) {
      const key = blobObjectKey(bindingId, hash, cse);
      const destination = blobsBindingFor(env, to);
      if ((await destination.head(key)) !== null) return;
      const object = await blobsBindingFor(env, from).get(key);
      if (!object) {
        throw new FailClosed(
          409,
          "migration_step_failed",
          `blob copy failed: ${hash} is missing from the source bucket`,
        );
      }
      await destination.put(key, new Uint8Array(await new Response(object.body).arrayBuffer()));
    },
    now: () => Date.now(),
  };
}
