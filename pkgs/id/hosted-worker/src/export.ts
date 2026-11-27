// §6 export production (spec §6.1 items 1–2; #2852 slice 4b,
// #13157): enumerate a per-user DO's applied signed objects and the
// R2 blobs they cite, as the object/blob sets `build_export`
// (pkgs/id/src/hosted_export.rs) assembles into a `HostedExport`.
// The binding record, its rebind records, and the manifest live on
// the binding-service side; this module is the data-plane half —
// everything the DO itself can produce, as a pure read. Export
// therefore stays available from a read-only, at-cap store (§5.4):
// nothing here writes.
//
// The object-model mapping this wiring settles (spec §6.3):
//
// - `bytes` is the stored envelope bytes verbatim. The bridge never
//   re-encodes or re-signs a stored event; the export doesn't
//   either.
// - `hash` is SHA-256 of the envelope bytes: the export-layer
//   digest that canonical (hash) order and the manifest's
//   concatenated-hash digest are computed over. The envelope's own
//   BLAKE3 id remains its identity at the object layer.
// - `signer` is the envelope's author key and `signature` is the
//   envelope's own signature, over the canonical header encoding —
//   the object's own signature scheme, verified at import by
//   decoding the envelope exactly as chain.rs `decode` does. The
//   producer never re-signs: it holds no key it could sign with.
// - Payload bodies are not held by this store (the bridge keeps
//   headers; a header's payload_hash commits to off-store data)
//   and are not §6 blobs of this store — document content is the
//   blob path, mapped as follows.
// - Blob citations: a `document.put` envelope cites its content by
//   BLAKE3 (`DocumentRef.content`, the native blob identity), while
//   this store's R2 blobs are SHA-256-addressed (the slice-A
//   reference table). Citations are resolved by content: a fetched
//   blob whose BLAKE3 equals the citation is the cited blob, and
//   the export names it by its SHA-256 hash (§6 form). A citation
//   no referenced blob satisfies — or a referenced blob R2 cannot
//   produce — fails the export loudly, naming the hash; an export
//   is never silently incomplete. Blobs no applied object cites
//   are not exported (§6.1 item 2, §6.2's unreferenced-blob rule).

import { decodeCanonical, type CborValue } from "../../../hosted-primitives/src/canonical-cbor.ts";
import { blake3 } from "../../../hosted-primitives/src/blake3.ts";
import { bytesToHex } from "./ed25519.ts";
import { parseEvent } from "./event.ts";
import type { HeldEvent } from "./bridge-store.ts";

/** One §6 object (§6.1 item 1) in the `ExportObject` wire shape,
 * with bytes still binary; `exportSetsToWire` renders the JSON
 * form serde expects on the Rust side. */
export interface ExportObjectSet {
  hash: string;
  signer: string;
  signature: string;
  blob_refs: string[];
  bytes: Uint8Array;
}

/** One §6 blob (§6.1 item 2) in the `ExportBlob` wire shape. */
export interface ExportBlobSet {
  hash: string;
  bytes: Uint8Array;
}

/** The object/blob sets of a §6 export, in canonical (hash)
 * order, with the manifest's counts and concatenated-hash digest
 * (§6.1 item 4) computed over exactly these sets. */
export interface ExportSets {
  objects: ExportObjectSet[];
  blobs: ExportBlobSet[];
  object_count: number;
  blob_count: number;
  objects_digest: string;
}

/** A named export-production failure. `hash` is the hash the
 * failure names: the cited content hash (BLAKE3) an object needs
 * and no referenced blob produces, or the reference hash (SHA-256)
 * whose stored bytes R2 cannot produce or cannot verify. */
export class ExportProductionError extends Error {
  readonly hash: string;
  constructor(hash: string, message: string) {
    super(message);
    this.hash = hash;
  }
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return bytesToHex(new Uint8Array(digest));
}

function mapGet(map: CborValue, key: string): CborValue | undefined {
  if (map.kind !== "map") return undefined;
  for (const [k, v] of map.value) {
    if (k.kind === "text" && k.value === key) return v;
  }
  return undefined;
}

/** The BLAKE3 content hashes an envelope cites: the `content` of
 * its `document` payload (`document.put`), and nothing else — in
 * the current `ChainExt` form one envelope cites at most one
 * content. The envelope was verified at ingest; a document payload
 * that does not carry a 32-byte content hash is corruption, and is
 * refused as such rather than silently uncited. */
