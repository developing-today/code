// Pure helpers for Lab Ops Hub: crawl-target validation (#441), the
// /api/budget computation (#442), and upload R2 key derivation (#2478).
// Kept free of Worker bindings so they can be unit-tested with plain
// Node (test/lib.test.ts).

// --- Crawl URL validation ---------------------------------------------------
// The crawler fetches a caller-supplied URL from inside the Worker's /
// container's network position, so it must refuse anything that is not a
// public http(s) target: non-http schemes, localhost-style names, and
// private / loopback / link-local / otherwise non-global IP literals.
// (This checks the literal host only; a public hostname that resolves to a
// private address — DNS rebinding — is a documented residual in README.md.
// The Python container crawler applies the same policy independently.)

function ipv4ToInt(host: string): number | null {
  const parts = host.split(".");
  if (parts.length !== 4) return null;
  let n = 0;
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) return null;
    const v = Number(part);
    if (v > 255) return null;
    n = n * 256 + v;
  }
  return n;
}

function ipv4IsGlobal(n: number): boolean {
  const a = Math.floor(n / 2 ** 24);
  const b = Math.floor(n / 2 ** 16) % 256;
  const c = Math.floor(n / 2 ** 8) % 256;
  const d = n % 256;
  if (a === 0 || a === 10 || a === 127) return false; // "this" net, private, loopback
  if (a === 100 && b >= 64 && b <= 127) return false; // CGNAT 100.64.0.0/10
  if (a === 169 && b === 254) return false; // link-local (cloud metadata)
  if (a === 172 && b >= 16 && b <= 31) return false; // private 172.16.0.0/12
  // The /24 special ranges are keyed on the third octet (#476): keying
  // them on b alone widened each to a /16 and refused public neighbours.
  if (a === 192 && b === 0 && c === 0) return d === 9 || d === 10; // 192.0.0.0/24, except PCP/DNS anycast .9/.10
  if (a === 192 && b === 0 && c === 2) return false; // TEST-NET-1 192.0.2.0/24
  if (a === 192 && b === 88 && c === 99) return false; // 6to4 relay anycast 192.88.99.0/24 (deprecated)
  if (a === 192 && b === 168) return false; // private
  if (a === 198 && (b === 18 || b === 19)) return false; // benchmarking
  if (a === 198 && b === 51 && c === 100) return false; // TEST-NET-2 198.51.100.0/24
  if (a === 203 && b === 0 && c === 113) return false; // TEST-NET-3 203.0.113.0/24
  if (a >= 224) return false; // multicast + reserved
  return true;
}

// Expand an IPv6 literal to its eight 16-bit groups, or null if it does
// not parse. A trailing dotted-quad counts as the last two groups.
function ipv6Groups(host: string): number[] | null {
  let h = host;
  const v4tail = h.match(/(\d+\.\d+\.\d+\.\d+)$/);
  if (v4tail) {
    const v4 = ipv4ToInt(v4tail[1]);
    if (v4 === null) return null;
    h = `${h.slice(0, -v4tail[1].length)}${Math.floor(v4 / 65536).toString(16)}:${v4 % 65536}`;
  }
  const halves = h.split("::");
  if (halves.length > 2) return null;
  const parseSide = (s: string): number[] | null => {
    if (s === "") return [];
    const groups: number[] = [];
    for (const part of s.split(":")) {
      if (!/^[0-9a-f]{1,4}$/.test(part)) return null;
      groups.push(parseInt(part, 16));
    }
    return groups;
  };
  const left = parseSide(halves[0]);
  if (left === null) return null;
  if (halves.length === 1) return left.length === 8 ? left : null;
  const right = parseSide(halves[1]);
  if (right === null) return null;
  const missing = 8 - left.length - right.length;
  if (missing < 1) return null;
  return [...left, ...new Array<number>(missing).fill(0), ...right];
}

