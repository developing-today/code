// The bridge wire protocol: authenticated push/pull sessions
// between a native node (or the reference client below) and the
// hosted per-user store, over one ordered binary message channel —
// a WebSocket in production.
//
// A session is the #2276 proof of concept's session, hardened into
// the production shape the #2046 investigation requires:
//
//   CHALLENGE / AUTH   the ID-key proof (bridge-auth.ts); nothing
//                      moves before it verifies.
//   HELLO              each side announces its chain heads — the
//                      frontier evidence withholding detection reads.
//   RESUME / RESUMED   a returning client may present the token of
//                      its last completed session plus its positions;
//                      the token is checked, the positions are
//                      evidence only, and the range exchange below
//                      re-derives the truth either way.
//   RANGE flights      range.ts reconciliation over the sync set.
//                      Because fork-blindness is real (the
//                      investigation, finding 2), the exchange is
//                      the proof-carrying range protocol, never a
//                      height comparison.
//   WANT / BATCH /     each side asks for the ids it learned it is
//   WANT_END / ACK     missing; batches are bounded in count and
//                      bytes, and every batch is acknowledged with
//                      its ingest outcomes, so a sender can retry
//                      exactly what was refused — and a re-sent event
//                      is a duplicate, never a second copy.
//   DONE / TOKEN       both sides done; the store issues the resume
//                      token for the next session.
//
// Withholding is detectable, not silent: if a peer announces a head
// it never delivers, `detectWithholding` produces the evidence
// record (announced head vs applied frontier) at session end.
//
// Bounds, all enforced on receipt: event bytes, batch count, batch
// bytes, want count, frame size, and the range session budget from
// range.ts. A session that breaks a bound ends with a named error.

import {
  authTranscript,
  bodyHashHex,
  checkResumeToken,
  issueNonce,
  issueResumeToken,
  verifyProof,
} from "./bridge-auth.ts";
import type { BridgeStore, IngestOutcome } from "./bridge-store.ts";
import { bytesToHex, type IdKey } from "./ed25519.ts";
import { decodeEvent, Malformed } from "./event.ts";
import {
  allSkip,
  decodeFrame,
  decodeFlight,
  frameEntryCount,
  frameFlight,
  RangeParty,
  type RangeEntry,
  type RangeParams,
  DEFAULT_RANGE_PARAMS,
  ENTRY_CHARGE,
} from "./range.ts";

export const TAG = {
  challenge: 0x01,
  auth: 0x02,
  hello: 0x10,
  range: 0x11,
  want: 0x12,
  batch: 0x13,
  done: 0x14,
  resume: 0x15,
  resumed: 0x16,
  token: 0x17,
  ack: 0x18,
  wantEnd: 0x19,
  error: 0x1f,
} as const;

export interface BridgeLimits {
  maxEventBytes: number;
  maxBatchEvents: number;
  maxBatchBytes: number;
  maxWantIds: number;
}

export const DEFAULT_BRIDGE_LIMITS: BridgeLimits = {
  maxEventBytes: 64 * 1024,
  maxBatchEvents: 64,
  maxBatchBytes: 1024 * 1024,
  maxWantIds: 512,
};

export interface Head {
  author: string;
  next_seq: number;
  head: string | null;
}

export interface Withholding {
  author: string;
  announced_next_seq: number;
  announced_head: string | null;
  actual_next_seq: number;
  undelivered: string[];
}

/** Withholding evidence, the investigation's shape: for each
 * announced head the store cannot apply, the events the peer
 * authored (from what arrived, plus the announced head itself when
 * it is among the missing) that never became held here. An author
 * whose events arrived but wait on a withheld dependency is not
 * itself withholding — its own head applies or waits on another
 * author's gap, and the record names the author actually missing. */
export function detectWithholding(
  announced: Head[],
  store: BridgeStore,
  authored: { author: string; id: string }[],
  missing: Set<string>,
): Withholding[] {
  const held = new Set(store.syncEntries().map((e) => e.id));
  const out: Withholding[] = [];
  for (const head of announced) {
    const chain = store.chainOf(head.author);
    if (chain.next_seq >= head.next_seq) continue;
    const undelivered = new Set<string>();
    for (const pair of authored) {
      if (pair.author === head.author && !held.has(pair.id)) undelivered.add(pair.id);
    }
    if (head.head && missing.has(head.head)) undelivered.add(head.head);
    if (undelivered.size > 0) {
      out.push({
        author: head.author,
        announced_next_seq: head.next_seq,
        announced_head: head.head,
        actual_next_seq: chain.next_seq,
        undelivered: [...undelivered].sort(),
      });
    }
  }
  return out;
}

