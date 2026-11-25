// Hosted read-only decision activity pages (#14967). The caller must verify
// the owner BEFORE calling this handler.
export interface DecisionPagesIo {
  secret?: string;
  read: (request: Request) => Promise<Response>;
  now: () => number;
}

const ORIGIN = "https://decision-events.internal";
const PAGE_SIZE = 200;

function escapeHtml(value: unknown): string {
  return String(value).replace(/[&<>"']/g, (char) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]!);
}

function page(body: string, status = 200): Response {
  return new Response(`<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Decisions — PM Stack</title>
<style>body{font:16px/1.5 system-ui,sans-serif;background:#0b0f14;color:#e6eaf0;max-width:58rem;margin:5vh auto;padding:0 1rem}a{color:#79c0ff}.context{color:#9da7b1}.item{border:1px solid #30363d;border-radius:.6rem;padding:1rem;margin:.8rem 0;background:#11161d}code{overflow-wrap:anywhere}</style></head><body><main>${body}</main></body></html>`, {
    status,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
      "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
      "X-Content-Type-Options": "nosniff",
      "Referrer-Policy": "no-referrer",
    },
  });
}

function unavailable(): Response {
  return page(`<h1>Decision activity unavailable</h1><p class="context">The D1 decision-events read did not answer. No empty or stale result is being shown.</p>`, 503);
}

function validSetId(value: string): boolean {
  return value.length > 0 && value.length <= 128 && !/[\u0000-\u0020\u007f]/.test(value);
}

function validCursor(value: string): boolean {
  const sep = value.indexOf("|");
  const ts = sep < 0 ? value : value.slice(0, sep);
  const eventId = sep < 0 ? "" : value.slice(sep + 1);
  return !Number.isNaN(Date.parse(ts)) && ts.includes("T") && (sep < 0 || (/^\S{1,128}$/.test(eventId)));
}

async function readJson(request: Request, io: DecisionPagesIo): Promise<unknown> {
  if (!io.secret) throw new Error("missing server credential");
  const headers = new Headers({ Authorization: `Bearer ${io.secret}`, Accept: "application/json" });
  const response = await io.read(new Request(request, { headers, method: "GET", redirect: "error" }));
  if (!response.ok) throw new Error("decision-events read failed");
  return response.json();
}

function readPath(path: string): string {
  return encodeURIComponent(path);
}

function ageLabel(at: string, now: number): string {
  const seconds = Math.max(0, Math.floor((now - Date.parse(at)) / 1000));
  if (seconds <= 60) return `${seconds} seconds`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"}`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"}`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"}`;
}

export async function decisionPage(request: Request, io: DecisionPagesIo): Promise<Response | null> {
  const url = new URL(request.url);
  if (request.method !== "GET" || (url.pathname !== "/decisions" && !/^\/decisions\/[^/]+$/.test(url.pathname))) return null;
  const querySetId = url.pathname === "/decisions" ? url.searchParams.get("set") : null;
  if (url.pathname === "/decisions" && querySetId === null) {
    try {
      const data = await readJson(new Request(`${ORIGIN}/sets?limit=${PAGE_SIZE}`), io) as {
        sets?: Array<{ setId?: unknown; events?: unknown; firstTs?: unknown; lastTs?: unknown; lastType?: unknown }>;
      };
      if (!Array.isArray(data?.sets) || data.sets.some((s) => !s || typeof s.setId !== "string" || !validSetId(s.setId) ||
        !Number.isFinite(s.events) || Number(s.events) < 1 || typeof s.lastTs !== "string" || Number.isNaN(Date.parse(s.lastTs)) || typeof s.lastType !== "string")) return unavailable();
      const fetchedAt = new Date(io.now()).toISOString();
      const rows = data.sets.map((set) => `<div class="item"><h2><a href="https://pm.security.cab/decisions?set=${encodeURIComponent(set.setId as string)}">Decision set ${escapeHtml(set.setId)}</a></h2>
<p class="context">${escapeHtml(set.events)} events · latest ${escapeHtml(set.lastType)} at ${escapeHtml(set.lastTs)} (${escapeHtml(ageLabel(set.lastTs as string, io.now()))} ago)</p></div>`).join("");
      return page(`<h1>Decision activity</h1><p class="context">Read from D1 decision-events at ${escapeHtml(fetchedAt)}. Showing at most ${PAGE_SIZE} recent sets, not a complete index. Activity is current as of each event time; question definitions are not in this event feed.</p>
${rows || '<p class="context">No decision-event activity is recorded.</p>'}`);
    } catch {
      return unavailable();
    }
  }

  let setId: string;
  try { setId = querySetId ?? decodeURIComponent(url.pathname.slice("/decisions/".length)); } catch { return page("<h1>Invalid decision set</h1>", 400); }
  if (!validSetId(setId)) return page("<h1>Invalid decision set</h1>", 400);
  const after = url.searchParams.get("after");
  if (after !== null && (!validCursor(after) || after.length > 256)) return page("<h1>Invalid page cursor</h1>", 400);
  const query = new URLSearchParams({ limit: String(PAGE_SIZE) });
  if (after) query.set("after", after);
  try {
    const data = await readJson(new Request(`${ORIGIN}/sets/${readPath(setId)}/events?${query}`), io) as {
      setId?: unknown; events?: Array<{ eventId?: unknown; ts?: unknown; type?: unknown; questionRef?: unknown }>;
      nextAfter?: unknown;
    };
    if (data?.setId !== setId || !Array.isArray(data.events) || data.events.some((e) => !e || typeof e.eventId !== "string" ||
      typeof e.ts !== "string" || Number.isNaN(Date.parse(e.ts)) || typeof e.type !== "string" ||
      (e.questionRef !== null && e.questionRef !== undefined && typeof e.questionRef !== "string")) ||
      (data.nextAfter !== null && data.nextAfter !== undefined && (typeof data.nextAfter !== "string" || !validCursor(data.nextAfter)))) return unavailable();
    const fetchedAt = new Date(io.now()).toISOString();
    const rows = data.events.map((event) => `<div class="item"><h2>${escapeHtml(event.type)}</h2><p class="context">${escapeHtml(event.ts)} · event <code>${escapeHtml(event.eventId)}</code>${event.questionRef ? ` · question <code>${escapeHtml(event.questionRef)}</code>` : ""}</p></div>`).join("");
    const next = typeof data.nextAfter === "string"
      ? `<p><a href="https://pm.security.cab/decisions?set=${encodeURIComponent(setId)}&amp;after=${encodeURIComponent(data.nextAfter)}">Next page</a></p>` : "";
    return page(`<p><a href="https://pm.security.cab/decisions">All decision activity</a></p><h1>Decision set ${escapeHtml(setId)}</h1>
<p class="context">Read from D1 decision-events at ${escapeHtml(fetchedAt)}. Question definitions are not in this event feed; decisions, choices, answers, and write actions are not exposed by this activity-only page.</p>
<p class="context">${escapeHtml(data.events.length)} events on this page.${after ? ` After <code>${escapeHtml(after)}</code>.` : ""}</p>
${rows || '<p class="context">No events on this page.</p>'}${next}`);
  } catch {
    return unavailable();
  }
}
