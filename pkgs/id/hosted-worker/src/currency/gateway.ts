// Currency PoC (#2298) — stub gateway: the only ledger client.
//
// A single-process gateway in front of the ledger stub, shaped by
// the #2062 control-plane design (PR #2315) and Drew's ruling
// hosted-boundary-2298 (2026-11-12):
//
//   - server-scoped tokens: snapshots and receipts are signed with
//     THIS server's token admin key and carry this server's id; a
//     snapshot verifies only against that server's admin key and
//     expected server id — it is not ledger truth anywhere else;
//   - the hosted tier holds signed snapshots only (see relay.ts);
//   - cross-server transfers are bilateral signed double-entry:
//     each server records its own side against its settlement
//     account and signs its own entry; neither side treats the
//     other's record as its own ledger truth.
//
// Endpoints (the issue's step 1, plus the request-ID lookup the
// relay reconciles by on wake):
//   POST /v1/transfers            client-signed, idempotent by request ID
//   POST /v1/holds                client-signed two-phase transfer
//   POST /v1/holds/{id}/post      finalise a hold
//   POST /v1/holds/{id}/void      release a hold
//   GET  /v1/balances/{account}   gateway-signed snapshot
//   GET  /v1/transfers/{id}       lookup by request ID (reconciliation)
//
// Client signatures: a transfer/hold body carries the client's
// Ed25519 signature over the intent (domain "currency-intent/v1").
// The gateway holds only the client's PUBLIC key (registered per
// debit account at construction — the account factory's job in the
// real service); the signing key never reaches the gateway, the
// Worker, or the DO.

import { Ed25519Key, intentHash, signingPayload } from "./crypto.ts";
import { InProcessLedger, LedgerError, type Ledger, type TransferRecord } from "./ledger.ts";

export const INTENT_DOMAIN = "currency-intent/v1";
export const SNAPSHOT_DOMAIN = "currency-snapshot/v1";
export const RECEIPT_DOMAIN = "currency-receipt/v1";
export const ENTRY_DOMAIN = "currency-double-entry/v1";

export interface TransferIntent {
  request_id: string;
  from: string;
  to: string;
  amount: number;
}

export interface SignedSnapshot {
  server_id: string;
  account: string;
  balance: number;
  pending: number;
  watermark: number;
  issued_at: string;
  signature: string;
}

export interface GatewayConfig {
  serverId: string;
  adminKey: Ed25519Key;
  ledger?: Ledger;
  // debit account -> client public key hex (account-factory output)
  clientKeys: Record<string, string>;
  // settlement account used for cross-server entries
  settlementAccount?: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function snapshotBody(s: Omit<SignedSnapshot, "signature">): unknown {
  return {
    server_id: s.server_id,
    account: s.account,
    balance: s.balance,
    pending: s.pending,
    watermark: s.watermark,
    issued_at: s.issued_at,
  };
}

// Verify a snapshot exactly as a hosted-tier holder must: signature
// under the EXPECTED server's admin key, and the snapshot's server
// scope equal to the expected server. A snapshot from another
// server — even a well-signed one — is not valid here.
export async function verifySnapshot(
  snapshot: SignedSnapshot,
  expectedServerId: string,
  adminPublicKeyHex: string,
): Promise<boolean> {
  if (snapshot.server_id !== expectedServerId) return false;
  return Ed25519Key.verifyHex(
    adminPublicKeyHex,
    signingPayload(SNAPSHOT_DOMAIN, snapshotBody(snapshot)),
    snapshot.signature,
  );
}

// Client-side helper (runs wherever the client's key lives — never
// in the gateway/Worker/DO): sign a transfer or hold intent.
export async function signIntent(key: Ed25519Key, intent: TransferIntent): Promise<string> {
  return key.signHex(signingPayload(INTENT_DOMAIN, intent));
}

export class CurrencyGateway {
  readonly serverId: string;
  readonly adminKey: Ed25519Key;
  readonly ledger: Ledger;
  readonly #clientKeys: Record<string, string>;
  readonly #settlement: string;

  constructor(config: GatewayConfig) {
    this.serverId = config.serverId;
    this.adminKey = config.adminKey;
    this.ledger = config.ledger ?? new InProcessLedger();
    this.#clientKeys = config.clientKeys;
    this.#settlement = config.settlementAccount ?? "settlement";
  }

  adminPublicKeyHex(): string {
    return this.adminKey.publicKeyHex;
  }

  async snapshotFor(account: string): Promise<SignedSnapshot> {
    const bal = this.ledger.balance(account);
    const body = {
      server_id: this.serverId,
      account,
      balance: bal.balance,
      pending: bal.pending,
      watermark: bal.watermark,
      issued_at: new Date().toISOString(),
    };
    return {
      ...body,
      signature: await this.adminKey.signHex(signingPayload(SNAPSHOT_DOMAIN, snapshotBody(body))),
    };
  }

  async #receipt(rec: TransferRecord): Promise<string> {
    return this.adminKey.signHex(
      signingPayload(RECEIPT_DOMAIN, {
        server_id: this.serverId,
        request_id: rec.request_id,
        intent_hash: rec.intent_hash,
        state: rec.state,
        seq: rec.seq,
      }),
    );
  }

  async #transferView(rec: TransferRecord, deduplicated: boolean): Promise<unknown> {
    return { transfer: rec, deduplicated, receipt_signature: await this.#receipt(rec), server_id: this.serverId };
  }

