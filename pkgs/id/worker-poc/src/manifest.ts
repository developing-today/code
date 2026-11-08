// Hosted-loader manifest checks for the Worker isolate host (#2271, #2057).
// Pre-registered difference (findings note on #2271): the hosted loader
// refuses up front, naming the reason; the native host traps at run time.
import { PlazaGuest, DEFAULT_LIMITS, type GuestLimits } from "./guest.ts";

export const HOSTED_MEMORY_CEILING_BYTES = 64 * 1024 * 1024; // 64MB, #2057 hosted ceiling

export interface HostedManifest {
  name: string;
  portability?: "portable" | "native";
  wasm_memory_bytes?: number;
}

export function assertHostable(manifest: HostedManifest): void {
  if (manifest.portability === "native") {
    throw new Error(
      `manifest ${manifest.name}: portability=native is not hostable in a Worker isolate`,
    );
  }
  if (
    manifest.wasm_memory_bytes !== undefined &&
    manifest.wasm_memory_bytes > HOSTED_MEMORY_CEILING_BYTES
  ) {
    throw new Error(
      `manifest ${manifest.name}: declared memory ${manifest.wasm_memory_bytes} exceeds hosted ceiling ${HOSTED_MEMORY_CEILING_BYTES} (64MB)`,
    );
  }
}

export async function loadHostedGuest(
  manifest: HostedManifest,
  bytes: ArrayBuffer | Uint8Array,
  seed: number | bigint,
  limits: GuestLimits = DEFAULT_LIMITS,
): Promise<PlazaGuest> {
  assertHostable(manifest);
  return PlazaGuest.instantiate(bytes, seed, limits);
}

export async function loadHostedModule(
  manifest: HostedManifest,
  module: WebAssembly.Module,
  seed: number | bigint,
  limits: GuestLimits = DEFAULT_LIMITS,
): Promise<PlazaGuest> {
  assertHostable(manifest);
  return PlazaGuest.fromModule(module, seed, limits);
}