function ipv6IsGlobal(host: string): boolean {
  const g = ipv6Groups(host.toLowerCase());
  if (g === null) return false;
  // Prefixes that embed an IPv4 address are judged by that address, the
  // same policy as plain IPv4: IPv4-mapped ::ffff:0/96, the NAT64
  // well-known prefix 64:ff9b::/96, and 6to4 2002::/16 (#472). (WHATWG
  // URL normalises the mapped form to hex groups, e.g. ::ffff:7f00:1,
  // so this must work on groups, not the dotted spelling.)
  const embedded = (hi: number, lo: number) => hi * 65536 + lo;
  if (g[0] === 0 && g[1] === 0 && g[2] === 0 && g[3] === 0 && g[4] === 0 && g[5] === 0xffff) {
    return ipv4IsGlobal(embedded(g[6], g[7]));
  }
  if (g[0] === 0x64 && g[1] === 0xff9b && g[2] === 0 && g[3] === 0 && g[4] === 0 && g[5] === 0) {
    return ipv4IsGlobal(embedded(g[6], g[7]));
  }
  if (g[0] === 0x2002) return ipv4IsGlobal(embedded(g[1], g[2]));
  if (g.every((x) => x === 0)) return false; // unspecified ::
  if (g[7] === 1 && g.slice(0, 7).every((x) => x === 0)) return false; // loopback ::1
  if (g[0] >= 0xfe80 && g[0] <= 0xfebf) return false; // link-local fe80::/10
  if (g[0] >= 0xfc00 && g[0] <= 0xfdff) return false; // unique-local fc00::/7
  if (g[0] >= 0xff00) return false; // multicast ff00::/8
  if (g[0] === 0x2001 && g[1] === 0x0db8) return false; // documentation 2001:db8::/32
  if (g[0] === 0x3fff && (g[1] & 0xf000) === 0) return false; // documentation 3fff::/20 (RFC 9637) (#519)
  // The 2001::/23 IANA special-purpose block is non-global except six
  // entries (#474, mirroring the Python validator's ipaddress verdicts):
  // 2001:1::1 and 2001:1::2 (PCP anycast), 2001:3::/32 (AMT),
  // 2001:4:112::/48 (AS112), 2001:20::/28 and 2001:30::/28 (ORCHIDv2).
  if (g[0] === 0x2001 && (g[1] & 0xfe00) === 0) {
    if (g[1] === 0x0003) return true;
    if (g[1] === 0x0004 && g[2] === 0x0112) return true;
    if (g[1] >= 0x0020 && g[1] <= 0x003f) return true;
    if (
      g[1] === 0x0001 &&
      g[2] === 0 &&
      g[3] === 0 &&
      g[4] === 0 &&
      g[5] === 0 &&
      g[6] === 0 &&
      (g[7] === 1 || g[7] === 2)
    ) {
      return true;
    }
    return false;
  }
  // Global unicast lives in 2000::/3 (first hextet 2xxx or 3xxx).
  return g[0] >= 0x2000 && g[0] <= 0x3fff;
}

const BLOCKED_HOST_SUFFIXES = [".localhost", ".local", ".internal", ".lan", ".home.arpa"];

