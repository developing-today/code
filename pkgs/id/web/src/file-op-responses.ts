/** Runtime shape validation for the editor shell's file-operation API responses. */

/**
 * The four file-operation handlers in `main.ts` (POST /api/save,
 * /api/new, /api/rename, /api/copy) used to cast their `fetch` JSON
 * responses with bare `as` casts and use the fields unchecked (#7837).
 * A 200 response with the wrong shape (proxy error page parsed as
 * JSON, a partial deploy, a server-side change) was therefore treated
 * as success: save reported `{ ok: true }` and set
 * `editorContainer.dataset.hash` to the literal string "undefined"
 * (corrupting every later save in the session), and new/rename/copy
 * navigated to `/edit/undefined` after announcing success.
 *
 * These guards are the runtime check the casts replaced. Each handler
 * validates the parsed body before any field is used; on a wrong
 * shape no state is mutated and the failure is reported through the
 * same channel as a non-OK response.
 */

export interface SaveFileResponse {
  hash: string;
  name: string;
  archive_name: string | null;
}

export interface NewFileResponse {
  hash: string;
  name: string;
}

export interface RenameFileResponse {
  name: string;
  hash: string;
  archived_original: string | null;
  archived_replaced: string | null;
}

export interface CopyFileResponse {
  name: string;
  hash: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isNonEmptyString = (value: unknown): value is string =>
  typeof value === "string" && value.length > 0;

const isStringOrNull = (value: unknown): value is string | null =>
  typeof value === "string" || value === null;

export function isSaveFileResponse(value: unknown): value is SaveFileResponse {
  return (
    isRecord(value) &&
    isNonEmptyString(value.hash) &&
    isNonEmptyString(value.name) &&
    isStringOrNull(value.archive_name)
  );
}

export function isNewFileResponse(value: unknown): value is NewFileResponse {
  return isRecord(value) && isNonEmptyString(value.hash) && isNonEmptyString(value.name);
}

export function isRenameFileResponse(value: unknown): value is RenameFileResponse {
  return (
    isRecord(value) &&
    isNonEmptyString(value.name) &&
    isNonEmptyString(value.hash) &&
    isStringOrNull(value.archived_original) &&
    isStringOrNull(value.archived_replaced)
  );
}

export function isCopyFileResponse(value: unknown): value is CopyFileResponse {
  return isRecord(value) && isNonEmptyString(value.name) && isNonEmptyString(value.hash);
}
