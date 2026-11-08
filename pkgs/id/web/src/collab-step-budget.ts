import { Packr } from "msgpackr";

/**
 * Per-message step bound, mirroring MAX_STEPS_PER_MESSAGE in
 * pkgs/id/src/web/collab.rs. A transaction may exceed this count, so the
 * client flushes remainder steps as the server confirms each batch.
 */
export const MAX_STEPS_PER_MESSAGE = 1000;

/**
 * Wire-size budget for one Steps message, mirroring the server frame cap
 * MAX_FRAME_BYTES (1 MiB, pkgs/id/src/frames.rs) minus a reserve for the
 * MessagePack envelope. Both editor-side splitting and outbound batching
 * use this shared bound.
 */
export const MAX_STEPS_MESSAGE_BYTES = 1024 * 1024 - 4096;

const packr = new Packr({ useRecords: false, structuredClone: true });

/** Encoded size of one step on the wire (MessagePack, same config as collab.ts). */
export function stepWireSize(step: unknown): number {
  return packr.pack(step).byteLength;
}