export function citedContentHashes(bytes: Uint8Array, eventId: string): string[] {
  const root = decodeCanonical(bytes);
  if (root.kind !== "array" || root.value.length === 0) {
    throw new ExportProductionError(eventId, `stored event ${eventId} is not a header array`);
  }
  const ext = root.value[root.value.length - 1];
  const document = mapGet(ext, "document");
  if (document === undefined) return [];
  const content = mapGet(document, "content");
  if (!content || content.kind !== "bytes" || content.value.length !== 32) {
    throw new ExportProductionError(
      eventId,
      `stored event ${eventId} carries a document payload with no 32-byte content hash`,
    );
  }
  return [bytesToHex(content.value)];
}

/** Produce the §6 object/blob sets from a store enumeration.
 *
 * `events` is the store's applied events (`BridgeStore
 * .appliedEvents()`); `blobRefs` is the user's blob reference
 * table (SHA-256 hashes, the slice-A access path); `fetchBlob`
 * reads one blob's bytes from R2 by its reference hash, returning
 * null when R2 cannot produce it. */
export async function produceExportSets(
  events: HeldEvent[],
  blobRefs: { hash: string }[],
  fetchBlob: (hash: string) => Promise<Uint8Array | null>,
): Promise<ExportSets> {
  // Fetch and verify every referenced blob first: the export is
  // assembled only from bytes that hash to their reference.
  const blobsBySha = new Map<string, ExportBlobSet>();
  const shaByContent = new Map<string, string>();
  for (const ref of blobRefs) {
    const bytes = await fetchBlob(ref.hash);
    if (bytes === null) {
      throw new ExportProductionError(
        ref.hash,
        `blob ${ref.hash} is referenced by this store but R2 cannot produce it`,
      );
    }
    const actual = await sha256Hex(bytes);
    if (actual !== ref.hash) {
      throw new ExportProductionError(
        ref.hash,
        `blob bytes fetched for ${ref.hash} hash to ${actual}; the stored blob is corrupt`,
      );
    }
    blobsBySha.set(ref.hash, { hash: ref.hash, bytes });
    shaByContent.set(bytesToHex(blake3(bytes)), ref.hash);
  }

  const objects: ExportObjectSet[] = [];
  const citedShas = new Set<string>();
  for (const held of events) {
    const event = parseEvent(held.bytes);
    const refs: string[] = [];
    for (const cited of citedContentHashes(held.bytes, held.id)) {
      const sha = shaByContent.get(cited);
      if (sha === undefined) {
        throw new ExportProductionError(
          cited,
          `object ${held.id} cites content ${cited} but no referenced blob produces it`,
        );
      }
      refs.push(sha);
      citedShas.add(sha);
    }
    refs.sort();
    objects.push({
      hash: await sha256Hex(held.bytes),
      signer: event.authorHex,
      signature: event.signatureHex,
      blob_refs: [...new Set(refs)],
      bytes: held.bytes,
    });
  }
  objects.sort((a, b) => (a.hash < b.hash ? -1 : a.hash > b.hash ? 1 : 0));

  const blobs = [...citedShas].map((sha) => blobsBySha.get(sha) as ExportBlobSet);
  blobs.sort((a, b) => (a.hash < b.hash ? -1 : a.hash > b.hash ? 1 : 0));

  // §6.1 item 4: SHA-256 of the concatenated object hash bytes,
  // objects in canonical (hash) order — the same computation
  // hosted_export.rs performs over the received set.
  const concatenated = new Uint8Array(objects.length * 32);
  objects.forEach((object, i) => {
    for (let j = 0; j < 32; j++) {
      concatenated[i * 32 + j] = parseInt(object.hash.slice(j * 2, j * 2 + 2), 16);
    }
  });
  const objects_digest = await sha256Hex(concatenated);

  return {
    objects,
    blobs,
    object_count: objects.length,
    blob_count: blobs.length,
    objects_digest,
  };
}

/** The JSON wire form of the sets: byte fields as number arrays,
 * exactly the serde shape of `ExportObject` / `ExportBlob` in
 * hosted_export.rs, so the binding-service side can feed the sets
 * to `build_export` without a translation layer. */
export function exportSetsToWire(sets: ExportSets): {
  objects: { hash: string; signer: string; signature: string; blob_refs: string[]; bytes: number[] }[];
  blobs: { hash: string; bytes: number[] }[];
  object_count: number;
  blob_count: number;
  objects_digest: string;
} {
  return {
    objects: sets.objects.map((o) => ({
      hash: o.hash,
      signer: o.signer,
      signature: o.signature,
      blob_refs: o.blob_refs,
      bytes: [...o.bytes],
    })),
    blobs: sets.blobs.map((b) => ({ hash: b.hash, bytes: [...b.bytes] })),
    object_count: sets.object_count,
    blob_count: sets.blob_count,
    objects_digest: sets.objects_digest,
  };
}
