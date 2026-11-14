// codedb-bridge — thin stateless stdio MCP bridge to a shared codedb daemon (#4120, from the #2225 determination).
//
// Why this exists: `codedb mcp` runs one stdio process per client session
// and each process loads the whole project index (~1.1 GB RSS warm /
// ~2.1 GB cold-peak on ~/git/code, measured in #2225). `codedb serve`
// holds one index per project root as an HTTP daemon on :7719, but it
// speaks no MCP. This bridge is the per-session shim: it exposes the
// daemon-backed codedb MCP tools over stdio and forwards every call to
// http://127.0.0.1:7719. It holds no index and no state; the expensive
// part (the daemon) is shared by every session.
//
// Daemon surface (verified live against codedb 0.2.2, 2026-11-02):
//   GET /health                          -> {"status":"ok"}
//   GET /explore/tree                    -> {"tree":"..."}
//   GET /explore/outline?path=<rel>      -> {"path":...,"symbols":[...]}
//   GET /explore/symbol?name=<name>      -> {"name":...,"results":[...]}
//   GET /explore/search?q=<query>        -> {"query":...,"results":[...]}
//   GET /explore/word?q=<word>           -> {"query":...,"hits":[...]}
//   GET /explore/hot                     -> {"files":[...]}
//   GET /explore/deps?path=<rel>         -> {"path":...,"imported_by":[...]}
// The daemon has no HTTP route for codedb_read / codedb_edit /
// codedb_changes / codedb_status / codedb_snapshot / codedb_bundle /
// codedb_projects / codedb_index, so the bridge does not advertise them.
//
// Argument contract (per #3352: no silent third paths): arguments the
// daemon cannot honour are rejected by name, never silently ignored —
// `project` (the daemon serves exactly one root), `regex: true`,
// `scope: true`, `compact: true`. `max_results` / `limit` are honoured
// by slicing the daemon's full result set. `body: true` on codedb_symbol
// is served from the daemon's per-result `detail` (the definition line);
// it is not a separate fetch.
//
// Config: CODEDB_DAEMON_URL (default http://127.0.0.1:7719),
// CODEDB_DAEMON_TIMEOUT_MS (default 10000).

export const DEFAULT_DAEMON_URL = 'http://127.0.0.1:7719';
export const DEFAULT_DAEMON_FETCH_TIMEOUT_MS = 10_000;

export const parseDaemonFetchTimeoutMs = (raw) => {
  if (raw === undefined || raw === '') return DEFAULT_DAEMON_FETCH_TIMEOUT_MS;
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`CODEDB_DAEMON_TIMEOUT_MS="${raw}" must be a positive integer`);
  }
  return value;
};

export const parseDaemonUrl = (raw) => {
  const value = raw === undefined || raw === '' ? DEFAULT_DAEMON_URL : raw;
  let url;
  try {
    url = new URL(value);
  } catch {
    // No branch of this function interpolates the raw value or any part
    // of the parsed URL into an error (#9829): the caller prints the
    // message to stderr, and the value can carry userinfo credentials,
    // a ?token= query, or a fragment -- all secrets in host logs. Errors
    // name the rejected component only.
    throw new Error('CODEDB_DAEMON_URL is not a valid URL (value withheld)');
  }
  if (url.protocol !== 'http:' || (url.pathname !== '/' && url.pathname !== '')) {
    throw new Error(
      'CODEDB_DAEMON_URL must be an http:// origin with no path (the bridge appends /explore/* routes itself; value withheld)'
    );
  }
  if (url.search) {
    throw new Error(
      'CODEDB_DAEMON_URL must not carry a query string - a query has no meaning for a daemon origin and would be silently dropped (query value withheld)'
    );
  }
  if (url.hash) {
    throw new Error(
      'CODEDB_DAEMON_URL must not carry a fragment - a fragment has no meaning for a daemon origin and would be silently dropped (fragment value withheld)'
    );
  }
  if (url.username || url.password) {
    throw new Error(
      'CODEDB_DAEMON_URL must not carry userinfo (embedded credentials) - the daemon has no authentication, so credentials would be silently dropped (userinfo withheld)'
    );
  }
  return url.origin;
};

const PROJECT_ARG_NOTE =
  'Not supported by codedb-bridge: the shared daemon serves exactly one project root, so there is no per-call project to select.';

const tool = (name, description, properties, required = []) => ({
  name,
  description,
  inputSchema: { type: 'object', properties, required },
});

