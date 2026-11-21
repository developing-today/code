import { Unpackr } from "msgpackr";
import type { ScreenPatch } from "./world-surface";

export type BinaryViewPatchFrame = {
  type: "view";
  sequence: number;
  view_revision: number;
  base_view_revision: number;
  resume_token?: string;
  patch: ScreenPatch;
};

const MAX_BINARY_VIEW_FRAME_BYTES = 1024 * 1024;
const unpackr = new Unpackr({ useRecords: false });

/** Decode and validate the v2 MessagePack envelope before applying a patch. */
export function decodeBinaryViewPatch(bytes: ArrayBuffer | Uint8Array): BinaryViewPatchFrame | null {
  if (bytes.byteLength > MAX_BINARY_VIEW_FRAME_BYTES) return null;
  try {
    const decoded: unknown = unpackr.unpack(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
    if (!isRecord(decoded) || decoded.version !== 1 || decoded.type !== "view") return null;
    if (
      !Number.isSafeInteger(decoded.sequence) ||
      !Number.isSafeInteger(decoded.view_revision) ||
      !Number.isSafeInteger(decoded.base_view_revision) ||
      !isRecord(decoded.patch) ||
      (decoded.resume_token !== undefined && typeof decoded.resume_token !== "string")
    ) {
      return null;
    }
    return {
      type: "view",
      sequence: decoded.sequence as number,
      view_revision: decoded.view_revision as number,
      base_view_revision: decoded.base_view_revision as number,
      ...(typeof decoded.resume_token === "string" ? { resume_token: decoded.resume_token } : {}),
      patch: decoded.patch as unknown as ScreenPatch,
    };
  } catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