export function validateCrawlUrl(raw: string): { ok: true; url: string } | { ok: false; error: string } {
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return { ok: false, error: "url is not a valid URL" };
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") {
    return { ok: false, error: "only http: and https: URLs may be crawled" };
  }
  // WHATWG URL normalises integer/hex IPv4 forms (e.g. 2130706433,
  // 0x7f.1) to dotted-quad, and keeps IPv6 literals bracketed.
  // A single trailing dot is the DNS root dot (the FQDN spelling of the
  // same host: "localhost." is "localhost"); strip it before the checks
  // so the dotted spelling cannot evade them (#466).
  const rawHost = u.hostname.toLowerCase();
  const host = rawHost.endsWith(".") ? rawHost.slice(0, -1) : rawHost;
  if (!host) return { ok: false, error: "url has no host" };
  // An empty label (a second trailing dot, an interior "..", or a
  // leading dot) is not a valid DNS name at all, and the one-dot strip
  // above would let "localhost.." slip past the blocked-host checks as
  // "localhost." — refuse it instead of certifying it as a public DNS
  // name, matching the container's validator (#511). Bracketed IPv6
  // literals are judged by ipv6IsGlobal below, not by label structure.
  if (!host.startsWith("[") && host.split(".").some((label) => label === "")) {
    return { ok: false, error: `refusing to crawl host with an empty label ${host}` };
  }
  // A label over 63 octets, or a name over 253 characters, can never
  // be a DNS name at all (RFC 1035 §2.3.4: 63-octet labels, 255-octet
  // wire names = 253 characters of text without the root dot, which
  // the strip above already removed) — refuse it up front instead of
  // certifying it as a public DNS name and letting the resolver fail
  // on it, matching the container's validator (#513). u.hostname is
  // already the punycode (IDNA-encoded) form, so these lengths are
  // the wire lengths exactly. Bracketed IPv6 literals are exempt, as
  // with the empty-label check above.
  if (!host.startsWith("[") && (host.length > 253 || host.split(".").some((label) => label.length > 63))) {
    return { ok: false, error: `refusing to crawl host with an overlong label or name ${host}` };
  }
  // "home.arpa" needs the same equality arm as "localhost": the
  // suffixes carry a leading dot, so the bare apex escapes them, but
  // home.arpa is itself the RFC 8375 special-use name for the local
  // home network (#608). The other suffixes' bare labels ("local",
  // "internal", "lan") are ordinary single-label hostnames, not
  // designated local names, and stay accepted.
  if (host === "localhost" || host === "home.arpa" || BLOCKED_HOST_SUFFIXES.some((s) => host.endsWith(s))) {
    return { ok: false, error: `refusing to crawl local host ${host}` };
  }
  if (host.startsWith("[") && host.endsWith("]")) {
    if (!ipv6IsGlobal(host.slice(1, -1))) {
      return { ok: false, error: `refusing to crawl non-public IPv6 address ${host}` };
    }
    return { ok: true, url: u.toString() };
  }
  const v4 = ipv4ToInt(host);
  if (v4 !== null && !ipv4IsGlobal(v4)) {
    return { ok: false, error: `refusing to crawl non-public IPv4 address ${host}` };
  }
  return { ok: true, url: u.toString() };
}

// --- Redirect-following fetch with per-hop validation (#460) ----------------
// fetch()'s default redirect mode ("follow") silently follows a 3xx from a
// validated public URL to any target — including private / loopback /
// link-local addresses validateCrawlUrl refuses as direct targets. The
// container crawler validates every redirect target (container/app.py's
// _ValidatingRedirectHandler); the Worker fallback must hold the same
// policy. Follow redirects manually: resolve each Location against the
// current URL, validate it, and only then fetch it. The start URL is
// validated the same way before the first fetch (#464): this helper is the
// component that actually fetches on the Worker-fallback path, so it must
// be safe by construction rather than rely on callers pre-validating. The
// fetcher is injected (it must use redirect:"manual") so this is testable
// with plain Node.

/** The R2 archive key for one crawl's full text (#2503). Date.now()
 * alone collides for same-millisecond crawls — the second PUT silently
 * overwrites the first crawl's archive while both D1 rows point at it —
 * so the key carries a caller-supplied unique suffix (the route passes
 * crypto.randomUUID()). Pure and injectable for tests. */
export function crawlArchiveKey(nowMs: number, unique: string): string {
  return `crawls/${nowMs}-${unique}.json`;
}

export type ManualFetcher = (url: string) => Promise<Response>;

// Keep this policy in sync with container/app.py's _ValidatingRedirectHandler:
// urllib max_redirections=10, max_repeats=4, and only 301/302/303/307/308.
const FOLLOWABLE_REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);
const MAX_REDIRECT_REPEATS = 4;

export async function fetchFollowingValidatedRedirects(
  startUrl: string,
  fetchFn: ManualFetcher,
  maxRedirects = 10,
): Promise<Response> {
  const start = validateCrawlUrl(startUrl);
  if (!start.ok) {
    throw new Error(`refusing to crawl ${startUrl}: ${start.error}`);
  }
  let current = start.url;
  const redirectVisits = new Map<string, number>();
  for (let hop = 0; ; hop++) {
    const res = await fetchFn(current);
    if (res.status < 300 || res.status >= 400) return res;
    if (!FOLLOWABLE_REDIRECT_STATUSES.has(res.status)) {
      throw new Error(`unsupported redirect status ${res.status} while crawling ${startUrl}`);
    }
    const location = res.headers.get("location");
    if (!location) {
      throw new Error(`redirect response ${res.status} has no Location while crawling ${startUrl}`);
    }
    if (hop >= maxRedirects) {
      throw new Error(`too many redirects (more than ${maxRedirects}) while crawling ${startUrl}`);
    }
    let next: string;
    try {
      next = new URL(location, current).toString();
    } catch {
      throw new Error(`redirect from ${current} has an invalid Location: ${location}`);
    }
    const target = validateCrawlUrl(next);
    if (!target.ok) {
      throw new Error(`refusing to follow redirect from ${current}: ${target.error}`);
    }
    current = target.url;
    const visits = redirectVisits.get(current) ?? 0;
    if (visits >= MAX_REDIRECT_REPEATS) {
      throw new Error(`redirect target repeated more than ${MAX_REDIRECT_REPEATS} times while crawling ${startUrl}`);
    }
    redirectVisits.set(current, visits + 1);
  }
}