export const CODEDB_TOOLS = [
  tool(
    'codedb_tree',
    'Get the full file tree of the indexed codebase with language detection, line counts, and symbol counts per file. Use this first to understand the project structure. Served by the shared codedb daemon (GET /explore/tree).',
    {}
  ),
  tool(
    'codedb_outline',
    'Get the structural outline of a file: all functions, structs, enums, imports, constants with line numbers. Like an IDE symbol view. Served by the shared codedb daemon (GET /explore/outline).',
    { path: { type: 'string', description: 'File path relative to project root' } },
    ['path']
  ),
  tool(
    'codedb_symbol',
    'Find ALL definitions of a symbol name across the entire codebase. Returns every file and line where this symbol is defined, with the definition line as detail. Served by the shared codedb daemon (GET /explore/symbol).',
    {
      name: { type: 'string', description: 'Symbol name to search for (exact match)' },
      body: {
        type: 'boolean',
        description:
          'Accepted for compatibility; the daemon result already carries the definition line as `detail` and has no separate body fetch (default: false)',
      },
    },
    ['name']
  ),
  tool(
    'codedb_search',
    'Full-text search across all indexed files. Uses trigram index for fast substring matching. Returns matching lines with file paths and line numbers. Served by the shared codedb daemon (GET /explore/search). Regex, scope and compact modes have no daemon route and are rejected, not ignored.',
    {
      query: { type: 'string', description: 'Text to search for (substring match)' },
      max_results: { type: 'integer', description: 'Maximum results to return (sliced from the daemon result set)' },
    },
    ['query']
  ),
  tool(
    'codedb_word',
    'O(1) word lookup using inverted index. Finds all occurrences of an exact word (identifier) across the codebase. Much faster than search for single-word queries. Served by the shared codedb daemon (GET /explore/word).',
    { word: { type: 'string', description: 'Exact word/identifier to look up' } },
    ['word']
  ),
  tool(
    'codedb_hot',
    'Get the most recently modified files in the codebase, ordered by recency. Useful to see what\'s been actively worked on. Served by the shared codedb daemon (GET /explore/hot).',
    { limit: { type: 'integer', description: 'Number of files to return (sliced from the daemon result set)' } }
  ),
  tool(
    'codedb_deps',
    'Get reverse dependencies: which files import/depend on the given file. Useful for impact analysis. Served by the shared codedb daemon (GET /explore/deps).',
    { path: { type: 'string', description: 'File path to check dependencies for' } },
    ['path']
  ),
];

// Keys the bridge rejects with a dedicated message when set (handled
// rejection keys, #3352): `project` on every tool, plus the daemon-less
// modes per tool. These stay accepted by the allowlist below so their
// existing by-name rejection messages are unchanged; anything else
// outside the tool's schema is refused before it can be silently dropped
// (#4216).
const HANDLED_REJECTION_KEYS = {
  codedb_tree: ['project'],
  codedb_outline: ['project', 'compact'],
  codedb_symbol: ['project'],
  codedb_search: ['project', 'regex', 'scope', 'compact'],
  codedb_word: ['project'],
  codedb_hot: ['project'],
  codedb_deps: ['project'],
};

// The accepted argument names for a tool: its advertised schema
// properties plus its handled rejection keys. Returns null for unknown
// tools (the switch in routeForTool still owns that error).
const acceptedArgsFor = (name) => {
  const declared = CODEDB_TOOLS.find((t) => t.name === name);
  if (!declared) return null;
  return new Set([...Object.keys(declared.inputSchema.properties), ...(HANDLED_REJECTION_KEYS[name] ?? [])]);
};

