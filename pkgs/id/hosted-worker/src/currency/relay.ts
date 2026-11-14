// Currency PoC (#2298) — hosted-edition client: the UserDO relay.
//
// One UserDO per user is the hosted tier's only currency actor, and
// per the #2066 boundary (accepted for #2043; Drew ruling
// hosted-boundary-2298) it holds signed snapshots only, never
// ledger truth. Its write path, end to end:
//
//   1. persist {request ID, intent hash, state} in its own storage
//      BEFORE the gateway fetch — a hibernation (kill/restart)
//      between submit and response can never lose the intent;
//   2. forward the CLIENT-signed intent (the client signs; this
//      relay, the Worker, and the DO never hold the signing key);
//   3. on a definite response, mark the intent completed with the
//      gateway's signed receipt and cache the gateway-signed
//      balance snapshot verbatim;
//   4. on wake with an intent still `submitted` (the response was
//      lost), reconcile by request-ID lookup against the gateway
//      instead of resubmitting blind: found → completed; absent →
//      the submit never landed and a retry is safe.
//
// A retry with the same request ID and identical fields is answered
// by the gateway's idempotency gate (exactly one transfer exists);
// a retry with a mutated field surfaces the ledger's
// exists_with_different_fields as a hard error (RelayError), never
// as a silent second transfer.
//
// Storage is behind a two-method store interface: SqlIntentStore
// runs against a Durable Object's SQLite storage (the same exec
// shape as user-do.ts); MemoryIntentStore backs the tests, where a
// "hibernation" is a fresh relay instance over the same store.

import { Ed25519Key, intentHash } from "./crypto.ts";
import { verifySnapshot, type SignedSnapshot, type TransferIntent } from "./gateway.ts";
import type { TransferRecord } from "./ledger.ts";

export interface IntentStore {
  get(key: string): Promise<string | null>;
  put(key: string, value: string): Promise<void>;
}

export class MemoryIntentStore implements IntentStore {
  readonly map = new Map<string, string>();
  async get(key: string): Promise<string | null> {
    return this.map.get(key) ?? null;
  }
  async put(key: string, value: string): Promise<void> {
    this.map.set(key, value);
  }
}

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}

// DO-backed store: a dedicated intents table with the user-do.ts
// schema-bootstrap posture (created idempotently before serving).
export class SqlIntentStore implements IntentStore {
  readonly #sql: SqlStorage;
  constructor(sql: SqlStorage) {
    this.#sql = sql;
    this.#sql.exec("CREATE TABLE IF NOT EXISTS currency_intents (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
  }
  async get(key: string): Promise<string | null> {
    const rows = this.#sql
      .exec("SELECT value FROM currency_intents WHERE key = ?", key)
      .toArray();
    return rows.length === 0 ? null : String(rows[0].value);
  }
  async put(key: string, value: string): Promise<void> {
    this.#sql.exec(
      "INSERT INTO currency_intents (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      key,
      value,
    );
  }
}

export type IntentState = "submitted" | "completed" | "not_submitted" | "failed";

export interface PersistedIntent {
  request_id: string;
  intent_hash: string;
  state: IntentState;
  intent: TransferIntent;
  client_signature: string;
  receipt_signature?: string;
  transfer_state?: string;
  error?: string;
}

export class RelayError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export interface GatewayLike {
  fetch(req: Request): Promise<Response>;
}

export interface RelayConfig {
  store: IntentStore;
  gateway: GatewayLike;
  gatewayUrl?: string;
  // The gateway server this relay trusts, and its admin public key:
  // snapshots are cached only after verification against both.
  serverId: string;
  adminPublicKeyHex: string;
  account: string;
}

const intentKey = (requestId: string) => "intent:" + requestId;
const SNAPSHOT_KEY = "snapshot:latest";

export class UserDORelay {
  readonly #store: IntentStore;
  readonly #gateway: GatewayLike;
  readonly #base: string;
  readonly #serverId: string;
  readonly #adminPub: string;
  readonly account: string;

  constructor(config: RelayConfig) {
    this.#store = config.store;
    this.#gateway = config.gateway;
    this.#base = config.gatewayUrl ?? "https://gateway.invalid";
    this.#serverId = config.serverId;
    this.#adminPub = config.adminPublicKeyHex;
    this.account = config.account;
  }