// ----- batch codec ---------------------------------------------------

/** Bytes one event occupies inside an encoded batch: its 4-byte
 * length prefix plus its payload. Senders budget this framed size —
 * receivers enforce maxBatchBytes against the encoded batch, not
 * the payload sum (#14644). */
export function encodedEventBytes(event: Uint8Array): number {
  return 4 + event.length;
}

/** The encoded size of a batch of `events`: the 4-byte count prefix
 * plus every event's framed size — exactly what encodeBatch emits
 * and what decodeBatch bounds. */
export function encodedBatchBytes(events: Uint8Array[]): number {
  let size = 4;
  for (const e of events) size += encodedEventBytes(e);
  return size;
}

/** Splits `events` into batches that each fit `limits` by count and
 * by encoded size, preserving order: a batch closes before the
 * first event that would cross either bound. An event whose framed
 * size alone exceeds maxBatchBytes can never be sent under these
 * limits — that is a named error, never an oversized batch. */
export function splitBatches(events: Uint8Array[], limits: BridgeLimits): Uint8Array[][] {
  const batches: Uint8Array[][] = [];
  let batch: Uint8Array[] = [];
  let batchBytes = 4; // the encoded count prefix
  for (const event of events) {
    const framed = encodedEventBytes(event);
    if (
      batch.length > 0 &&
      (batch.length >= limits.maxBatchEvents || batchBytes + framed > limits.maxBatchBytes)
    ) {
      batches.push(batch);
      batch = [];
      batchBytes = 4;
    }
    if (batchBytes + framed > limits.maxBatchBytes) {
      throw new Error(
        `event of ${event.length} bytes cannot fit the ${limits.maxBatchBytes}-byte batch bound`,
      );
    }
    batch.push(event);
    batchBytes += framed;
  }
  if (batch.length > 0) batches.push(batch);
  return batches;
}

export function encodeBatch(events: Uint8Array[]): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(encodedBatchBytes(events));
  const view = new DataView(out.buffer);
  view.setUint32(0, events.length);
  let at = 4;
  for (const e of events) {
    view.setUint32(at, e.length);
    at += 4;
    out.set(e, at);
    at += e.length;
  }
  return out;
}

export function decodeBatch(bytes: Uint8Array, limits: BridgeLimits): Uint8Array[] {
  if (bytes.length > limits.maxBatchBytes) {
    throw new Error(`batch of ${bytes.length} bytes is over the ${limits.maxBatchBytes}-byte bound`);
  }
  if (bytes.length < 4) throw new Error("batch is truncated");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const count = view.getUint32(0);
  if (count > limits.maxBatchEvents) {
    throw new Error(`batch of ${count} events is over the ${limits.maxBatchEvents}-event bound`);
  }
  const events: Uint8Array[] = [];
  let at = 4;
  for (let i = 0; i < count; i++) {
    if (at + 4 > bytes.length) throw new Error("batch is truncated");
    const len = view.getUint32(at);
    at += 4;
    if (len > limits.maxEventBytes) {
      throw new Error(`event of ${len} bytes is over the ${limits.maxEventBytes}-byte bound`);
    }
    if (at + len > bytes.length) throw new Error("batch is truncated");
    events.push(bytes.subarray(at, at + len));
    at += len;
  }
  if (at !== bytes.length) throw new Error("trailing bytes after the batch");
  return events;
}

/** Decodes raw event bytes into ingest entries, in order: a verified
 * event, or the refusal recorded in its place (the native
 * ingest_batch shape). */
export async function decodeEntries(
  rawEvents: Uint8Array[],
): Promise<(Awaited<ReturnType<typeof decodeEvent>> | { malformed: string })[]> {
  const entries: (Awaited<ReturnType<typeof decodeEvent>> | { malformed: string })[] = [];
  for (const raw of rawEvents) {
    try {
      entries.push(await decodeEvent(raw));
    } catch (e) {
      entries.push({ malformed: e instanceof Malformed ? e.message : "event failed to decode" });
    }
  }
  return entries;
}

export function summarizeOutcomes(outcomes: IngestOutcome[]): Record<string, number> {
  const summary: Record<string, number> = {
    received: outcomes.length,
    accepted: 0,
    duplicate: 0,
    fork: 0,
    waiting: 0,
    refused: 0,
  };
  for (const o of outcomes) summary[o.kind] += 1;
  return summary;
}

