// Currency PoC (#2298) end-to-end proofs.
//
// Oracle for every test: the issue body's four proofs —
//   1. stub gateway endpoints live (transfers, holds + post/void,
//      gateway-signed balance snapshots);
//   2. the UserDO relay persists {request ID, intent hash, state}
//      before the gateway fetch and caches the signed snapshot;
//   3. one client-signed transfer and one hold → post cycle driven
//      through the relay (the relay never holds the signing key —
//      it only forwards the client's signature);
//   4. hibernation idempotency: kill/restart between submit and
//      response, retry with the same request ID → exactly one
//      transfer; a mutated retry under the same ID surfaces
//      exists_with_different_fields as a hard error.
// Plus Drew's ruling (hosted-boundary-2298): server-scoped signed
// snapshots, and cross-server transfers as bilateral signed
// double-entry.

import { test } from "node:test";
import assert from "node:assert/strict";
import { Ed25519Key } from "../src/currency/crypto.ts";
import { CurrencyGateway, signIntent, verifySnapshot } from "../src/currency/gateway.ts";
import { InProcessLedger } from "../src/currency/ledger.ts";
import { MemoryIntentStore, RelayError, UserDORelay } from "../src/currency/relay.ts";

const SERVER = "server-a";
const ALICE = "alice";
const BOB = "bob";

async function setup() {
  const clientKey = await Ed25519Key.generate();
  const ledger = new InProcessLedger();
  ledger.createAccount(ALICE, 1000);
  ledger.createAccount(BOB, 0);
  ledger.createAccount("settlement", 5000);
  const gateway = new CurrencyGateway({
    serverId: SERVER,
    adminKey: await Ed25519Key.generate(),
    ledger,
    clientKeys: { [ALICE]: clientKey.publicKeyHex },
  });
  const store = new MemoryIntentStore();
  const relay = new UserDORelay({
    store,
    gateway,
    serverId: SERVER,
    adminPublicKeyHex: gateway.adminPublicKeyHex(),
    account: ALICE,
  });
  return { clientKey, ledger, gateway, store, relay };
}

const post = (path: string, body: unknown) =>
  new Request("https://gw" + path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });

test("proof 1: gateway transfer + signed snapshot verifies and is server-scoped", async () => {
  const { clientKey, gateway } = await setup();
  const intent = { request_id: "req-1", from: ALICE, to: BOB, amount: 250 };
  const res = await gateway.fetch(
    post("/v1/transfers", { ...intent, client_signature: await signIntent(clientKey, intent) }),
  );
  assert.equal(res.status, 201);
  const body = await res.json();
  assert.equal(body.transfer.state, "posted");
  assert.equal(body.deduplicated, false);

  const snapRes = await gateway.fetch(new Request("https://gw/v1/balances/" + ALICE));
  const snap = await snapRes.json();
  assert.equal(snap.balance, 750);
  assert.equal(snap.watermark, body.transfer.seq);
  assert.equal(await verifySnapshot(snap, SERVER, gateway.adminPublicKeyHex()), true);
  // Tampered balance no longer verifies.
  assert.equal(await verifySnapshot({ ...snap, balance: 9999 }, SERVER, gateway.adminPublicKeyHex()), false);
  // The same well-signed snapshot is not valid under another server's scope.
  assert.equal(await verifySnapshot(snap, "server-b", gateway.adminPublicKeyHex()), false);
});

test("proof 1b: gateway rejects an unsigned or wrongly-signed transfer", async () => {
  const { gateway } = await setup();
  const intent = { request_id: "req-x", from: ALICE, to: BOB, amount: 10 };
  const wrong = await Ed25519Key.generate();
  const res = await gateway.fetch(
    post("/v1/transfers", { ...intent, client_signature: await signIntent(wrong, intent) }),
  );
  assert.equal(res.status, 401);
  assert.equal((await res.json()).error, "invalid_client_signature");
});

test("proof 3: client-signed transfer and hold -> post cycle through the relay", async () => {
  const { clientKey, relay, ledger } = await setup();
  const t = { request_id: "req-t1", from: ALICE, to: BOB, amount: 100 };
  const done = await relay.submitTransfer(t, await signIntent(clientKey, t));
  assert.equal(done.state, "completed");
  assert.equal(done.transfer_state, "posted");
  assert.ok(done.receipt_signature);

  const h = { request_id: "req-h1", from: ALICE, to: BOB, amount: 300 };
  const held = await relay.submitHold(h, await signIntent(clientKey, h));
  assert.equal(held.transfer_state, "pending");
  assert.equal(ledger.balance(ALICE).pending, 300);
  assert.equal(ledger.balance(BOB).balance, 100); // hold has not landed yet

  const posted = await relay.holdAction("req-h1", "post");
  assert.equal(posted.transfer_state, "posted");
  assert.equal(ledger.balance(ALICE).pending, 0);
  assert.equal(ledger.balance(ALICE).balance, 600);
  assert.equal(ledger.balance(BOB).balance, 400);

  // The relay's only balance view is the cached signed snapshot.
  const cached = await relay.cachedBalance();
  assert.equal(cached.balance, 600);
  assert.equal(cached.server_id, SERVER);
});

