// Shared encoding helpers for the hosted Workers (G2 of #9085).
//
// One implementation of the hex and base64/base64url idioms previously
// copied across `pkgs/id/hosted-worker` (`src/ed25519.ts`,
// `src/migration-snapshot.ts`) and `pkgs/fleet/hosted-worker`
// (`src/jwt.ts`, `src/stamp-sig.ts`, `src/config.ts`). Workers have no
// Buffer; everything here is `atob`/`btoa` plus typed arrays, so it
// runs unchanged in workerd and Node.

/** Lower-case hex of `bytes`. */
export function bytesToHex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

/** Inverse of {@link bytesToHex}: strict lower-case hex only. */
export function hexToBytes(hex: string): Uint8Array {
  if (!/^(?:[0-9a-f]{2})*$/.test(hex)) throw new Error("not lowercase hex");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** Unpadded base64url of `bytes` (the JWT compact-serialization alphabet). */
export function bytesToBase64Url(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Inverse of {@link bytesToBase64Url}. */
export function base64UrlToBytes(text: string): Uint8Array {
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Standard padded base64 of `bytes`. */
export function bytesToBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

/** Inverse of {@link bytesToBase64}. */
export function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}
