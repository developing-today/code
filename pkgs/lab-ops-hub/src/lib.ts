// Pure helpers for Lab Ops Hub: crawl-target validation (#441) and the
// /api/budget computation (#442). Kept free of Worker bindings so they can
// be unit-tested with plain Node (test/lib.test.ts).

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
  if (a === 0 || a === 10 || a === 127) return false; // "this" net, private, loopback
  if (a === 100 && b >= 64 && b <= 127) return false; // CGNAT 100.64.0.0/10
  if (a === 169 && b === 254) return false; // link-local (cloud metadata)
  if (a === 172 && b >= 16 && b <= 31) return false; // private 172.16.0.0/12
  if (a === 192 && b === 0) return false; // 192.0.0.0/24 + TEST-NET-1 192.0.2.0/24
  if (a === 192 && b === 88) return false; // 6to4 relay anycast (deprecated)
  if (a === 192 && b === 168) return false; // private
  if (a === 198 && (b === 18 || b === 19)) return false; // benchmarking
  if (a === 198 && b === 51) return false; // TEST-NET-2
  if (a === 203 && b === 0) return false; // TEST-NET-3 203.0.113.0/24
  if (a >= 224) return false; // multicast + reserved
  return true;
}

function ipv6IsGlobal(host: string): boolean {
  const h = host.toLowerCase();
  // IPv4-mapped address: judge the embedded IPv4 address.
  const mapped = h.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/);
  if (mapped) {
    const v4 = ipv4ToInt(mapped[1]);
    return v4 !== null && ipv4IsGlobal(v4);
  }
  if (h === "::" || h === "::1") return false; // unspecified, loopback
  if (/^fe[89ab]/.test(h)) return false; // link-local fe80::/10
  if (/^f[cd]/.test(h)) return false; // unique-local fc00::/7
  if (/^ff/.test(h)) return false; // multicast
  if (h.startsWith("2001:db8:")) return false; // documentation 2001:db8::/32
  // Global unicast lives in 2000::/3 (first hextet 2xxx or 3xxx).
  return /^[23]/.test(h);
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
  if (host === "localhost" || BLOCKED_HOST_SUFFIXES.some((s) => host.endsWith(s))) {
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

export type ManualFetcher = (url: string) => Promise<Response>;

export async function fetchFollowingValidatedRedirects(
  startUrl: string,
  fetchFn: ManualFetcher,
  maxRedirects = 5,
): Promise<Response> {
  const start = validateCrawlUrl(startUrl);
  if (!start.ok) {
    throw new Error(`refusing to crawl ${startUrl}: ${start.error}`);
  }
  let current = start.url;
  for (let hop = 0; ; hop++) {
    const res = await fetchFn(current);
    if (res.status < 300 || res.status >= 400) return res;
    const location = res.headers.get("location");
    if (!location) return res;
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
  }
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
