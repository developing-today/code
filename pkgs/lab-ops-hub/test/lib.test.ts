// Unit tests for the pure helpers in src/lib.ts — run with plain Node
// (type stripping): `npm test`. No Worker bindings are involved.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  computeBudget,
  fetchFollowingValidatedRedirects,
  MAX_CRAWL_BODY_BYTES,
  readResponseTextCapped,
  validateCrawlUrl,
} from "../src/lib.ts";

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

test("validateCrawlUrl judges NAT64 / 6to4 literals by their embedded IPv4 (#472)", () => {
  const blocked = [
    "https://[64:ff9b::7f00:1]/", // NAT64 embedding 127.0.0.1
    "https://[64:ff9b::a9fe:a9fe]/", // NAT64 embedding 169.254.169.254 (cloud metadata)
    "https://[64:ff9b::c0a8:101]/", // NAT64 embedding 192.168.1.1
    "https://[2002:7f00:1::]/", // 6to4 embedding 127.0.0.1
    "https://[2002:a9fe:a9fe::]/", // 6to4 embedding 169.254.169.254
    "https://[::ffff:7f00:1]/", // IPv4-mapped loopback, hex-group spelling
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  const allowed = [
    "https://[64:ff9b::808:808]/", // NAT64 embedding 8.8.8.8
    "https://[2002:808:808::]/", // 6to4 embedding 8.8.8.8
    "https://[::ffff:8.8.8.8]/", // IPv4-mapped public
  ];
  for (const u of allowed) {
    assert.equal(validateCrawlUrl(u).ok, true, u);
  }
});

test("validateCrawlUrl agrees with the container validator on IPv4 ranges (#476)", () => {
  // Multicast is not a public unicast target, and the /24 special ranges
  // must not widen to /16s: their public neighbours stay crawlable.
  const blocked = [
    "http://224.0.0.1/", // multicast 224.0.0.0/4
    "http://230.1.2.3/", // multicast
    "http://239.255.255.255/", // multicast
    "https://[::ffff:224.0.0.1]/", // IPv4-mapped multicast
    "http://192.0.0.1/", // 192.0.0.0/24
    "http://192.0.2.1/", // TEST-NET-1
    "http://192.88.99.1/", // 6to4 relay anycast (deprecated)
    "http://198.51.100.1/", // TEST-NET-2
    "http://203.0.113.1/", // TEST-NET-3
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  const allowed = [
    "http://192.0.0.9/", // PCP anycast exception in 192.0.0.0/24
    "http://192.0.0.10/", // DNS anycast exception in 192.0.0.0/24
    "http://192.0.1.1/", // public neighbour of 192.0.0.0/24 + 192.0.2.0/24
    "http://192.88.0.1/", // public neighbour of 192.88.99.0/24
    "http://198.51.0.1/", // public neighbour of TEST-NET-2
    "http://203.0.0.1/", // public neighbour of TEST-NET-3
    "http://8.8.8.8/", // ordinary public
  ];
  for (const u of allowed) {
    assert.equal(validateCrawlUrl(u).ok, true, u);
  }
});

test("validateCrawlUrl agrees with the container validator on special IPv6 ranges (#474)", () => {
  // Verdicts must match the Python validator's suite case for case:
  // multicast/reserved/site-local/SRv6 space is not global unicast, and
  // the 2001::/23 IANA special-purpose block is non-global except its
  // six global entries.
  const blocked = [
    "https://[ff00::1]/", // multicast
    "https://[4000::1]/", // reserved, outside 2000::/3
    "https://[fec0::1]/", // site-local (deprecated)
    "https://[5f00::1]/", // SRv6 SIDs
    "https://[100:1::1]/", // outside the 100::/64 discard prefix
    "https://[2001::1]/", // Teredo 2001::/32
    "https://[2001:2::1]/", // benchmarking 2001:2::/48
    "https://[2001:10::1]/", // ORCHID 2001:10::/28
    "https://[2001:40::1]/", // 2001::/23 special-purpose
    "https://[2001:50::1]/", // 2001::/23 special-purpose
    "https://[2001:1::3]/", // 2001:1::/48 is global only at ::1 and ::2
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  const allowed = [
    "https://[2001:1::1]/", // PCP anycast
    "https://[2001:1::2]/", // PCP anycast
    "https://[2001:3::1]/", // AMT 2001:3::/32
    "https://[2001:4:112::1]/", // AS112 2001:4:112::/48
    "https://[2001:20::1]/", // ORCHIDv2 2001:20::/28
    "https://[2001:30::1]/", // ORCHIDv2 2001:30::/28
    "https://[2001:4860:4860::8888]/", // ordinary global unicast
  ];
  for (const u of allowed) {
    assert.equal(validateCrawlUrl(u).ok, true, u);
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

test("validateCrawlUrl refuses empty-label hosts (#511)", () => {
  // A second trailing dot defeats the one-dot strip (#466): "localhost.."
  // becomes "localhost." and misses the blocked-host checks. An interior
  // or leading empty label is not a valid DNS name either.
  const blocked = [
    "http://localhost../",
    "http://127.0.0.1../",
    "http://foo.localhost../",
    "http://example..com/",
    "http://.example.com/",
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Controls: single root dot and ordinary names still pass.
  assert.equal(validateCrawlUrl("http://example.com./").ok, true);
  assert.equal(validateCrawlUrl("http://example.com/").ok, true);
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

test("readResponseTextCapped returns a body under the cap whole (#481)", async () => {
  const res = new Response("<title>hi</title> hello");
  assert.equal(await readResponseTextCapped(res, 100), "<title>hi</title> hello");
});

test("readResponseTextCapped truncates at the cap and stops pulling the stream (#481)", async () => {
  let pulled = 0;
  let cancelled = false;
  const chunk = new Uint8Array(100).fill(97); // "a" * 100
  const body = new ReadableStream<Uint8Array>({
    pull(controller) {
      pulled += chunk.byteLength;
      controller.enqueue(chunk);
    },
    cancel() {
      cancelled = true;
    },
  });
  const text = await readResponseTextCapped(new Response(body), 250);
  assert.equal(text, "a".repeat(250));
  assert.equal(cancelled, true);
  // The reader stopped at the cap instead of draining the endless stream.
  assert.ok(pulled <= 300, `pulled ${pulled} bytes for a 250-byte cap`);
});

test("readResponseTextCapped decodes a character split at the cap as a replacement (#481)", async () => {
  // "é" is 0xC3 0xA9 in UTF-8; a 2-byte cap keeps only the first byte.
  const res = new Response(new TextEncoder().encode("aébc"));
  assert.equal(await readResponseTextCapped(res, 2), "a�");
});

test("the crawl body cap matches the container's 2,000,000-byte read (#481)", () => {
  assert.equal(MAX_CRAWL_BODY_BYTES, 2_000_000);
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
