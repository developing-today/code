import { SECURITY } from "../../src/http.ts";

// The read-only endpoint is the fleet-platform ledger. This URL matches
// the aggregator's fleet-platform store client (decision-events-sync.ts).
const TURSO_URL = "https://fleet-platform-dezren39.aws-us-east-1.turso.io";
const COLUMNS = ["id", "object_ref", "kind", "author", "body", "created_at", "triage_state", "github_state"] as const;
const PAGE_SIZE = 100;

export interface CommentsSurface {
  enabled: boolean;
  token?: string;
  url?: string;
  fetchImpl?: typeof fetch;
}

type CommentRow = Record<(typeof COLUMNS)[number], string | null>;

function escapeHtml(value: unknown): string {
  const entities: Record<string, string> = {
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  };
  return String(value ?? "").replace(/[&<>"']/g, (char) => entities[char]);
}

function cellValue(cell: any): string | null {
  if (cell?.type === "null") return null;
  if (cell?.type === "text" && typeof cell.value === "string") return cell.value;
  if ((cell?.type === "integer" || cell?.type === "float") && typeof cell.value === "string") return cell.value;
  throw new Error("malformed comments result cell");
}

function decodeRows(payload: any): CommentRow[] {
  const result = payload?.results?.[0]?.response?.result;
  if (payload?.results?.[0]?.type !== "ok" || !Array.isArray(result?.cols) || !Array.isArray(result?.rows)) {
    throw new Error("malformed comments result");
  }
  const names = result.cols.map((col: any) => col?.name);
  if (!COLUMNS.every((column) => names.includes(column))) throw new Error("comments columns missing");
  return result.rows.map((cells: any[]) => {
    if (!Array.isArray(cells) || cells.length !== names.length) throw new Error("malformed comments row");
    const values = Object.fromEntries(cells.map((cell, i) => [names[i], cellValue(cell)]));
    return Object.fromEntries(COLUMNS.map((column) => [column, values[column]])) as CommentRow;
  });
}

function page(rows: CommentRow[], readAt: string, truncated: boolean): string {
  const entries = rows.map((row) => {
    let author = row.author ?? "Unknown author";
    try {
      const parsed = JSON.parse(author);
      if (parsed && typeof parsed.label === "string") author = parsed.label;
    } catch { /* Display the stored value as text, escaped below. */ }
    return `<article><h2>${escapeHtml(row.object_ref)} · ${escapeHtml(row.kind)}</h2><p>${escapeHtml(author)} · ${escapeHtml(row.created_at)}</p><pre>${escapeHtml(row.body)}</pre><p>Triage: ${escapeHtml(row.triage_state ?? "not set")} · GitHub: ${escapeHtml(row.github_state ?? "not set")}</p></article>`;
  }).join("\n");
  return `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Comments</title><main><h1>Comments</h1><p>Read from shared_comments at <time>${escapeHtml(readAt)}</time>.</p><p>Producer freshness unknown; this is the time this page queried the ledger, not when its producer last wrote.</p>${truncated ? `<p>Showing the newest ${PAGE_SIZE} records; more records may exist.</p>` : ""}${rows.length ? entries : "<p>No comments returned by the ledger.</p>"}</main></html>`;
}

function response(body: string, status = 200): Response {
  return new Response(body, { status, headers: {
    ...SECURITY,
    "Content-Type": "text/html; charset=utf-8",
    "Cache-Control": "no-store",
  } });
}

export async function commentsSurface(req: Request, options: CommentsSurface, now: () => number): Promise<Response> {
  if (req.method !== "GET") return response("<!doctype html><title>Method not allowed</title>", 405);
  if (!options.token) return response("<!doctype html><title>Comments unavailable</title><h1>Comments unavailable</h1><p>The comments data source is not configured.</p>", 503);
  try {
    const endpoint = new URL("/v2/pipeline", options.url ?? TURSO_URL);
    if (endpoint.protocol !== "https:" || endpoint.username || endpoint.password) throw new Error("invalid comments endpoint");
    const result = await (options.fetchImpl ?? fetch)(endpoint.href, {
      method: "POST",
      headers: { Authorization: `Bearer ${options.token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ requests: [{ type: "execute", stmt: {
        sql: `SELECT ${COLUMNS.join(", ")} FROM shared_comments ORDER BY created_at DESC LIMIT ${PAGE_SIZE + 1}`,
        args: [], want_rows: true,
      } }, { type: "close" }] }),
      signal: AbortSignal.timeout(10_000),
    });
    if (!result.ok) throw new Error("comments query failed");
    const rows = decodeRows(await result.json());
    return response(page(rows.slice(0, PAGE_SIZE), new Date(now()).toISOString(), rows.length > PAGE_SIZE));
  } catch {
    return response("<!doctype html><title>Comments unavailable</title><main><h1>Comments unavailable</h1><p>The comments ledger could not be read. No empty result is being shown.</p></main>", 503);
  }
}
