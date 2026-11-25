/**
 * Web client for the documents/document session frames (#14923, the
 * D1-040 web remainder of #3762; frames landed server-side in PR #7678).
 *
 * The server contract (pkgs/id/contract/shell-contract.v1.json):
 * - ClientFrame::Documents { capability, prefix?, after? } is answered by
 *   ServerFrame::Documents { names: string[], next_after: string? } — a
 *   point-in-time snapshot page (deliberately no sequence); `next_after`
 *   is an exclusive cursor: pass it back as `after` for the next page,
 *   and a null/absent `next_after` ends the listing.
 * - ClientFrame::Document { capability, name } is answered by
 *   ServerFrame::Document { name, size, text } — the read-only text of
 *   one public document.
 * - Refusals ride the session error frame ({ type: "error", message })
 *   with the server's DocumentError strings (world_session/documents.rs)
 *   or "join denied" when the capability is not a participant's.
 *
 * Everything here fails closed like its sibling parsers (tags-response,
 * world-http): a wrong-shape reply parses to null instead of data the
 * panel would have to guess at, and an error frame is surfaced as a
 * DocumentsRefusal carrying the server's exact message, never rendered
 * as an empty listing or an empty document.
 */

export interface DocumentsPage {
  names: string[];
  nextAfter: string | null;
}

export interface DocumentRead {
  name: string;
  size: number;
  text: string;
}

/** The refusal messages the session layer sends for document frames. */
export const DOCUMENT_REFUSALS = {
  joinDenied: "join denied",
  invalidName: "document name must be a public .txt, .md, or .markdown tag",
  missing: "document not found",
  tooLarge: "document exceeds the 64 KiB SSH viewing limit",
  binary: "document is not valid UTF-8 text",
  unavailable: "document unavailable",
} as const;

export type DocumentRefusalKind = keyof typeof DOCUMENT_REFUSALS;

export class DocumentsRefusal extends Error {
  readonly kind: DocumentRefusalKind | "error";

  constructor(message: string) {
    super(message);
    this.name = "DocumentsRefusal";
    this.kind =
      (Object.entries(DOCUMENT_REFUSALS).find(([, text]) => text === message)?.[0] as
        | DocumentRefusalKind
        | undefined) ?? "error";
  }
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** Build the first (or a subsequent) documents list frame. */
export function documentsFrame(
  capability: string,
  options: { prefix?: string | null; after?: string | null } = {},
): Record<string, unknown> {
  const frame: Record<string, unknown> = { type: "documents", capability };
  if (options.prefix != null) frame.prefix = options.prefix;
  if (options.after != null) frame.after = options.after;
  return frame;
}

/** Build the single-document read frame. */
export function documentFrame(capability: string, name: string): Record<string, unknown> {
  return { type: "document", capability, name };
}

/**
 * If `raw` is a session error frame, return its refusal; otherwise null.
 * A present-but-non-string message is a contract violation, not a
 * refusal the panel may display coerced — it also returns a refusal,
 * with a fixed local message.
 */
export function documentsRefusal(raw: unknown): DocumentsRefusal | null {
  if (!isRecord(raw) || raw.type !== "error") return null;
  if (typeof raw.message !== "string") {
    return new DocumentsRefusal("documents request failed: malformed error frame");
  }
  return new DocumentsRefusal(raw.message);
}

/** Parse a ServerFrame::Documents reply; null on any shape violation. */
export function parseDocumentsPage(raw: unknown): DocumentsPage | null {
  if (!isRecord(raw) || raw.type !== "documents") return null;
  if (!Array.isArray(raw.names) || raw.names.some((name) => typeof name !== "string")) return null;
  if (raw.next_after !== undefined && raw.next_after !== null && typeof raw.next_after !== "string") return null;
  return { names: [...raw.names], nextAfter: raw.next_after ?? null };
}

/** Parse a ServerFrame::Document reply; null on any shape violation. */
export function parseDocumentRead(raw: unknown): DocumentRead | null {
  if (!isRecord(raw) || raw.type !== "document") return null;
  if (typeof raw.name !== "string") return null;
  if (typeof raw.size !== "number" || !Number.isFinite(raw.size) || raw.size < 0) return null;
  if (typeof raw.text !== "string") return null;
  return { name: raw.name, size: raw.size, text: raw.text };
}

/** One page request: send a documents frame for `after`, return the raw reply frame. */
export type DocumentsPageRequest = (after: string | null) => Promise<unknown>;

/** Safety bound so a misbehaving peer cannot page this client forever. */
export const MAX_DOCUMENT_PAGES = 512;

/**
 * Collect a complete listing by following `next_after` pages from the
 * first page. Throws DocumentsRefusal on an error frame, and Error on a
 * malformed reply, a non-advancing cursor, or the page bound — the
 * caller renders none of those as a (partial) listing.
 */
export async function collectDocuments(requestPage: DocumentsPageRequest): Promise<string[]> {
  const names: string[] = [];
  let after: string | null = null;
  for (let page = 0; page < MAX_DOCUMENT_PAGES; page++) {
    const raw = await requestPage(after);
    const refusal = documentsRefusal(raw);
    if (refusal) throw refusal;
    const parsed = parseDocumentsPage(raw);
    if (!parsed) throw new Error("documents request failed: malformed documents frame");
    names.push(...parsed.names);
    if (parsed.nextAfter === null) return names;
    if (parsed.nextAfter === after) {
      throw new Error("documents request failed: listing cursor did not advance");
    }
    after = parsed.nextAfter;
  }
  throw new Error(`documents request failed: listing exceeded ${MAX_DOCUMENT_PAGES} pages`);
}

/** Read one document through a raw-reply request; refusals throw DocumentsRefusal. */
export async function readDocument(
  request: (name: string) => Promise<unknown>,
  name: string,
): Promise<DocumentRead> {
  const raw = await request(name);
  const refusal = documentsRefusal(raw);
  if (refusal) throw refusal;
  const parsed = parseDocumentRead(raw);
  if (!parsed) throw new Error("document request failed: malformed document frame");
  return parsed;
}