// ----- transport -----------------------------------------------------

export interface FrameTransport {
  send(data: Uint8Array): void;
  onMessage(cb: (data: Uint8Array) => void | Promise<void>): void;
  onClose(cb: () => void): void;
  close(): void;
}

function frame(tag: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(1 + payload.length);
  out[0] = tag;
  out.set(payload, 1);
  return out;
}

function jsonFrame(tag: number, value: unknown): Uint8Array {
  return frame(tag, new TextEncoder().encode(JSON.stringify(value)));
}

function parseJson(payload: Uint8Array): unknown {
  return JSON.parse(new TextDecoder().decode(payload));
}

// ----- the server session (the hosted store side) --------------------

export interface ServerSessionOptions {
  store: BridgeStore;
  bindingId: string;
  /** The bound ID key (hex) from the binding record. */
  irohHex: string;
  transport: FrameTransport;
  params?: RangeParams;
  limits?: BridgeLimits;
  now?: () => number;
  /** Storage-ceiling guard: given the bytes a batch is about to add,
   * return false to refuse it before anything is written. */
  guardBatch?: (newBytes: number) => boolean;
}

export class BridgeSessionServer {
  private authed = false;
  private announced: Head[] = [];
  private learned = new Set<string>();
  private rangeFlight: Uint8Array[] = [];
  private spent = 0;
  private closed = false;
  private tokenIssued = false;
  private queue: Promise<void> = Promise.resolve();

  constructor(private opts: ServerSessionOptions) {
    // Messages are handled strictly in arrival order: an async
    // handler (batch ingest verifies signatures) must not let a
    // later message overtake an earlier one.
    this.opts.transport.onMessage((data) => {
      this.queue = this.queue.then(() => this.onMessage(data));
    });
    this.opts.transport.onClose(() => {
      this.closed = true;
    });
    const { nonce } = issueNonce(this.opts.store, this.now());
    this.send(frame(TAG.challenge, new TextEncoder().encode(nonce)));
  }

  private get params(): RangeParams {
    return this.opts.params ?? DEFAULT_RANGE_PARAMS;
  }

  private get limits(): BridgeLimits {
    return this.opts.limits ?? DEFAULT_BRIDGE_LIMITS;
  }

  private now(): number {
    return this.opts.now ? this.opts.now() : Date.now();
  }

  private send(data: Uint8Array): void {
    if (!this.closed) this.opts.transport.send(data);
  }

  private fail(code: string, message: string): void {
    this.send(jsonFrame(TAG.error, { code, message }));
    this.closed = true;
    this.opts.transport.close();
  }

  private async onMessage(data: Uint8Array): Promise<void> {
    if (this.closed || data.length === 0) return;
    const tag = data[0];
    const payload = data.subarray(1);
    try {
      if (!this.authed) {
        if (tag !== TAG.auth) return this.fail("auth_required", "the session is not authenticated");
        await this.onAuth(payload);
        return;
      }
      switch (tag) {
        case TAG.hello:
          this.announced = parseJson(payload) as Head[];
          this.send(jsonFrame(TAG.hello, this.opts.store.heads()));
          return;
        case TAG.resume:
          this.onResume(payload);
          return;
        case TAG.range:
          this.onRangeFrame(payload);
          return;
        case TAG.want:
          this.onWant(payload);
          return;
        case TAG.batch:
          await this.onBatch(payload);
          return;
        case TAG.wantEnd:
          this.onClientWantEnd();
          return;
        case TAG.done:
          this.onDone();
          return;
        default:
          return this.fail("unknown_tag", `unknown message tag ${tag}`);
      }
    } catch (e) {
      return this.fail("session_error", e instanceof Error ? e.message : "session failed");
    }
  }

  private async onAuth(payload: Uint8Array): Promise<void> {
    const body = parseJson(payload) as { nonce: string; signature: string };
    const ok = await verifyProof(this.opts.store, {
      bindingId: this.opts.bindingId,
      irohHex: this.opts.irohHex,
      context: "ws",
      body: new Uint8Array(0),
      nonce: body.nonce,
      signatureHex: body.signature,
      nowMs: this.now(),
    });
    if (!ok) return this.fail("auth_failed", "the ID-key proof did not verify");
    this.authed = true;
  }