// --- Timed crawl fetcher (#500, testable since #524) -------------------------
// Every hop of the Worker-fallback crawl gets the container crawler's 20s
// timeout (container/app.py's _OPENER.open(timeout=20)): without a signal, a
// validated public server that accepts the connection and then stalls — or
// slow-drips a body under the byte cap — hangs the crawl until platform
// limits kill it. The signal stays attached to the response, so it also
// bounds the readResponseTextCapped body read. This factory lives here (not
// inline in the Worker entry) so the timeout is pinned by unit tests: before
// #524 the fetcher was an anonymous inline arrow in src/index.ts, which no
// test can import, and the #500 fix had no regression coverage.

export const CRAWL_FETCH_TIMEOUT_MS = 20_000;

export type FetchImpl = (url: string, init: RequestInit) => Promise<Response>;

export function crawlFetcher(timeoutMs: number = CRAWL_FETCH_TIMEOUT_MS, fetchImpl: FetchImpl = fetch): ManualFetcher {
  return (u) =>
    fetchImpl(u, {
      redirect: "manual",
      headers: { "user-agent": "LabOpsHub/0.1 (+personal crawler)" },
      signal: AbortSignal.timeout(timeoutMs),
    });
}

// --- Crawl body reading -----------------------------------------------------
// The container crawler caps its body read at 2,000,000 bytes
// (container/app.py: res.read(2_000_000)) and truncates silently. The Worker
// fallback must do the same (#481): an unbounded res.text() buffers the whole
// body of a validated public URL — however large — in the Worker's memory
// before the caller slices it down to a 500-char excerpt / 20k-char text.

export const MAX_CRAWL_BODY_BYTES = 2_000_000;

export async function readResponseTextCapped(res: Response, maxBytes = MAX_CRAWL_BODY_BYTES): Promise<string> {
  if (!res.body) return "";
  const reader = res.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: false });
  let text = "";
  let read = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    read += value.byteLength;
    if (read > maxBytes) {
      // Keep only the bytes up to the cap (a multi-byte character split at
      // the boundary decodes as a replacement character, matching the
      // container's errors="replace"), then cancel so the rest of the body
      // is never pulled into memory.
      text += decoder.decode(value.subarray(0, value.byteLength - (read - maxBytes)), { stream: true });
      await reader.cancel();
      break;
    }
    text += decoder.decode(value, { stream: true });
  }
  return text + decoder.decode();
}

// --- Budget computation -------------------------------------------------------
// estimated_overage_usd is computed from the counted usage, never a
// literal: for each resource with an included amount and a rate,
// max(0, used - included) * rate, summed. Resources without a rate
// (container runs, AI neurons) are shown but excluded from the sum.
//
// D1 rows written: the counter covers every application write path
// (each emits a "d1_write" usage event), PLUS the usage ledger's own
// writes, accounted for explicitly: every usage_events row is itself
// one D1 row written, so the ledger total (the sum of every kind's
// count, d1_write events included) is added on top.

export interface BudgetResource {
  used: number;
  included: number | null;
  overage_per_million_usd?: number;
  overage_per_operation_usd?: number;
  estimated_overage_usd?: number;
  note?: string;
}

function ratePerUnit(r: BudgetResource): number | null {
  if (r.overage_per_million_usd !== undefined) return r.overage_per_million_usd / 1_000_000;
  if (r.overage_per_operation_usd !== undefined) return r.overage_per_operation_usd;
  return null;
}

