// TS CLI query verbs (#12587; design §3.2, §7.1) — the CLI side of
// the Metadata query surface. The thin CLI owns rendering: it takes
// the query flags an agent workflow types, builds the same filter
// the REST door validates (src/query.ts's parseQueryFilter — one
// validation path for both shapes), and renders the surface's
// response document JSON-first, exactly as the service answered it,
// so a CLI result and a REST result are the same bytes modulo
// whitespace. This module is the verb logic; a CLI entrypoint wires
// it to argv/stdout and to an authenticated fetch of the door (the
// session carriage is src/session-resolution.ts).

import { FailClosed } from "./errors.ts";
import { parseQueryFilter, type MetadataFilter, type QueryResponse } from "./query.ts";

/** Flag name → filter field. `--session` names the session_id
 * filter, as the native query verbs do. */
const FLAG_FIELDS: Record<string, string> = {
  "--repo": "repo",
  "--kind": "kind",
  "--direct-parent": "direct_parent",
  "--session": "session_id",
  "--lane": "lane",
  "--host": "host",
  "--queue-id": "queue_id",
  "--link-relation": "link_relation",
  "--link-status": "link_status",
  "--link-target": "link_target",
};

/** Parse `query` verb arguments into the surface's filter. Every
 * flag takes exactly one value; an unknown flag or a missing value
 * refuses (400 invalid_query) rather than being silently dropped —
 * a dropped filter is a wrong answer, not an empty one. */
export function parseQueryCliArgs(argv: string[]): MetadataFilter {
  const raw: Record<string, unknown> = {};
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    const field = FLAG_FIELDS[flag];
    if (field === undefined) {
      throw new FailClosed(400, "invalid_query", `unknown query flag: ${flag}`);
    }
    const value = argv[i + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new FailClosed(400, "invalid_query", `query flag ${flag} requires a value`);
    }
    raw[field] = value;
    i++;
  }
  return parseQueryFilter(raw);
}

/** Render a query response the CLI way: JSON first (design §3.2) —
 * the response document itself, pretty-printed, nothing else. */
export function renderQueryResponse(response: QueryResponse): string {
  return `${JSON.stringify(response, null, 2)}\n`;
}