  private onResume(payload: Uint8Array): void {
    const body = parseJson(payload) as {
      token: string;
      positions: { author: string; next_seq: number; head: string | null }[];
    };
    if (!checkResumeToken(this.opts.store, body.token, this.opts.bindingId, this.now())) {
      return this.fail("resume_rejected", "the resume token is unknown or expired");
    }
    this.announced = body.positions;
    this.send(
      jsonFrame(TAG.resumed, {
        positions: this.opts.store.heads(body.positions.map((p) => p.author)),
      }),
    );
  }

  private onRangeFrame(payload: Uint8Array): void {
    this.spent += payload.length + frameEntryCount(payload) * ENTRY_CHARGE;
    if (this.spent > this.params.sessionBudget) {
      return this.fail("budget_exceeded", "the range session budget is spent");
    }
    this.rangeFlight.push(new Uint8Array(payload));
    const last = decodeFrame(payload);
    if (last.more) return;
    const flight = decodeFlight(this.rangeFlight);
    this.rangeFlight = [];
    if (allSkip(flight)) {
      // The peer is done; echo the all-skip flight so it can finish
      // too (the simulate loop's closing flight).
      this.send(frame(TAG.range, frameFlight([{ upper: null, mode: { t: "skip" } }], this.params.frameBudget)[0]));
      return;
    }
    const party = new RangeParty(this.opts.store.syncEntries());
    const reply = party.respond(flight, this.params, this.learned);
    for (const wire of frameFlight(reply, this.params.frameBudget)) {
      this.send(frame(TAG.range, wire));
    }
  }

  private onWant(payload: Uint8Array): void {
    const ids = parseJson(payload) as string[];
    if (!Array.isArray(ids) || ids.length > this.limits.maxWantIds) {
      return this.fail("want_too_large", `a want may name at most ${this.limits.maxWantIds} ids`);
    }
    const held: Uint8Array[] = [];
    for (const id of ids) {
      const event = this.opts.store.heldEvent(id);
      if (event) held.push(event.bytes);
    }
    this.sendBatches(held);
    this.send(frame(TAG.wantEnd, new Uint8Array(0)));
    // Answering done: now the store asks for what it learned it is
    // missing (bounded like any want). The client answers with
    // batches and its own WANT_END, then closes with DONE.
    const missing = [...this.learned].filter((id) => !this.opts.store.heldEvent(id));
    this.send(jsonFrame(TAG.want, missing.slice(0, this.limits.maxWantIds)));
  }

  private sendBatches(events: Uint8Array[]): void {
    let batches: Uint8Array[][];
    try {
      batches = splitBatches(events, this.limits);
    } catch (e) {
      // A held event that cannot fit any batch under these limits:
      // name it and close, rather than emit a batch the peer's
      // decoder is bound to reject (#14644).
      return this.fail(
        "event_too_large",
        e instanceof Error ? e.message : "an event cannot fit the batch bound",
      );
    }
    for (const batch of batches) {
      this.send(frame(TAG.batch, encodeBatch(batch)));
    }
  }

  private async onBatch(payload: Uint8Array): Promise<void> {
    const rawEvents = decodeBatch(payload, this.limits);
    const entries = await decodeEntries(rawEvents);
    if (this.opts.guardBatch) {
      // The ceiling is checked before anything is written: the
      // bytes this batch would add are the verified events the store
      // does not already hold in any form.
      let newBytes = 0;
      for (const entry of entries) {
        if ("malformed" in entry) continue;
        if (!this.opts.store.heldEvent(entry.id)) newBytes += entry.bytes.length;
      }
      if (!this.opts.guardBatch(newBytes)) {
        return this.fail("storage_ceiling", "the batch would exceed the storage ceiling");
      }
    }
    const outcomes = this.opts.store.ingestBatch(entries, this.now());
    this.send(jsonFrame(TAG.ack, summarizeOutcomes(outcomes)));
  }

  private onClientWantEnd(): void {
    // The client's answer is complete; nothing remains but DONE.
  }

  private onDone(): void {
    if (!this.tokenIssued) {
      this.tokenIssued = true;
      const token = issueResumeToken(this.opts.store, this.opts.bindingId, this.now());
      this.send(jsonFrame(TAG.token, { token }));
    }
    this.send(frame(TAG.done, new Uint8Array(0)));
  }
}

// ----- the reference client (the native side, in TypeScript) ---------

