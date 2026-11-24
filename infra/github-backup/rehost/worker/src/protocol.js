// Pure git smart-HTTP (protocol v0) helpers for the fleet-git-rehost
// Worker (#8584). No I/O lives here: index.js feeds these functions the
// refs document the publisher wrote to R2 and the request bodies git
// sends, which keeps every protocol decision unit-testable.
//
// The serving model is deliberately narrow (see ../README.md): the
// publisher pre-generates ONE self-contained pack holding every object
// in the repo, so upload-pack never negotiates deltas against client
// haves. The Worker advertises the published refs, validates that each
// want names a published object, answers NAK, and streams the pack.
// A client with haves simply receives objects it already has again —
// correct, just not incremental.

export const FLUSH = "0000";

const encoder = new TextEncoder();

/** One pkt-line: 4-hex-digit byte length (prefix included) + payload. */
export function pktLine(payload) {
  const len = encoder.encode(payload).length + 4;
  if (len > 0xffff) throw new Error("pkt-line too large");
  return len.toString(16).padStart(4, "0") + payload;
}

/**
 * The info/refs advertisement for one published repo.
 * doc: {head, head_ref, refs: [{name, oid, peeled}]} as written by
 * rehost_publish.py. Only ofs-delta is advertised (the publisher
 * generates packs with --delta-base-offset) plus the HEAD symref so a
 * clone checks out the default branch. No side-band, no multi_ack:
 * the response to an upload-pack request is a bare NAK pkt-line
 * followed by the raw pack stream.
 */
export function buildAdvertisement(doc) {
  const caps = `ofs-delta symref=HEAD:${doc.head_ref} agent=fleet-git-rehost/1.0`;
  let out = pktLine("# service=git-upload-pack\n") + FLUSH;
  out += pktLine(`${doc.head} HEAD\0${caps}\n`);
  for (const ref of doc.refs) {
    out += pktLine(`${ref.oid} ${ref.name}\n`);
    if (ref.peeled) out += pktLine(`${ref.peeled} ${ref.name}^{}\n`);
  }
  return out + FLUSH;
}

/** Split a pkt-line stream into payloads; flush packets become null. */
export function parsePktLines(text) {
  const lines = [];
  let i = 0;
  while (i < text.length) {
    const hex = text.slice(i, i + 4);
    if (!/^[0-9a-fA-F]{4}$/.test(hex)) throw new Error("malformed pkt-line length");
    const len = parseInt(hex, 16);
    if (len === 0) {
      lines.push(null);
      i += 4;
      continue;
    }
    if (len < 4 || i + len > text.length) throw new Error("malformed pkt-line length");
    lines.push(text.slice(i + 4, i + len));
    i += len;
  }
  return lines;
}

/**
 * Parse a protocol-v0 upload-pack request body.
 * Returns {wants, haves, done}. Capabilities ride on the first want
 * line after a NUL and are stripped. deepen/shallow lines are ignored
 * by design: the pack always carries full history, so a shallow
 * request still gets a complete (working) clone.
 */
export function parseUploadPackRequest(text) {
  const wants = [];
  const haves = [];
  let done = false;
  for (const raw of parsePktLines(text)) {
    if (raw === null) continue;
    const line = raw.replace(/\n$/, "").split("\0")[0];
    if (line.startsWith("want ")) wants.push(line.slice(5).trim());
    else if (line.startsWith("have ")) haves.push(line.slice(5).trim());
    else if (line === "done") done = true;
  }
  return { wants, haves, done };
}

/** Every object id a client may legitimately want from this repo. */
export function knownOids(doc) {
  const set = new Set([doc.head]);
  for (const ref of doc.refs) {
    set.add(ref.oid);
    if (ref.peeled) set.add(ref.peeled);
  }
  return set;
}