export function computeBudget(by: Record<string, number>): {
  month_to_date_self_counted: Record<string, BudgetResource>;
  estimated_overage_usd: number;
} {
  const ledgerRows = Object.values(by).reduce((sum, n) => sum + n, 0);
  const resources: Record<string, BudgetResource> = {
    worker_requests: { used: by.worker_request ?? 0, included: 10_000_000, overage_per_million_usd: 0.3 },
    d1_rows_written: {
      used: (by.d1_write ?? 0) + ledgerRows,
      included: 50_000_000,
      overage_per_million_usd: 1.0,
      note: "application writes (d1_write events) + the usage ledger's own rows (one D1 write per usage event)",
    },
    queue_operations: { used: by.queue_msg ?? 0, included: 1_000_000, overage_per_operation_usd: 0.0000004 },
    container_runs: {
      used: by.container_run ?? 0,
      included: null,
      note: "billed per 10ms vCPU/memory/disk; runs are on-demand only, sleepAfter=2m",
    },
    ai_requests: { used: by.ai_request ?? 0, included: 10_000, note: "10,000 free neurons/day on Workers AI" },
  };
  let total = 0;
  for (const r of Object.values(resources)) {
    const rate = ratePerUnit(r);
    if (rate === null || r.included === null) continue;
    r.estimated_overage_usd = Math.max(0, r.used - r.included) * rate;
    total += r.estimated_overage_usd;
  }
  return {
    month_to_date_self_counted: resources,
    estimated_overage_usd: Math.round(total * 1_000_000) / 1_000_000,
  };
}

// --- JSON POST body validation (#2480) --------------------------------------
// The JSON POST routes (machines, notes, tasks, crawl) used to call
// req.json() unguarded and bind required fields unchecked: malformed JSON
// threw a SyntaxError and a missing NOT NULL field threw a raw SQLite
// constraint error, and the outer catch in index.ts turned both into a
// 500 carrying internal error text. These pure helpers give the routes a
// shared 400 vocabulary instead: an unparseable body -> "invalid JSON", a
// JSON value that is not an object -> "JSON object required", and a
// required field that is missing, not a string, or blank ->
// "<field> required". Valid values are returned untouched (no trimming),
// so successful requests are unchanged; genuine operational failures
// still fall through to the 500 path in index.ts.

export type JsonObjectBody = { ok: true; body: Record<string, unknown> } | { ok: false; error: string };

export async function parseJsonObjectBody(req: Request): Promise<JsonObjectBody> {
  let parsed: unknown;
  try {
    parsed = await req.json();
  } catch {
    return { ok: false, error: "invalid JSON" };
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { ok: false, error: "JSON object required" };
  }
  return { ok: true, body: parsed as Record<string, unknown> };
}

export type RequiredString = { ok: true; value: string } | { ok: false; error: string };

// A required field is satisfied only by a string with non-whitespace
// content; the original string is preserved exactly (callers bind what
// the client sent, not a trimmed copy).
export function requireNonBlankString(body: Record<string, unknown>, field: string): RequiredString {
  const v = body[field];
  if (typeof v !== "string" || v.trim() === "") return { ok: false, error: `${field} required` };
  return { ok: true, value: v };
}

export type Validated<T> = { ok: true; values: T } | { ok: false; error: string };

export function validateMachineBody(body: Record<string, unknown>): Validated<{ name: string }> {
  const name = requireNonBlankString(body, "name");
  if (!name.ok) return name;
  return { ok: true, values: { name: name.value } };
}

// True only for the machines.name UNIQUE violation (#2631). D1 surfaces
// constraint failures in the error message (possibly wrapped, e.g.
// "D1_ERROR: UNIQUE constraint failed: machines.name"); any other error —
// including a UNIQUE violation on a different column — is not a machine
// name conflict and must keep flowing to the 500 handler. Matching the
// thrown constraint (rather than pre-checking the name) keeps concurrent
// duplicate registrations from racing past validation.
export function isMachineNameConflict(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("UNIQUE constraint failed: machines.name");
}

// Notes check title before body, matching the order the fields are named
// in the route's contract (and the triage note for #2480).
export function validateNoteBody(body: Record<string, unknown>): Validated<{ title: string; text: string }> {
  const title = requireNonBlankString(body, "title");
  if (!title.ok) return title;
  const text = requireNonBlankString(body, "body");
  if (!text.ok) return text;
  return { ok: true, values: { title: title.value, text: text.value } };
}