// Map a tool call to the daemon route. Throws an Error naming the exact
// problem for unknown tools, missing required arguments, and arguments
// the daemon cannot honour — nothing is silently dropped (#3352).
export const routeForTool = (name, args = {}) => {
  const accepted = acceptedArgsFor(name);
  if (accepted !== null) {
    for (const key of Object.keys(args)) {
      if (!accepted.has(key)) {
        throw new Error(
          `argument "${key}": not a valid argument for ${name} (valid: ${[...accepted].sort().join(', ') || '(none)'}); it would be silently ignored by the codedb daemon route`
        );
      }
    }
  }
  const rejectProject = () => {
    if (args.project !== undefined) throw new Error(`argument "project": ${PROJECT_ARG_NOTE}`);
  };
  const rejectTrue = (key) => {
    if (args[key] === true) {
      throw new Error(`argument "${key}": true is not supported by the codedb daemon route for ${name}; omit it or call codedb directly`);
    }
  };
  // regex/scope/compact have no daemon route on any tool — reject
  // uniformly so no caller ever believes a mode was applied (#4197).
  const rejectUnhonourable = () => {
    rejectTrue('regex');
    rejectTrue('scope');
    rejectTrue('compact');
  };
  // limit/max_results are honoured by exactly one tool each
  // (codedb_hot honours limit, codedb_search honours max_results).
  const rejectLimitArg = (key, hint) => {
    if (args[key] !== undefined) {
      throw new Error(`argument "${key}" is not supported by the codedb daemon route for ${name}; ${hint}`);
    }
  };
  if (CODEDB_TOOLS.some((candidate) => candidate.name === name)) {
    rejectProject();
    rejectUnhonourable();
    if (name !== 'codedb_search') {
      rejectLimitArg('max_results', 'only codedb_search honours max_results');
    }
    if (name !== 'codedb_hot') {
      rejectLimitArg('limit', 'only codedb_hot honours limit');
    }
  }
  const need = (key) => {
    if (typeof args[key] !== 'string' || args[key] === '') {
      throw new Error(`argument "${key}" is required for ${name} and must be a non-empty string`);
    }
    return args[key];
  };
  switch (name) {
    case 'codedb_tree':
      return { route: '/explore/tree', kind: 'tree' };
    case 'codedb_outline':
      return { route: `/explore/outline?path=${encodeURIComponent(need('path'))}`, kind: 'json' };
    case 'codedb_symbol':
      return { route: `/explore/symbol?name=${encodeURIComponent(need('name'))}`, kind: 'json' };
    case 'codedb_search':
      return { route: `/explore/search?q=${encodeURIComponent(need('query'))}`, kind: 'json', slice: ['results', args.max_results] };
    case 'codedb_word':
      return { route: `/explore/word?q=${encodeURIComponent(need('word'))}`, kind: 'json' };
    case 'codedb_hot':
      return { route: '/explore/hot', kind: 'json', slice: ['files', args.limit] };
    case 'codedb_deps':
      return { route: `/explore/deps?path=${encodeURIComponent(need('path'))}`, kind: 'json' };
    default:
      throw new Error(
        `unknown tool "${name}": codedb-bridge serves only the daemon-backed tools (${CODEDB_TOOLS.map((t) => t.name).join(', ')}); the daemon has no route for the other codedb tools`
      );
  }
};

const sliceData = (data, slice) => {
  if (!slice) return data;
  const [key, n] = slice;
  if (n === undefined) return data;
  if (!Number.isInteger(n) || n < 0) throw new Error(`slice limit must be a non-negative integer, got ${JSON.stringify(n)}`);
  if (Array.isArray(data?.[key])) return { ...data, [key]: data[key].slice(0, n) };
  return data;
};

const textResult = (text, isError = false) => ({
  content: [{ type: 'text', text }],
  ...(isError ? { isError: true } : {}),
});