export interface ClientSessionResult {
  token: string | null;
  withholding: Withholding[];
  acks: Record<string, number>[];
  serverHeads: Head[];
}

export interface ClientOptions {
  store: BridgeStore;
  key: IdKey;
  bindingId: string;
  params?: RangeParams;
  limits?: BridgeLimits;
  resumeToken?: string | null;
  now?: () => number;
}

/** Drives one full session over a transport. The transport is a
 * message channel; production native nodes speak the same frames
 * from Rust, and the acceptance tests drive this client against the
 * server session over an in-memory pair (and the DO over a fake
 * socket) — the protocol code is identical either way. */
export class BridgeClient {
  private inbox: (Uint8Array | null)[] = [];
  private waiters: ((data: Uint8Array | null) => void)[] = [];
  private closed = false;

  constructor(
    private transport: FrameTransport,
    private opts: ClientOptions,
  ) {
    transport.onMessage((data) => this.deliver(data));
    transport.onClose(() => {
      this.closed = true;
      this.deliver(null);
    });
  }

  private get params(): RangeParams {
    return this.opts.params ?? DEFAULT_RANGE_PARAMS;
  }

  private get limits(): BridgeLimits {
    return this.opts.limits ?? DEFAULT_BRIDGE_LIMITS;
  }

  private now(): number {
    return this.opts.now ? this.opts.now() : Date.now();
  }

  private deliver(data: Uint8Array | null): void {
    const waiter = this.waiters.shift();
    if (waiter) waiter(data);
    else this.inbox.push(data);
  }

  private async next(): Promise<Uint8Array> {
    const data =
      this.inbox.length > 0
        ? (this.inbox.shift() as Uint8Array | null)
        : await new Promise<Uint8Array | null>((resolve) => this.waiters.push(resolve));
    if (data === null) throw new Error("the transport closed mid-session");
    return data;
  }

  private async nextTag(tag: number): Promise<Uint8Array> {
    const data = await this.next();
    if (data[0] === TAG.error) {
      const body = parseJson(data.subarray(1)) as { code: string; message: string };
      throw new Error(`bridge session error ${body.code}: ${body.message}`);
    }
    if (data[0] !== tag) {
      throw new Error(`expected tag ${tag}, got ${data[0]}`);
    }
    return data.subarray(1);
  }