export function validateTaskBody(body: Record<string, unknown>): Validated<{ title: string }> {
  const title = requireNonBlankString(body, "title");
  if (!title.ok) return title;
  return { ok: true, values: { title: title.value } };
}

// --- Upload R2 keys ---------------------------------------------------------
// The key used to be `uploads/${Date.now()}-${sanitized}`: two uploads in
// the same millisecond with the same (or sanitize-equivalent) filename
// derived the SAME key, so the second FILES.put overwrote the first
// object and its D1 INSERT then failed on the UNIQUE r2_key — leaving
// the first row pointing at the second upload's bytes (#2478). The
// random segment makes collisions practically impossible while the
// timestamp + sanitized name keep keys sortable and recognisable. The
// rand input is injectable so derivation stays a pure, testable
// function; callers pass crypto.randomUUID().
export function uploadKey(name: string, now: number, rand: string = crypto.randomUUID()): string {
  return `uploads/${now}-${rand}-${name.replace(/[^\w.\-]+/g, "_")}`;
}

// --- HTML extraction (#569) -------------------------------------------------
// extractHtml mirrors the container crawler's Extractor (container/app.py,
// an html.parser.HTMLParser) so the Worker fallback in index.ts and the
// container return identical {title, links, text} for the same page:
// - links counts every <a> start tag (bare <a> and <a/> included), never
//   anchors inside scripts, styles, or comments;
// - title is the concatenated character data of <title> (entities decoded),
//   and title data never leaks into text;
// - text is the remaining character data with script/style content
//   skipped (an unclosed script/style swallows the rest of the document,
//   as HTMLParser's CDATA mode does), chunks joined with one space and
//   whitespace collapsed — the same `" ".join` + `\s+` collapse app.py
//   applies. Paired fixtures: test/extractor-fixtures.json, exercised by
//   both test/extractor.test.ts and test/test_extractor_parity.py.
const NAMED_ENTITIES: Record<string, string> = {
  amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ",
  copy: "©", reg: "®", trade: "™", hellip: "…", mdash: "—", ndash: "–",
  lsquo: "‘", rsquo: "’", ldquo: "“", rdquo: "”", laquo: "«", raquo: "»",
  times: "×", divide: "÷", plusmn: "±", deg: "°", middot: "·",
  bull: "•", dagger: "†", permil: "‰", euro: "€", pound: "£", yen: "¥",
  cent: "¢", curren: "¤", sect: "§", para: "¶", micro: "µ", sup1: "¹",
  sup2: "²", sup3: "³", frac12: "½", frac14: "¼", frac34: "¾",
  iexcl: "¡", iquest: "¿", szlig: "ß", agrave: "à", aacute: "á",
  acirc: "â", atilde: "ã", auml: "ä", aring: "å", aelig: "æ",
  ccedil: "ç", egrave: "è", eacute: "é", ecirc: "ê", euml: "ë",
  igrave: "ì", iacute: "í", icirc: "î", iuml: "ï", ntilde: "ñ",
  ograve: "ò", oacute: "ó", ocirc: "ô", otilde: "õ", ouml: "ö",
  ugrave: "ù", uacute: "ú", ucirc: "û", uuml: "ü", yacute: "ý",
  yuml: "ÿ", Agrave: "À", Aacute: "Á", Auml: "Ä", Ccedil: "Ç",
  Egrave: "È", Eacute: "É", Euml: "Ë", Ntilde: "Ñ", Ograve: "Ò",
  Oacute: "Ó", Ouml: "Ö", Ugrave: "Ù", Uacute: "Ú", Uuml: "Ü",
};