export const createHandler = ({ fetchFn, daemonUrl, timeoutMs = DEFAULT_DAEMON_FETCH_TIMEOUT_MS }) => {
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw new Error(`timeoutMs must be a positive integer, got ${timeoutMs}`);
  }
  const callTool = async (params) => {
    const name = params?.name;
    const args = params?.arguments ?? {};
    let mapped;
    try {
      mapped = routeForTool(name, args);
    } catch (err) {
      return textResult(err.message, true);
    }
    let res;
    let data;
    let timeoutId;
    const controller = new AbortController();
    const requestUrl = `${daemonUrl}${mapped.route}`;
    let rejectTimeout;
    const timeout = new Promise((_, reject) => {
      rejectTimeout = reject;
    });
    timeoutId = setTimeout(() => {
      const err = new Error(`timed out after ${timeoutMs}ms`);
      err.name = 'TimeoutError';
      controller.abort(err);
      rejectTimeout(err);
    }, timeoutMs);
    try {
      res = await Promise.race([fetchFn(requestUrl, { signal: controller.signal }), timeout]);
      try {
        data = await Promise.race([res.json(), timeout]);
      } catch (err) {
        if (err.name === 'TimeoutError') throw err;
        return textResult(`codedb daemon at ${daemonUrl} returned a non-JSON response (HTTP ${res.status}) for ${mapped.route}`, true);
      }
    } catch (err) {
      if (err.name === 'TimeoutError') {
        return textResult(`codedb daemon request timed out after ${timeoutMs}ms at ${daemonUrl}${mapped.route}`, true);
      }
      return textResult(
        `codedb daemon unreachable at ${daemonUrl} (${err.message}). Is the codedb serve unit running for this project root? Start it with: systemctl --user start codedb-<root>.service`,
        true
      );
    } finally {
      clearTimeout(timeoutId);
    }
    if (!res.ok || data?.error) {
      return textResult(`codedb daemon error for ${name}: ${data?.error ?? `HTTP ${res.status}`}`, true);
    }
    // A 200 body that is null or not a JSON object is a daemon malfunction,
    // not a result (#6917): the tree path dereferences data.tree (a raw
    // TypeError escapes handle() and the caller never gets a response),
    // and the JSON paths would report the literal text "null" as success.
    if (!data || typeof data !== 'object' || Array.isArray(data)) {
      return textResult(
        `codedb daemon at ${daemonUrl} returned a ${data === null ? 'null' : Array.isArray(data) ? 'array' : typeof data} JSON body for ${mapped.route}; expected a JSON object`,
        true
      );
    }
    try {
      data = sliceData(data, mapped.slice);
    } catch (err) {
      return textResult(err.message, true);
    }
    return textResult(mapped.kind === 'tree' ? String(data.tree ?? '') : JSON.stringify(data));
  };

  // Returns a JSON-RPC response object, or null for notifications.
  const handle = async (msg) => {
    if (msg === null || typeof msg !== 'object' || Array.isArray(msg)) return null;
    const { id, method, params } = msg;
    if (id === undefined || id === null) return null; // notification: nothing to answer
    const ok = (result) => ({ jsonrpc: '2.0', id, result });
    const fail = (code, message) => ({ jsonrpc: '2.0', id, error: { code, message } });
    switch (method) {
      case 'initialize':
        return ok({
          protocolVersion: params?.protocolVersion ?? '2024-11-05',
          capabilities: { tools: { listChanged: false } },
          serverInfo: { name: 'codedb-bridge', version: '1.0.0' },
        });
      case 'ping':
        return ok({});
      case 'tools/list':
        return ok({ tools: CODEDB_TOOLS });
      case 'tools/call':
        return ok(await callTool(params ?? {}));
      default:
        return fail(-32601, `method not found: ${method}`);
    }
  };
  return handle;
};

const main = async () => {
  let daemonUrl;
  try {
    daemonUrl = parseDaemonUrl(process.env.CODEDB_DAEMON_URL);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
  let timeoutMs;
  try {
    timeoutMs = parseDaemonFetchTimeoutMs(process.env.CODEDB_DAEMON_TIMEOUT_MS);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
  const handle = createHandler({ fetchFn: (url, options) => fetch(url, options), daemonUrl, timeoutMs });
  const readline = await import('node:readline');
  const rl = readline.createInterface({ input: process.stdin, terminal: false });
  // Requests are dispatched independently, never awaited inside the read
  // loop: mcpx can share one bridge process across sessions, so a slow
  // daemon call must not head-of-line block unrelated calls or pings.
  // Responses are correlated by their JSON-RPC id and may complete out
  // of order (Codex finding on PR #4171).
  let pending = 0;
  let closed = false;
  const maybeExit = () => {
    if (!closed || pending !== 0) return;
    // process.exit() discards stdout bytes still queued for a pipe, which
    // truncates the final response when stdin closes mid-request (Codex
    // finding on PR #4171). Queue an empty write behind everything else:
    // its callback fires only once all prior output has flushed.
    process.stdout.write('', () => process.exit(0));
  };
  const settle = () => {
    pending -= 1;
    maybeExit();
  };
  rl.on('line', (line) => {
    const trimmed = line.trim();
    if (trimmed === '') return;
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch {
      return; // not JSON-RPC; codedb's own stdio server ignores malformed lines the same way
    }
    pending += 1;
    handle(msg)
      .then((response) => {
        if (response === null) {
          settle();
          return;
        }
        // Settle only when this response has actually flushed, so the
        // exit path above can never outrun a queued write.
        process.stdout.write(`${JSON.stringify(response)}\n`, settle);
      })
      .catch(() => settle()); // a handler failure must not kill the shared bridge; the caller times out on that one id
  });
  rl.on('close', () => {
    closed = true;
    maybeExit();
  });
};

import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (isMain) {
  await main();
}