test("proof 1c: hold -> void releases the reservation", async () => {
  const { clientKey, relay, ledger } = await setup();
  const h = { request_id: "req-h2", from: ALICE, to: BOB, amount: 200 };
  await relay.submitHold(h, await signIntent(clientKey, h));
  const voided = await relay.holdAction("req-h2", "void");
  assert.equal(voided.transfer_state, "voided");
  assert.equal(ledger.balance(ALICE).pending, 0);
  assert.equal(ledger.balance(ALICE).balance, 1000);
  assert.equal(ledger.balance(BOB).balance, 0);
});

test("proof 2: the intent is persisted before the gateway fetch", async () => {
  const { clientKey, store, gateway } = await setup();
  let seenAtFetch: string | null = null;
  const spying = {
    async fetch(req: Request): Promise<Response> {
      if (req.method === "POST") seenAtFetch = await store.get("intent:req-p1");
      return gateway.fetch(req);
    },
  };
  const relay = new UserDORelay({
    store,
    gateway: spying,
    serverId: SERVER,
    adminPublicKeyHex: gateway.adminPublicKeyHex(),
    account: ALICE,
  });
  const t = { request_id: "req-p1", from: ALICE, to: BOB, amount: 40 };
  await relay.submitTransfer(t, await signIntent(clientKey, t));
  assert.ok(seenAtFetch, "intent row must exist before the gateway fetch runs");
  const persisted = JSON.parse(seenAtFetch as string);
  assert.equal(persisted.state, "submitted");
  assert.equal(persisted.request_id, "req-p1");
  assert.ok(persisted.intent_hash);
});

test("proof 4: hibernation — response lost, wake reconciles, retry stays exactly one transfer", async () => {
  const { clientKey, store, gateway, ledger } = await setup();
  // Wrap the gateway so the first POST lands the transfer but the
  // response is lost — the DO is killed between submit and response.
  let dropNext = true;
  const flaky = {
    async fetch(req: Request): Promise<Response> {
      const res = await gateway.fetch(req);
      if (dropNext && req.method === "POST") {
        dropNext = false;
        await res.text();
        throw new Error("simulated hibernation: response lost");
      }
      return res;
    },
  };
  const mkRelay = () =>
    new UserDORelay({
      store,
      gateway: flaky,
      serverId: SERVER,
      adminPublicKeyHex: gateway.adminPublicKeyHex(),
      account: ALICE,
    });

  const t = { request_id: "req-hib", from: ALICE, to: BOB, amount: 125 };
  const sig = await signIntent(clientKey, t);
  await assert.rejects(() => mkRelay().submitTransfer(t, sig), RelayError);
  assert.equal((await mkRelay().intentState("req-hib"))?.state, "submitted");
  assert.equal(ledger.transferCount(), 1); // the submit DID land

  // Wake: a fresh relay over the same store reconciles by lookup.
  const woken = mkRelay();
  const settled = await woken.reconcile("req-hib");
  assert.equal(settled.state, "completed");

  // Retry with the same request ID + identical fields: deduplicated,
  // and still exactly one transfer exists.
  const retry = await woken.submitTransfer(t, sig);
  assert.equal(retry.state, "completed");
  assert.equal(ledger.transferCount(), 1);
  assert.equal(ledger.balance(BOB).balance, 125);
});

test("proof 4b: mutated retry under the same request ID is a hard error", async () => {
  const { clientKey, relay, gateway, ledger } = await setup();
  const t = { request_id: "req-mut", from: ALICE, to: BOB, amount: 50 };
  await relay.submitTransfer(t, await signIntent(clientKey, t));

  // Direct gateway retry with a mutated amount (fresh signature over
  // the mutated intent, same request ID) hits the ledger gate.
  const mutated = { ...t, amount: 51 };
  const res = await gateway.fetch(
    post("/v1/transfers", { ...mutated, client_signature: await signIntent(clientKey, mutated) }),
  );
  assert.equal(res.status, 409);
  assert.equal((await res.json()).error, "exists_with_different_fields");
  assert.equal(ledger.transferCount(), 1);

  // The relay refuses a mutated retry locally as well.
  await assert.rejects(
    async () => relay.submitTransfer(mutated, await signIntent(clientKey, mutated)),
    (e: unknown) => e instanceof RelayError && e.code === "exists_with_different_fields",
  );
});

test("ruling: cross-server transfer is bilateral signed double-entry", async () => {
  const clientKey = await Ed25519Key.generate();
  const mkGateway = async (serverId: string) => {
    const ledger = new InProcessLedger();
    ledger.createAccount(ALICE, 1000);
    ledger.createAccount(BOB, 0);
    ledger.createAccount("settlement", 5000);
    return new CurrencyGateway({
      serverId,
      adminKey: await Ed25519Key.generate(),
      ledger,
      clientKeys: { [ALICE]: clientKey.publicKeyHex },
    });
  };
  const a = await mkGateway("server-a");
  const b = await mkGateway("server-b");
  const intent = { request_id: "req-xs", from: ALICE, to: BOB, amount: 70 };
  const out = await a.executeCrossServerTransfer(b, intent, await signIntent(clientKey, intent));

  // Both sides recorded their own side; neither shares a ledger.
  assert.equal(a.ledger.balance(ALICE).balance, 930);
  assert.equal(a.ledger.balance("settlement").balance, 5070);
  assert.equal(b.ledger.balance(BOB).balance, 70);
  assert.equal(b.ledger.balance("settlement").balance, 4930);
  assert.ok(out.local_signature && out.remote_signature);
  assert.notEqual(out.local_signature, out.remote_signature);
});