// HTML numeric character-reference normalization: the same remapping
// html.unescape applies for the container's HTMLParser. C1 controls
// 0x80-0x9F map to their Windows-1252 characters; NUL, surrogates, and
// out-of-range values become U+FFFD.
const NUMERIC_REF_REMAP: Record<number, number> = {
  0x80: 0x20ac, 0x81: 0x0081, 0x82: 0x201a, 0x83: 0x0192, 0x84: 0x201e,
  0x85: 0x2026, 0x86: 0x2020, 0x87: 0x2021, 0x88: 0x02c6, 0x89: 0x2030,
  0x8a: 0x0160, 0x8b: 0x2039, 0x8c: 0x0152, 0x8d: 0x008d, 0x8e: 0x017d,
  0x8f: 0x008f, 0x90: 0x0090, 0x91: 0x2018, 0x92: 0x2019, 0x93: 0x201c,
  0x94: 0x201d, 0x95: 0x2022, 0x96: 0x2013, 0x97: 0x2014, 0x98: 0x02dc,
  0x99: 0x2122, 0x9a: 0x0161, 0x9b: 0x203a, 0x9c: 0x0153, 0x9d: 0x009d,
  0x9e: 0x017e, 0x9f: 0x0178,
};

function normalizeNumericRef(code: number): number {
  if (code === 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) return 0xfffd;
  return NUMERIC_REF_REMAP[code] ?? code;
}

export function decodeHtmlEntities(data: string): string {
  return data.replace(/&(#(?:[xX][0-9a-fA-F]+|\d+)|[a-zA-Z][a-zA-Z0-9]*);/g, (whole, body: string) => {
    if (body.startsWith("#")) {
      const hex = body[1] === "x" || body[1] === "X";
      const code = Number.parseInt(body.slice(hex ? 2 : 1), hex ? 16 : 10);
      return String.fromCodePoint(normalizeNumericRef(code));
    }
    // Own-property lookup only: a plain index would resolve inherited
    // Object properties (&constructor;, &toString;) to JavaScript
    // functions, where HTMLParser leaves unknown names literal.
    return Object.prototype.hasOwnProperty.call(NAMED_ENTITIES, body)
      ? NAMED_ENTITIES[body]
      : whole;
  });
}

// A '/' marks a start tag self-closing only where a new attribute could
// begin (after the tag name, whitespace, or a quoted value) AND it is
// the final character before '>': <script / > is an open script tag to
// HTMLParser, not a self-closing one. A '/' ending an UNQUOTED attribute
// value is part of the value: <script src=/x/> is likewise an open
// script tag, not a self-closing one.
function isSelfClosingTag(inner: string, nameEnd: number): boolean {
  let k = nameEnd;
  const len = inner.length;
  while (k < len) {
    while (k < len && /\s/.test(inner[k])) k += 1;
    if (k >= len) return false;
    if (inner[k] === "/") return k === len - 1;
    while (k < len && !/[\s=/]/.test(inner[k])) k += 1; // attribute name
    let m = k;
    while (m < len && /\s/.test(inner[m])) m += 1;
    if (m < len && inner[m] === "=") {
      m += 1;
      while (m < len && /\s/.test(inner[m])) m += 1;
      if (m < len && (inner[m] === '"' || inner[m] === "'")) {
        const q = inner[m];
        m += 1;
        while (m < len && inner[m] !== q) m += 1;
        if (m < len) m += 1;
      } else {
        // Unquoted value: everything up to whitespace ('/' included).
        while (m < len && !/\s/.test(inner[m])) m += 1;
      }
      k = m;
    }
  }
  return false;
}

