// Shared TypeScript primitives for the hosted Workers (G2 of #9085).
//
// One implementation of BLAKE3 (one-shot and streaming), canonical
// CBOR, and encoding helpers, imported by both `pkgs/id/hosted-worker`
// and `pkgs/fleet/hosted-worker`. Behaviour-preserving extraction:
// no wire formats change; the G1 shared fixtures remain the oracle.

export { Blake3Hasher, blake3 } from "./blake3.ts";
export {
  CborError,
  MAX_NESTING_DEPTH,
  decodeCanonical,
  encodeCanonical,
  validateRecord,
  type CborErrorCode,
  type CborValue,
  type RecordSpec,
} from "./canonical-cbor.ts";
export {
  base64ToBytes,
  base64UrlToBytes,
  bytesToBase64,
  bytesToBase64Url,
  bytesToHex,
  hexToBytes,
} from "./encoding.ts";
