/**
 * Validating parser for the GET /api/tags response (#7841).
 *
 * The endpoint may return a flat array of tags or an object carrying a
 * `tags` array. Anything else — a wrong-shape 200, a null body, entries
 * without a string `key`, entries whose `value` is not a string or
 * null — is a contract violation, and this parser fails closed by
 * returning null instead of handing the renderer data it would have to
 * guess at. Callers must treat null as a load failure (an explicit
 * error state), never as "the file has no tags": before this parser
 * existed, loadFileTags shape-trusted the body, so a wrong-shape 200
 * rendered the file as untagged, left the previous file's pills on
 * screen, or rendered coerced "[object Object]" pill labels.
 */
export interface FileTag {
  key: string;
  value: string | null;
}

export function parseTagsResponse(raw: unknown): FileTag[] | null {
  const list: unknown = Array.isArray(raw)
    ? raw
    : typeof raw === "object" && raw !== null
      ? (raw as { tags?: unknown }).tags
      : undefined;
  if (!Array.isArray(list)) return null;
  const tags: FileTag[] = [];
  for (const entry of list) {
    if (typeof entry !== "object" || entry === null) return null;
    const { key, value } = entry as { key?: unknown; value?: unknown };
    if (typeof key !== "string") return null;
    if (value !== undefined && value !== null && typeof value !== "string") return null;
    tags.push({ key, value: value ?? null });
  }
  return tags;
}