  async sync(): Promise<ClientSessionResult> {
    const store = this.opts.store;
    // AUTH: answer the server's challenge with the ID-key proof.
    const challenge = await this.nextTag(TAG.challenge);
    const nonce = new TextDecoder().decode(challenge);
    const transcriptBytes = authTranscript(this.opts.bindingId, nonce, "ws", bodyHashHex(new Uint8Array(0)));
    const signature = await this.opts.key.sign(transcriptBytes);
    this.transport.send(jsonFrame(TAG.auth, { nonce, signature: bytesToHex(signature) }));

    // HELLO or RESUME.
    let serverHeads: Head[];
    if (this.opts.resumeToken) {
      const positions = store.heads();
      this.transport.send(jsonFrame(TAG.resume, { token: this.opts.resumeToken, positions }));
      const resumed = parseJson(await this.nextTag(TAG.resumed)) as { positions: Head[] };
      serverHeads = resumed.positions;
    } else {
      this.transport.send(jsonFrame(TAG.hello, store.heads()));
      serverHeads = parseJson(await this.nextTag(TAG.hello)) as Head[];
    }

    // RANGE: ping-pong flights until both sides have sent all-skip.
    const learned = new Set<string>();
    const authored: { author: string; id: string }[] = [];
    let spent = 0;
    let myParty = new RangeParty(store.syncEntries());
    let msg: RangeEntry[] = [myParty.fullRange()];
    let sentSkip = false;
    let receivedSkip = false;
    for (let round = 0; round < 64 && !(sentSkip && receivedSkip); round++) {
      for (const wire of frameFlight(msg, this.params.frameBudget)) {
        this.transport.send(frame(TAG.range, wire));
      }
      sentSkip = allSkip(msg);
      // Read one flight back.
      const flightFrames: Uint8Array[] = [];
      for (;;) {
        const payload = await this.nextTag(TAG.range);
        spent += payload.length + frameEntryCount(payload) * ENTRY_CHARGE;
        if (spent > this.params.sessionBudget) throw new Error("range session budget exceeded");
        flightFrames.push(payload);
        if (!decodeFrame(payload).more) break;
      }
      const flight = decodeFlight(flightFrames);
      receivedSkip = allSkip(flight);
      if (receivedSkip && sentSkip) break;
      if (receivedSkip) {
        msg = [{ upper: null, mode: { t: "skip" } }];
        continue;
      }
      myParty = new RangeParty(store.syncEntries());
      msg = myParty.respond(flight, this.params, learned);
    }

    // WANT: ask for everything learned that the store does not hold.
    const missing = [...learned].filter((id) => !store.heldEvent(id));
    this.transport.send(jsonFrame(TAG.want, missing.slice(0, this.limits.maxWantIds)));
    const acks: Record<string, number>[] = [];
    const received = new Set<string>();
    for (;;) {
      const data = await this.next();
      if (data[0] === TAG.batch) {
        const rawEvents = decodeBatch(data.subarray(1), this.limits);
        const entries = await decodeEntries(rawEvents);
        for (const entry of entries) {
          if (!("malformed" in entry)) {
            received.add(entry.id);
            authored.push({ author: entry.authorHex, id: entry.id });
          }
        }
        const outcomes = store.ingestBatch(entries, this.now());
        acks.push(summarizeOutcomes(outcomes));
      } else if (data[0] === TAG.wantEnd) {
        break;
      } else if (data[0] === TAG.error) {
        const body = parseJson(data.subarray(1)) as { code: string; message: string };
        throw new Error(`bridge session error ${body.code}: ${body.message}`);
      } else {
        throw new Error(`unexpected tag ${data[0]} during transfer`);
      }
    }

    // The server's WANT: answer with batches, then WANT_END.
    const serverWant = parseJson(await this.nextTag(TAG.want)) as string[];
    const answer: Uint8Array[] = [];
    for (const id of serverWant) {
      const held = store.heldEvent(id);
      if (held) answer.push(held.bytes);
    }
    // Batches are cut on the encoded size (splitBatches throws
    // on an event that cannot fit any batch under these limits).
    for (const batch of splitBatches(answer, this.limits)) {
      this.transport.send(frame(TAG.batch, encodeBatch(batch)));
    }
    this.transport.send(frame(TAG.wantEnd, new Uint8Array(0)));
    // The server's ACK for our answer batches may arrive; drain one
    // ack per batch sent (acks are informational for the client).
    // (The server sends its DONE only after our DONE below, so any
    // pending ack is read opportunistically before DONE.)
    this.transport.send(frame(TAG.done, new Uint8Array(0)));
    let token: string | null = null;
    for (;;) {
      const data = await this.next();
      if (data[0] === TAG.token) {
        token = (parseJson(data.subarray(1)) as { token: string }).token;
      } else if (data[0] === TAG.done) {
        break;
      } else if (data[0] === TAG.ack) {
        continue;
      } else if (data[0] === TAG.error) {
        const body = parseJson(data.subarray(1)) as { code: string; message: string };
        throw new Error(`bridge session error ${body.code}: ${body.message}`);
      }
    }

    const stillMissing = new Set(missing.filter((id) => !received.has(id) && !store.heldEvent(id)));
    const withholding = detectWithholding(serverHeads, store, authored, stillMissing);
    return { token, withholding, acks, serverHeads };
  }
}

/** Retry with backoff around `sync`: a dropped session resumes with
 * the last token when one exists. Positions come from the store, so
 * a retry never re-sends what already landed — re-sent events are
 * duplicates by construction. */
export async function syncWithRetry(
  makeTransport: () => FrameTransport,
  opts: ClientOptions,
  retry: { attempts: number; baseDelayMs: number; sleep?: (ms: number) => Promise<void> },
): Promise<ClientSessionResult> {
  const sleep = retry.sleep ?? ((ms: number) => new Promise((r) => setTimeout(r, ms)));
  let token = opts.resumeToken ?? null;
  let lastError: unknown = null;
  for (let attempt = 0; attempt < retry.attempts; attempt++) {
    try {
      const client = new BridgeClient(makeTransport(), { ...opts, resumeToken: token });
      const result = await client.sync();
      return result;
    } catch (e) {
      lastError = e;
      // A rejected resume is not a failure of the sync itself: the
      // token is stale, so the next attempt opens with HELLO and
      // re-derives everything through the range exchange.
      if (e instanceof Error && e.message.includes("resume_rejected")) token = null;
      if (attempt + 1 < retry.attempts) {
        await sleep(retry.baseDelayMs * 2 ** attempt);
      }
    }
  }
  throw lastError;
}