  async #readIntent(requestId: string): Promise<PersistedIntent | null> {
    const raw = await this.#store.get(intentKey(requestId));
    return raw === null ? null : (JSON.parse(raw) as PersistedIntent);
  }

  async #writeIntent(rec: PersistedIntent): Promise<void> {
    await this.#store.put(intentKey(rec.request_id), JSON.stringify(rec));
  }

  async intentState(requestId: string): Promise<PersistedIntent | null> {
    return this.#readIntent(requestId);
  }

  // Submit (or re-submit) a client-signed transfer. The intent row
  // is durable BEFORE the gateway fetch in every path.
  async submitTransfer(intent: TransferIntent, clientSignature: string): Promise<PersistedIntent> {
    return this.#submit("/v1/transfers", intent, clientSignature);
  }

  async submitHold(intent: TransferIntent, clientSignature: string): Promise<PersistedIntent> {
    return this.#submit("/v1/holds", intent, clientSignature);
  }

  async #submit(path: string, intent: TransferIntent, clientSignature: string): Promise<PersistedIntent> {
    const hash = await intentHash(intent);
    const existing = await this.#readIntent(intent.request_id);
    if (existing) {
      if (existing.intent_hash !== hash) {
        // Local form of the same hard error: never forward a mutated
        // retry under a request ID this DO already used.
        throw new RelayError(
          "exists_with_different_fields",
          "request " + intent.request_id + " was already submitted with different fields",
        );
      }
      if (existing.state === "completed") return existing;
      if (existing.state === "submitted") {
        const reconciled = await this.reconcile(intent.request_id);
        if (reconciled.state === "completed") return reconciled;
        // not_submitted: fall through and resubmit the same intent.
      }
    }

    const rec: PersistedIntent = {
      request_id: intent.request_id,
      intent_hash: hash,
      state: "submitted",
      intent,
      client_signature: clientSignature,
    };
    await this.#writeIntent(rec); // durable BEFORE the gateway fetch

    let res: Response;
    try {
      res = await this.#gateway.fetch(
        new Request(this.#base + path, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ ...intent, client_signature: clientSignature }),
        }),
      );
    } catch (e) {
      // Response lost (hibernation, transport): the intent stays
      // `submitted`; reconcile() on wake settles it by lookup.
      throw new RelayError("response_lost", "gateway response lost for " + intent.request_id + ": " + String(e));
    }
    return this.#settle(rec, res);
  }

  async #settle(rec: PersistedIntent, res: Response): Promise<PersistedIntent> {
    const body = (await res.json()) as {
      transfer?: TransferRecord;
      receipt_signature?: string;
      error?: string;
      message?: string;
    };
    if (!res.ok || !body.transfer) {
      rec.state = "failed";
      rec.error = body.error ?? "gateway_error";
      await this.#writeIntent(rec);
      throw new RelayError(rec.error, body.message ?? "gateway rejected " + rec.request_id);
    }
    rec.state = "completed";
    rec.receipt_signature = body.receipt_signature;
    rec.transfer_state = body.transfer.state;
    await this.#writeIntent(rec);
    await this.refreshSnapshot();
    return rec;
  }

  // Wake-path reconciliation: settle a `submitted` intent by
  // request-ID lookup. Never resubmits; the lookup is the proof.
  async reconcile(requestId: string): Promise<PersistedIntent> {
    const rec = await this.#readIntent(requestId);
    if (!rec) throw new RelayError("unknown_request", "no persisted intent for " + requestId);
    if (rec.state === "completed") return rec;
    const res = await this.#gateway.fetch(new Request(this.#base + "/v1/transfers/" + encodeURIComponent(requestId)));
    if (res.status === 404) {
      rec.state = "not_submitted";
      await this.#writeIntent(rec);
      return rec;
    }
    return this.#settle(rec, res);
  }

  async holdAction(requestId: string, action: "post" | "void"): Promise<PersistedIntent> {
    const rec = await this.#readIntent(requestId);
    if (!rec) throw new RelayError("unknown_request", "no persisted intent for " + requestId);
    const res = await this.#gateway.fetch(
      new Request(this.#base + "/v1/holds/" + encodeURIComponent(requestId) + "/" + action, { method: "POST" }),
    );
    return this.#settle(rec, res);
  }

  // Fetch the gateway-signed snapshot for this relay's account and
  // cache it VERBATIM (signature included) — the hosted tier holds
  // signed snapshots only. An unverifiable or wrong-server snapshot
  // is refused, never cached.
  async refreshSnapshot(): Promise<SignedSnapshot> {
    const res = await this.#gateway.fetch(
      new Request(this.#base + "/v1/balances/" + encodeURIComponent(this.account)),
    );
    if (!res.ok) throw new RelayError("snapshot_failed", "snapshot fetch failed: " + res.status);
    const snapshot = (await res.json()) as SignedSnapshot;
    if (!(await verifySnapshot(snapshot, this.#serverId, this.#adminPub))) {
      throw new RelayError("snapshot_unverifiable", "gateway snapshot did not verify for " + this.#serverId);
    }
    await this.#store.put(SNAPSHOT_KEY, JSON.stringify(snapshot));
    return snapshot;
  }

  // The only balance read the hosted tier serves: the cached signed
  // snapshot, re-verified on every read.
  async cachedBalance(): Promise<SignedSnapshot> {
    const raw = await this.#store.get(SNAPSHOT_KEY);
    if (raw === null) throw new RelayError("no_snapshot", "no signed snapshot cached yet");
    const snapshot = JSON.parse(raw) as SignedSnapshot;
    if (!(await verifySnapshot(snapshot, this.#serverId, this.#adminPub))) {
      throw new RelayError("snapshot_unverifiable", "cached snapshot did not verify for " + this.#serverId);
    }
    return snapshot;
  }
}

// Re-exported so consumers of the relay never need the gateway's
// key material: verification keys are public by construction.
export { Ed25519Key };
