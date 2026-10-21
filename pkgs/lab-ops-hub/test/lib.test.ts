// Unit tests for the pure helpers in src/lib.ts — run with plain Node
// (type stripping): `npm test`. No Worker bindings are involved.
import assert from "node:assert/strict";
import { test } from "node:test";
import { computeBudget, fetchFollowingValidatedRedirects, validateCrawlUrl } from "../src/lib.ts";

test("validateCrawlUrl accepts public http(s) targets", () => {
  assert.equal(validateCrawlUrl("https://example.com/page").ok, true);
  assert.equal(validateCrawlUrl("http://developers.cloudflare.com/containers/").ok, true);
  assert.equal(validateCrawlUrl("https://[2606:4700:4700::1111]/").ok, true);
  assert.equal(validateCrawlUrl("https://8.8.8.8/").ok, true);
});

test("validateCrawlUrl refuses non-http schemes and junk", () => {
  assert.equal(validateCrawlUrl("ftp://example.com/").ok, false);
  assert.equal(validateCrawlUrl("file:///etc/passwd").ok, false);
  assert.equal(validateCrawlUrl("not a url").ok, false);
});

test("validateCrawlUrl refuses loopback / private / link-local targets (#441)", () => {
  const blocked = [
    "http://127.0.0.1/",
    "http://127.1/",
    "http://2130706433/", // WHATWG-normalises to 127.0.0.1
    "http://0x7f.1/", // hex form, also normalises to 127.0.0.1
    "http://10.0.0.5/",
    "http://172.16.0.9/",
    "http://192.168.1.1/",
    "http://169.254.169.254/latest/meta-data/", // cloud metadata
    "http://100.64.0.1/", // CGNAT
    "https://[::1]/",
    "https://[::ffff:127.0.0.1]/", // IPv4-mapped loopback
    "https://[fe80::1]/",
    "https://[fd00::1]/",
    "http://localhost:8787/",
    "http://printer.local/",
    "http://nas.internal/",
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
});

test("validateCrawlUrl refuses trailing-dot spellings of blocked hosts (#466)", () => {
  const blocked = ["http://localhost./", "http://foo.localhost./", "http://printer.local./", "http://nas.internal./"];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // A public name keeps its fully-qualified spelling.
  assert.equal(validateCrawlUrl("http://example.com./").ok, true);
});

function stubFetcher(routes: Record<string, { status: number; location?: string }>) {
  const seen: string[] = [];
  const fetchFn = async (url: string): Promise<Response> => {
    seen.push(url);
    const r = routes[url];
    if (!r) throw new Error(`unexpected fetch ${url}`);
    return new Response(null, {
      status: r.status,
      headers: r.location ? { location: r.location } : {},
    });
  };
  return { fetchFn, seen };
}

test("fetchFollowingValidatedRedirects follows public redirects, resolving relative Locations (#460)", async () => {
  const { fetchFn, seen } = stubFetcher({
    "https://example.com/start": { status: 302, location: "/next" },
    "https://example.com/next": { status: 301, location: "https://example.org/final" },
    "https://example.org/final": { status: 200 },
  });
  const res = await fetchFollowingValidatedRedirects("https://example.com/start", fetchFn);
  assert.equal(res.status, 200);
  assert.deepEqual(seen, ["https://example.com/start", "https://example.com/next", "https://example.org/final"]);
});

test("fetchFollowingValidatedRedirects refuses redirects to non-public targets (#460)", async () => {
  for (const target of [
    "http://127.0.0.1/admin",
    "http://169.254.169.254/latest/meta-data/",
    "http://192.168.1.1/",
    "https://[::1]/",
    "http://localhost:8787/",
    "http://localhost./",
    "file:///etc/passwd",
  ]) {
    const { fetchFn, seen } = stubFetcher({
      "https://example.com/start": { status: 302, location: target },
    });
    await assert.rejects(
      fetchFollowingValidatedRedirects("https://example.com/start", fetchFn),
      /refusing to follow redirect/,
      target,
    );
    // The refused target itself is never fetched.
    assert.deepEqual(seen, ["https://example.com/start"], target);
  }
});

test("fetchFollowingValidatedRedirects refuses a non-public start URL without fetching (#464)", async () => {
  for (const start of [
    "http://127.0.0.1/secret",
    "http://169.254.169.254/latest/meta-data/",
    "http://2130706433/", // WHATWG-normalises to 127.0.0.1
    "https://[::ffff:127.0.0.1]/", // IPv4-mapped loopback
    "http://localhost:8787/",
    "file:///etc/passwd",
    "not a url",
  ]) {
    const { fetchFn, seen } = stubFetcher({});
    await assert.rejects(fetchFollowingValidatedRedirects(start, fetchFn), /refusing to crawl/, start);
    // The start URL itself is never fetched.
    assert.deepEqual(seen, [], start);
  }
});

test("fetchFollowingValidatedRedirects bounds the hop count (#460)", async () => {
  const { fetchFn } = stubFetcher({
    "https://example.com/loop": { status: 302, location: "https://example.com/loop" },
  });
  await assert.rejects(fetchFollowingValidatedRedirects("https://example.com/loop", fetchFn, 3), /too many redirects/);
});

test("computeBudget reports $0 while usage is inside the included amounts", () => {
  const b = computeBudget({ worker_request: 120, d1_write: 40, queue_msg: 3 });
  assert.equal(b.estimated_overage_usd, 0);
  // d1 count = application writes + the ledger's own rows (120+40+3).
  assert.equal(b.month_to_date_self_counted.d1_rows_written.used, 40 + 163);
});

test("computeBudget computes a non-zero overage past an included amount (#442)", () => {
  // 20M worker requests: (20M - 10M) * $0.30/M = $3.00. The 20M ledger
  // rows also count as D1 writes but stay under the 50M D1 allowance.
  const b = computeBudget({ worker_request: 20_000_000 });
  assert.equal(b.month_to_date_self_counted.worker_requests.estimated_overage_usd, 3);
  assert.equal(b.month_to_date_self_counted.d1_rows_written.used, 20_000_000);
  assert.equal(b.estimated_overage_usd, 3);
});

test("computeBudget counts ledger rows toward D1 writes (#442)", () => {
  // 30M application writes + their 30M ledger rows = 60M D1 rows:
  // (60M - 50M) * $1.00/M = $10.00.
  const b = computeBudget({ d1_write: 30_000_000 });
  assert.equal(b.month_to_date_self_counted.d1_rows_written.used, 60_000_000);
  assert.equal(b.month_to_date_self_counted.d1_rows_written.estimated_overage_usd, 10);
  assert.equal(b.estimated_overage_usd, 10);
});