  // Parse + authenticate a client-signed create body. Throws
  // LedgerError with an HTTP-ish code carried in `code`.
  async #signedCreate(body: unknown): Promise<{ intent: TransferIntent; hash: string }> {
    const b = body as Partial<TransferIntent> & { client_signature?: string };
    if (
      typeof b?.request_id !== "string" ||
      typeof b?.from !== "string" ||
      typeof b?.to !== "string" ||
      typeof b?.amount !== "number" ||
      typeof b?.client_signature !== "string"
    ) {
      throw new LedgerError("invalid_request", "body must carry request_id, from, to, amount, client_signature");
    }
    const intent: TransferIntent = { request_id: b.request_id, from: b.from, to: b.to, amount: b.amount };
    const pub = this.#clientKeys[intent.from];
    if (!pub || !(await Ed25519Key.verifyHex(pub, signingPayload(INTENT_DOMAIN, intent), b.client_signature))) {
      throw new LedgerError("invalid_client_signature", "client signature does not verify for the debit account");
    }
    return { intent, hash: await intentHash(intent) };
  }

  async fetch(req: Request): Promise<Response> {
    try {
      const url = new URL(req.url);
      const path = url.pathname;

      if (req.method === "POST" && (path === "/v1/transfers" || path === "/v1/holds")) {
        const { intent, hash } = await this.#signedCreate(await req.json());
        const input = {
          request_id: intent.request_id,
          debit_account: intent.from,
          credit_account: intent.to,
          amount: intent.amount,
          intent_hash: hash,
        };
        const result = path === "/v1/transfers" ? this.ledger.transfer(input) : this.ledger.hold(input);
        return json(await this.#transferView(result.transfer, result.deduplicated), result.deduplicated ? 200 : 201);
      }

      const holdAction = path.match(/^\/v1\/holds\/([^/]+)\/(post|void)$/);
      if (req.method === "POST" && holdAction) {
        const rec =
          holdAction[2] === "post"
            ? this.ledger.postHold(decodeURIComponent(holdAction[1]))
            : this.ledger.voidHold(decodeURIComponent(holdAction[1]));
        return json(await this.#transferView(rec, false));
      }

      const balanceMatch = path.match(/^\/v1\/balances\/([^/]+)$/);
      if (req.method === "GET" && balanceMatch) {
        return json(await this.snapshotFor(decodeURIComponent(balanceMatch[1])));
      }

      const lookupMatch = path.match(/^\/v1\/transfers\/([^/]+)$/);
      if (req.method === "GET" && lookupMatch) {
        const rec = this.ledger.lookupByRequestId(decodeURIComponent(lookupMatch[1]));
        if (!rec) return json({ error: "transfer_not_found" }, 404);
        return json(await this.#transferView(rec, false));
      }

      return json({ error: "not_found" }, 404);
    } catch (e) {
      if (e instanceof LedgerError) {
        const status =
          e.code === "invalid_client_signature"
            ? 401
            : e.code === "exists_with_different_fields"
              ? 409
              : e.code.endsWith("_not_found")
                ? 404
                : 400;
        return json({ error: e.code, message: e.message }, status);
      }
      throw e;
    }
  }

  // Bilateral signed double-entry (Drew's ruling): a cross-server
  // transfer is recorded on BOTH sides, each side against its own
  // settlement account, each entry signed by its own server's admin
  // key. Returns both entries with both signatures; each side can
  // verify the other's signature without trusting its ledger.
  async executeCrossServerTransfer(
    peer: CurrencyGateway,
    intent: TransferIntent,
    clientSignature: string,
  ): Promise<{ local: TransferRecord; remote: TransferRecord; local_signature: string; remote_signature: string }> {
    // The client signed the cross-server intent as stated (from ->
    // final recipient); verify that signature as-is, then record
    // this server's side against its settlement account.
    const pub = this.#clientKeys[intent.from];
    if (!pub || !(await Ed25519Key.verifyHex(pub, signingPayload(INTENT_DOMAIN, intent), clientSignature))) {
      throw new LedgerError("invalid_client_signature", "client signature does not verify for the debit account");
    }
    const localIntent: TransferIntent = { ...intent, to: this.#settlement };
    const local = this.ledger.transfer({
      request_id: intent.request_id,
      debit_account: intent.from,
      credit_account: this.#settlement,
      amount: intent.amount,
      intent_hash: await intentHash(localIntent),
    }).transfer;

    const remoteIntent: TransferIntent = { ...intent, from: peer.#settlement };
    const remote = peer.ledger.transfer({
      request_id: intent.request_id,
      debit_account: peer.#settlement,
      credit_account: intent.to,
      amount: intent.amount,
      intent_hash: await intentHash(remoteIntent),
    }).transfer;

    const entry = (serverId: string, rec: TransferRecord) => ({
      server_id: serverId,
      request_id: rec.request_id,
      debit_account: rec.debit_account,
      credit_account: rec.credit_account,
      amount: rec.amount,
    });
    return {
      local,
      remote,
      local_signature: await this.adminKey.signHex(signingPayload(ENTRY_DOMAIN, entry(this.serverId, local))),
      remote_signature: await peer.adminKey.signHex(signingPayload(ENTRY_DOMAIN, entry(peer.serverId, remote))),
    };
  }
}