export function extractHtml(html: string): { title: string; links: number; text: string } {
  let title = "";
  let links = 0;
  const textChunks: string[] = [];
  let skipDepth = 0;
  // Raw-text mode: script/style are HTMLParser CDATA elements, and
  // HTMLParser likewise reports <title> content as raw character data
  // (markup inside a title is literal and never counts as links).
  let cdataTag: "script" | "style" | "title" | null = null;

  const handleData = (raw: string) => {
    if (raw === "") return;
    const data = decodeHtmlEntities(raw);
    if (cdataTag === "title") title += data;
    else if (skipDepth === 0) textChunks.push(data);
  };
  const handleStartTag = (tag: string, selfClosing: boolean) => {
    if (tag === "title") {
      if (!selfClosing) cdataTag = "title";
    } else if (tag === "a") links += 1;
    else if (tag === "script" || tag === "style") {
      skipDepth += 1;
      if (selfClosing) skipDepth -= 1;
      else cdataTag = tag;
    }
  };
  const handleEndTag = (tag: string) => {
    if ((tag === "script" || tag === "style") && skipDepth > 0) skipDepth -= 1;
  };

  let i = 0;
  const n = html.length;
  while (i < n) {
    if (cdataTag !== null) {
      // HTMLParser CDATA mode: only the matching close tag is markup.
      const close = new RegExp(`</${cdataTag}(?=[\\s/>])`, "i").exec(html.slice(i));
      if (close === null) {
        handleData(html.slice(i));
        i = n;
        break;
      }
      handleData(html.slice(i, i + close.index));
      i += close.index;
      // Consume the close tag here, tolerantly: HTMLParser accepts
      // recoverable end tags carrying trailing attributes
      // (</script foo>), which the strict end-tag pattern in the normal
      // path rejects -- skipDepth would stay set and the rest of the
      // document would be discarded.
      const closeTag = new RegExp(`</${cdataTag}[^>]*>`, "i").exec(html.slice(i));
      const closedTag = cdataTag;
      cdataTag = null;
      if (closeTag !== null) {
        handleEndTag(closedTag);
        i += closeTag[0].length;
      }
      continue;
    }
    const lt = html.indexOf("<", i);
    if (lt === -1) {
      handleData(html.slice(i));
      break;
    }
    if (lt > i) handleData(html.slice(i, lt));
    i = lt;
    if (html.startsWith("<!--", i)) {
      // Comment ends per HTMLParser: --> or --!>, plus the abrupt
      // empty-comment forms <!--> and <!--->.
      if (html.startsWith(">", i + 4)) {
        i += 5;
        continue;
      }
      if (html.startsWith("->", i + 4)) {
        i += 6;
        continue;
      }
      const m = /--!?>/.exec(html.slice(i + 4));
      i = m === null ? n : i + 4 + m.index + m[0].length;
      continue;
    }
    const next = html[i + 1] ?? "";
    if (next === "!" || next === "?") {
      const end = html.indexOf(">", i + 2);
      i = end === -1 ? n : end + 1;
      continue;
    }
    if (next === "/") {
      // End tags may carry (invalid but recoverable) attributes, which
      // HTMLParser consumes rather than emitting as text: </p foo>
      // still closes the paragraph. The match is bounded at the next
      // '>' up front: an unbounded name-plus-remainder pattern
      // backtracks quadratically on an incomplete end tag at EOF
      // (</p followed by a long letter run and no '>').
      const gt = html.indexOf(">", i);
      const m =
        gt === -1 ? null : /^<\/([a-zA-Z][^\s/>]*)/.exec(html.slice(i, gt + 1));
      if (m === null) {
        if (gt === -1 && /^<\/[a-zA-Z]/.test(html.slice(i))) {
          // Incomplete end tag at EOF: HTMLParser drops it.
          i = n;
          continue;
        }
        handleData("<");
        i += 1;
        continue;
      }
      handleEndTag(m[1].toLowerCase());
      i = gt + 1;
      continue;
    }
    if (/[a-zA-Z]/.test(next)) {
      // Start tag: scan to the closing '>' without letting a quoted
      // attribute value end the tag early.
      let j = i + 1;
      let quote = "";
      let lastSig = "";
      while (j < n) {
        const ch = html[j];
        if (quote !== "") {
          if (ch === quote) {
            quote = "";
            lastSig = ch;
          }
        } else if ((ch === '"' || ch === "'") && lastSig === "=") {
          // A quote opens a quoted value only where a value begins
          // (right after '='); a quote inside an unquoted value, as in
          // <a href=/it's>, is data, as HTMLParser tokenizes it.
          quote = ch;
        } else if (ch === ">") break;
        else if (!/\s/.test(ch)) lastSig = ch;
        j += 1;
      }
      if (j >= n) {
        handleData(html.slice(i)); // unterminated tag: HTMLParser emits it as data
        break;
      }
      const inner = html.slice(i + 1, j);
      const nameMatch = /^([a-zA-Z][^\s/>]*)/.exec(inner);
      if (nameMatch === null) {
        handleData("<");
        i += 1;
        continue;
      }
      handleStartTag(nameMatch[1].toLowerCase(), isSelfClosingTag(inner, nameMatch[1].length));
      i = j + 1;
      continue;
    }
    handleData("<");
    i += 1;
  }

  const text = textChunks.join(" ").replace(/\s+/g, " ").trim();
  return { title: title.trim(), links, text };
}
