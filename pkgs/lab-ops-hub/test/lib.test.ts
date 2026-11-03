// Unit tests for the pure helpers in src/lib.ts — run with plain Node
// (type stripping): `npm test`. No Worker bindings are involved.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  computeBudget,
  crawlArchiveKey,
  CRAWL_FETCH_TIMEOUT_MS,
  crawlFetcher,
  fetchFollowingValidatedRedirects,
  MAX_CRAWL_BODY_BYTES,
  parseJsonObjectBody,
  readResponseTextCapped,
  uploadKey,
  validateCrawlUrl,
  isMachineNameConflict,
  validateMachineBody,
  validateNoteBody,
  validateTaskBody,
} from "../src/lib.ts";

test("validateCrawlUrl accepts public http(s) targets", () => {
  assert.equal(validateCrawlUrl("https://example.com/page").ok, true);
  assert.equal(validateCrawlUrl("http://developers.cloudflare.com/containers/").ok, true);
  assert.equal(validateCrawlUrl("https://[2606:4700:4700::1111]/").ok, true);
  assert.equal(validateCrawlUrl("https://8.8.8.8/").ok, true);
});

test("validateCrawlUrl percent-encodes non-ASCII outside the host (#615)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: WHATWG UTF-8 percent-encoding in userinfo/path/query.
  const cases: Array<[string, string]> = [
    ["http://example.com/café", "http://example.com/caf%C3%A9"],
    ["http://example.com/?q=café", "http://example.com/?q=caf%C3%A9"],
    ["http://user:café@example.com/", "http://user:caf%C3%A9@example.com/"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
});

test("validateCrawlUrl percent-encodes C0 controls, space and DEL outside the host (#614)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: WHATWG percent-encodes C0 controls, space and DEL inside
  // the path/query/fragment/userinfo, and trims leading and trailing
  // C0-or-space from the whole input.
  const cases: Array<[string, string]> = [
    ["http://example.com/pa\x00th", "http://example.com/pa%00th"],
    ["http://example.com/pa th", "http://example.com/pa%20th"],
    ["http://example.com/?q=a\x7fb", "http://example.com/?q=a%7Fb"],
    ["\x00http://example.com/", "http://example.com/"],
    ["http://example.com/\x00", "http://example.com/"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
});

test("validateCrawlUrl returns dot segments, host case and ports normalized (#633)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: WHATWG resolves dot segments (literal and %2e spellings),
  // lowercases the host, and drops the scheme's default port.
  const cases: Array<[string, string]> = [
    ["http://example.com/a/../b", "http://example.com/b"],
    ["http://example.com/%2e%2e/admin", "http://example.com/admin"],
    ["http://example.com/a/%2e%2e/b", "http://example.com/b"],
    ["http://example.com/a/.", "http://example.com/a/"],
    ["http://example.com/a/..", "http://example.com/"],
    ["http://example.com//%2e", "http://example.com//"],
    ["HTTP://EXAMPLE.COM/Path", "http://example.com/Path"],
    ["http://example.com:80/x", "http://example.com/x"],
    ["https://example.com:443/x", "https://example.com/x"],
    ["http://example.com:080/x", "http://example.com/x"],
    ["http://example.com:08080/x", "http://example.com:8080/x"],
    ["http://user:pass@EXAMPLE.com:80/", "http://user:pass@example.com/"],
    // Data, not dot segments: double-encoded spellings stay verbatim.
    ["http://example.com/%252e%252e/admin", "http://example.com/%252e%252e/admin"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
});

test("validateCrawlUrl returns the root dot for both dot spellings (#647)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: WHATWG host parsing decodes "%2e" to "." and serialises
  // the DNS root dot back, so the literal and percent-encoded spellings
  // return the same dotted URL. (IP literals serialise without one.)
  const cases: Array<[string, string]> = [
    ["http://example.com./", "http://example.com./"],
    ["http://example.com./path", "http://example.com./path"],
    ["http://example.com%2e/", "http://example.com./"],
    ["http://example.com%2E/", "http://example.com./"],
    ["http://example.com%2e/path", "http://example.com./path"],
    ["http://sub.example.com%2e/", "http://sub.example.com./"],
    ["http://8.8.8.8./", "http://8.8.8.8/"],
    ["http://8.8.8.8%2e/", "http://8.8.8.8/"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
  assert.equal(validateCrawlUrl("http://localhost%2e/").ok, false);
  assert.equal(validateCrawlUrl("http://example.com%2e%2e/").ok, false);
});

test("validateCrawlUrl returns the ASCII dot for literal separator spellings (#660)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: WHATWG host parsing maps the literal UTS-46 separators
  // (U+3002, U+FF0E, U+FF61) to ".", so literal spellings return the
  // same dotted URL as their percent-encoded spellings — root dot and
  // interior separators alike. (IP literals serialise without a root dot.)
  const cases: Array<[string, string]> = [
    ["http://example.com\u3002/", "http://example.com./"],
    ["http://example.com\uff0e/", "http://example.com./"],
    ["http://example.com\uff61/", "http://example.com./"],
    ["http://example.com\u3002:8080/", "http://example.com.:8080/"],
    ["http://user@example.com\uff61/", "http://user@example.com./"],
    ["http://example\u3002com/", "http://example.com/"],
    ["http://example\uff0ecom/path", "http://example.com/path"],
    ["http://8\u30028\u30028\u30028/", "http://8.8.8.8/"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
  assert.equal(validateCrawlUrl("http://localhost\u3002/").ok, false);
  assert.equal(validateCrawlUrl("http://example.com\u3002\u3002/").ok, false);
});

test("validateCrawlUrl returns the host without UTS-46 ignored codepoints (#652)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: WHATWG domain-to-ASCII deletes the UTS-46 "ignored"
  // codepoints (U+00AD, U+034F, U+180E, U+200B, U+FEFF, U+2060-...) from
  // the host, so a spelling carrying one returns the host with the
  // codepoint deleted — root dot, port and userinfo preserved.
  const cases: Array<[string, string]> = [
    ["http://exa\u200bmple.com/", "http://example.com/"],
    ["http://exa\u00admple.com/", "http://example.com/"],
    ["http://exa\u180emple.com/", "http://example.com/"],
    ["http://exa\u2060mple.com/", "http://example.com/"],
    ["http://exa\ufeffmple.com/", "http://example.com/"],
    ["http://example.com\u200b/", "http://example.com/"],
    ["http://exa\u200bmple.com./", "http://example.com./"],
    ["http://example.com.\u200b/", "http://example.com./"],
    ["http://exa\u200bmple.com:8080/path?q=1#frag", "http://example.com:8080/path?q=1#frag"],
    ["http://user@exa\u200bmple.com/", "http://user@example.com/"],
    ["http://8\u200b.8.8.8/", "http://8.8.8.8/"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
  assert.equal(validateCrawlUrl("http://local\u200bhost/").ok, false);
  assert.equal(validateCrawlUrl("http://127\u200b.0.0.1/").ok, false);
});

test("validateCrawlUrl returns empty components normalized (#639)", () => {
  // The Worker's returned form is the reference the container validator
  // mirrors: "/" supplied for an empty path, the empty port's colon and
  // the empty userinfo's "@" dropped, and empty "?" / "#" markers kept
  // (the query/fragment is present but empty).
  const cases: Array<[string, string]> = [
    ["http://example.com", "http://example.com/"],
    ["https://example.com", "https://example.com/"],
    ["http://example.com:8080", "http://example.com:8080/"],
    ["http://example.com:/path", "http://example.com/path"],
    ["http://@example.com/", "http://example.com/"],
    ["http://example.com/path?", "http://example.com/path?"],
    ["http://example.com/path#", "http://example.com/path#"],
    ["http://example.com?", "http://example.com/?"],
    ["http://example.com/path?#", "http://example.com/path?#"],
    ["http://user@example.com", "http://user@example.com/"],
    // Empty query in front of a non-empty fragment: the "?" stays
    // before the "#" (#654 — the container's suffix re-attachment
    // returned "path#frag?" for these spellings).
    ["http://example.com/path?#frag", "http://example.com/path?#frag"],
    ["http://example.com?#frag", "http://example.com/?#frag"],
    ["http://example.com/a?#b?c", "http://example.com/a?#b?c"],
  ];
  for (const [raw, expected] of cases) {
    const r = validateCrawlUrl(raw);
    assert.equal(r.ok, true, raw);
    if (r.ok) assert.equal(r.url, expected, raw);
  }
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
    "https://[3fff::1]/", // documentation 3fff::/20 (RFC 9637) (#519)
    "https://[3fff:0fff:ffff:ffff:ffff:ffff:ffff:ffff]/", // last address in 3fff::/20 (#519)
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
    "https://[3fff:1000::1]/", // first address past 3fff::/20 (#519)
    "https://[3ffe::1]/", // global unicast below 3fff::/20 (#519)
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

test("validateCrawlUrl refuses the bare home.arpa apex (#608)", () => {
  // Every subdomain of home.arpa is refused via the ".home.arpa"
  // suffix, but the apex itself is the RFC 8375 special-use name for
  // the local home network — it needs the equality arm localhost has.
  const blocked = ["http://home.arpa/", "http://home.arpa./", "http://HOME.ARPA/", "http://router.home.arpa/"];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Controls: an ordinary public name still passes.
  assert.equal(validateCrawlUrl("http://example.com/").ok, true);
});

test("validateCrawlUrl strips tab/CR/LF before judging, like WHATWG (#611)", () => {
  // The URL parser removes every ASCII tab, LF and CR from the input
  // before parsing — even inside the scheme spelling — and the
  // returned URL carries none. The container validator must reach
  // the same verdicts and return the same stripped spelling.
  const T = "\t",
    N = "\n",
    R = "\r";
  const accepted: Array<[string, string]> = [
    [`http://exa${T}mple.com/`, "http://example.com/"],
    [`http://exa${N}mple.com/`, "http://example.com/"],
    [`http://exa${R}mple.com/`, "http://example.com/"],
    [`http://example.com${T}.evil.com/`, "http://example.com.evil.com/"],
    [`http://example.com${R}${N}.evil.com/`, "http://example.com.evil.com/"],
    [`http://example.com/pa${T}th`, "http://example.com/path"],
    [`ht${T}tp://example.com/`, "http://example.com/"],
  ];
  for (const [u, normalized] of accepted) {
    const r = validateCrawlUrl(u);
    assert.equal(r.ok, true, u);
    if (r.ok) assert.equal(r.url, normalized, u);
  }
  for (const u of [`http://127${T}.0.0.1/`, `http://192.168${N}.0.1/`, `http://local${R}host/`]) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
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

test("validateCrawlUrl refuses IDNA-invalid hosts (#583)", () => {
  // WHATWG domain-to-ASCII refuses labels that are not valid IDNA
  // labels at all — unassigned codepoints, labels mixing LTR and RTL
  // characters, labels beginning with a combining mark. The container
  // validator now pins the same verdicts (#583).
  const blocked = [
    "http://\u0378ample.com/",
    "http://ex\u05d0ample.com/",
    "http://local\u05d0host/",
    "http://\u0301abc.com/",
    "http://ex\u066bample.com/",
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Controls: pure RTL labels and ordinary IDN stay accepted.
  assert.equal(validateCrawlUrl("http://\u05d0\u05d0.com/").ok, true);
  assert.equal(validateCrawlUrl("http://münchen.de/").ok, true);
});

test("validateCrawlUrl refuses Arabic numbers outside RTL labels (#595)", () => {
  // A character of bidi class AN (Arabic number) is only valid
  // inside an RTL label, and never alongside EN (European number):
  // the astral AN codepoints (Hanifi Rohingya digits U+10D30-U+10D39,
  // Rumi digits U+10E60-U+10E7E) were certified by the container
  // validator in Latin labels. It now pins the same verdicts (#595).
  const blocked = [
    "http://ex\u{10d30}ample.com/",
    "http://ex\u{10e60}ample.com/",
    "http://\u{10d30}.com/",
    "http://1\u{10d30}.com/",
    "http://\u05d0\u{10d30}1.com/",
    "http://\u066b.com/",
  ];
  // (The A-label spelling xn--example-eh36b is refused under Bun but
  // accepted by Node's URL parser, so it is pinned only in the
  // container suite.)
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Controls: an AN digit inside a genuine RTL label stays accepted.
  assert.equal(validateCrawlUrl("http://\u05d0\u{10d30}\u05d0.com/").ok, true);
  assert.equal(validateCrawlUrl("http://\u0627\u{10d30}\u0627.com/").ok, true);
  assert.equal(validateCrawlUrl("http://\u{10d00}\u{10d30}.com/").ok, true);
});

test("validateCrawlUrl judges Arabic-Indic digits by their bidi context (#605)", () => {
  // U+0660-U+0669 are bidi class AN, like the astral digits in the
  // #595 test: valid inside a genuine RTL label, refused outside
  // one. The container validator's blanket range refused the RTL
  // spellings too; it now pins the same contextual verdicts (#605).
  const blocked = [
    "http://\u0661.com/",
    "http://\u0661\u0662\u0667.\u0660.\u0660.\u0661/",
    "http://ex\u0661ample.com/",
    "http://\u05d0\u06611.com/",
    "http://\u0661\u0627.com/",
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Controls: the digits inside RTL labels, raw and A-label form.
  assert.equal(validateCrawlUrl("http://\u0627\u0661.com/").ok, true);
  assert.equal(validateCrawlUrl("http://\u0645\u0631\u062d\u0628\u0627\u0661.com/").ok, true);
  assert.equal(validateCrawlUrl("http://\u05d0\u0661.com/").ok, true);
  assert.equal(validateCrawlUrl("http://xn--mgb0j.com/").ok, true);
  assert.equal(validateCrawlUrl("http://xn--mgbbop3hzh.com/").ok, true);
  assert.equal(validateCrawlUrl("http://xn--4db40a.com/").ok, true);
});

test("validateCrawlUrl judges U+0345 by its UTS-46 mapping (#596)", () => {
  // UTS-46 maps U+0345 (combining Greek ypogegrammeni) to U+03B9
  // (ι, bidi class L): inside an RTL label the mapped form mixes
  // L with R and is refused. The container validator judged the
  // unmapped mark (bidi NSM) and certified these hosts; it now
  // pins the same verdicts (#596).
  assert.equal(validateCrawlUrl("http://\u05d0\u0345\u05d0.com/").ok, false);
  assert.equal(validateCrawlUrl("http://\u05d0\u0345.com/").ok, false);
  // (The A-label spelling xn--jua51kba is refused under Bun but
  // accepted by Node's URL parser, so it is pinned only in the
  // container suite.)
  // Controls: LTR/Greek contexts and the label-start spelling
  // (U+0345 maps to a letter, so the label starts with one) stay
  // accepted.
  assert.equal(validateCrawlUrl("http://ex\u0345ample.com/").ok, true);
  assert.equal(validateCrawlUrl("http://\u03b1\u0345.com/").ok, true);
  assert.equal(validateCrawlUrl("http://\u0345abc.com/").ok, true);
});

test("validateCrawlUrl refuses hosts whose mapping introduces a separator (#593)", () => {
  // U+FE12's compatibility mapping is U+3002, itself a UTS-46 label
  // separator: WHATWG domain-to-ASCII refuses the host outright
  // rather than judging it as two labels. The container validator
  // pins the same verdicts (#593).
  const blocked = [
    "http://ex\ufe12ample.com/",
    "http://a\ufe12b.com/",
    "http://example\ufe12com/",
    "http://ex%ef%b8%92ample.com/",
  ];
  // (The A-label spelling xn--example-t673b is refused under Bun but
  // accepted by Node's URL parser, so it is pinned only in the
  // container suite.)
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Control: a literal U+3002 separator spelling stays accepted.
  assert.equal(validateCrawlUrl("http://münchen。de/").ok, true);
});

test("validateCrawlUrl refuses overlong DNS names (#513)", () => {
  // RFC 1035 §2.3.4: no label over 63 octets, no name over 253
  // characters of text. Neither can ever resolve; refuse up front.
  const blocked = [
    `http://${"b".repeat(64)}.com/`,
    `http://${["a".repeat(63), "a".repeat(63), "a".repeat(63), "a".repeat(63), "com"].join(".")}/`,
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Controls at the limit stay allowed: a 63-char label, and a
  // 253-char name (63+63+63+61 + 3 dots).
  assert.equal(validateCrawlUrl(`http://${"b".repeat(63)}.com/`).ok, true);
  assert.equal(
    validateCrawlUrl(`http://${["a".repeat(63), "a".repeat(63), "a".repeat(63), "a".repeat(61)].join(".")}/`).ok,
    true,
  );
});

test("validateCrawlUrl judges percent-encoded hosts in their decoded form (#529)", () => {
  // WHATWG percent-decodes the host before judging it; the container
  // validator must reach the same verdicts on the decoded form.
  const accepted: Array<[string, string]> = [
    ["http://%65xample.com/", "http://example.com/"],
    ["http://example%2ecom/", "http://example.com/"],
    ["http://8%2e8%2e8%2e8/", "http://8.8.8.8/"],
    ["http://%38%2e8.8.8/", "http://8.8.8.8/"],
  ];
  for (const [u, normalized] of accepted) {
    const r = validateCrawlUrl(u);
    assert.equal(r.ok, true, u);
    if (r.ok) assert.equal(r.url, normalized, u);
  }
  const blocked = [
    "http://%6cocalhost/", // decodes to localhost
    "http://127%2e0%2e0%2e1/", // decodes to 127.0.0.1
    "http://foo%2elocalhost/", // decodes to foo.localhost
    "http://127.0.0.1%09/", // decodes to a forbidden tab
    "http://8.8.8.8%zz/", // invalid percent escape
    "http://example.com%/", // trailing bare %
    "https://[fe80::1%25eth0]/", // IPv6 zone id
    "http://exam%2fple.com/", // decodes to a host delimiter
    "http://example%252ecom/", // double-encoded spelling
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
});

test("validateCrawlUrl refuses percent-encoded hosts whose NFKC mapping is a delimiter (#567)", () => {
  // Parity battery for the container fix: a percent-encoded
  // compatibility character decodes to a codepoint whose NFKC
  // mapping is a host delimiter (or expands to one). The Worker's
  // WHATWG parser refuses every spelling outright; the container
  // originally spliced the mapped delimiter into the returned URL
  // and fetched an unjudged host (localhost / private addresses).
  const blocked = [
    "http://localhost%ef%bc%8fexample.com/", // U+FF0F -> /
    "http://127.0.0.1%ef%bc%8fexample.com/", // U+FF0F -> /
    "http://192.168.0.1%ef%bc%8fadmin/", // U+FF0F -> /
    "http://exam%ef%bc%8fple.com/", // U+FF0F -> /
    "http://exam%ef%bc%9aple.com/", // U+FF1A -> :
    "http://exam%ef%bc%a0ple.com/", // U+FF20 -> @
    "http://exam%ef%bc%83ple.com/", // U+FF03 -> #
    "http://exam%ef%bc%9fple.com/", // U+FF1F -> ?
    "http://exam%ef%bc%bcple.com/", // U+FF3C -> backslash
    "http://exam%ef%bc%bbple.com/", // U+FF3B -> [
    "http://exam%ef%bc%bdple.com/", // U+FF3D -> ]
    "http://example%ef%bc%85com/", // U+FF05 -> %
    "http://%e2%84%80.com/", // U+2100 expands to a/c
    "http://%e2%84%85.com/", // U+2105 expands to c/o
  ];
  for (const u of blocked) {
    assert.equal(validateCrawlUrl(u).ok, false, u);
  }
  // Control: U+FF5E maps to ~, not a delimiter — still accepted,
  // and the returned URL re-parses to the judged host.
  const tilde = validateCrawlUrl("http://exam%ef%bd%9eple.com/");
  assert.equal(tilde.ok, true);
  if (tilde.ok) assert.equal(tilde.url, "http://exam~ple.com/");
});

test("fetchFollowingValidatedRedirects follows a percent-encoded public redirect target (#529)", async () => {
  const { fetchFn, seen } = stubFetcher({
    "https://example.com/start": { status: 302, location: "http://%65xample.org/final" },
    "http://example.org/final": { status: 200 },
  });
  const res = await fetchFollowingValidatedRedirects("https://example.com/start", fetchFn);
  assert.equal(res.status, 200);
  // The redirect target is fetched in its decoded (normalised) form.
  assert.deepEqual(seen, ["https://example.com/start", "http://example.org/final"]);
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

test("fetchFollowingValidatedRedirects fetches userinfo URLs in their validated form (#622)", async () => {
  // The Worker side of the #622 pair: validateCrawlUrl accepts
  // userinfo targets and this helper hands the validated form —
  // userinfo intact, for the platform fetch to turn into an
  // Authorization: Basic header — to the fetcher, for the start URL
  // and for redirect targets alike. (The container fetcher used to
  // 502 every such URL; its side is pinned in the Python suite.)
  const { fetchFn, seen } = stubFetcher({
    "https://user:pass@example.com/start": {
      status: 302,
      location: "https://caf%C3%A9@example.org/final",
    },
    "https://caf%C3%A9@example.org/final": { status: 200 },
  });
  const res = await fetchFollowingValidatedRedirects("https://user:pass@example.com/start", fetchFn);
  assert.equal(res.status, 200);
  assert.deepEqual(seen, ["https://user:pass@example.com/start", "https://caf%C3%A9@example.org/final"]);
});

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

test("fetchFollowingValidatedRedirects matches urllib's ten-hop boundary", async () => {
  const tenHopRoutes: Record<string, { status: number; location?: string }> = {};
  for (let hop = 0; hop < 10; hop++) {
    tenHopRoutes[`https://example.com/hop/${hop}`] = {
      status: 302,
      location: `/hop/${hop + 1}`,
    };
  }
  tenHopRoutes["https://example.com/hop/10"] = { status: 200 };
  const tenHop = stubFetcher(tenHopRoutes);
  assert.equal((await fetchFollowingValidatedRedirects("https://example.com/hop/0", tenHop.fetchFn)).status, 200);
  assert.equal(tenHop.seen.length, 11);

  const elevenHopRoutes: Record<string, { status: number; location?: string }> = {};
  for (let hop = 0; hop < 11; hop++) {
    elevenHopRoutes[`https://example.com/hop/${hop}`] = {
      status: 302,
      location: `/hop/${hop + 1}`,
    };
  }
  elevenHopRoutes["https://example.com/hop/11"] = { status: 200 };
  const elevenHop = stubFetcher(elevenHopRoutes);
  await assert.rejects(
    fetchFollowingValidatedRedirects("https://example.com/hop/0", elevenHop.fetchFn),
    /too many redirects/,
  );
  assert.equal(elevenHop.seen.length, 11);
});

test("fetchFollowingValidatedRedirects follows only urllib redirect statuses", async () => {
  for (const status of [301, 302, 303, 307, 308]) {
    const { fetchFn } = stubFetcher({
      "https://example.com/start": { status, location: "/final" },
      "https://example.com/final": { status: 200 },
    });
    assert.equal((await fetchFollowingValidatedRedirects("https://example.com/start", fetchFn)).status, 200, `${status}`);
  }

  for (const status of [300, 304]) {
    const { fetchFn, seen } = stubFetcher({
      "https://example.com/start": { status, location: "/final" },
      "https://example.com/final": { status: 200 },
    });
    await assert.rejects(
      fetchFollowingValidatedRedirects("https://example.com/start", fetchFn),
      /unsupported redirect status/,
      `${status}`,
    );
    assert.deepEqual(seen, ["https://example.com/start"], `${status}`);
  }
});

test("fetchFollowingValidatedRedirects enforces urllib's per-URL repeat limit", async () => {
  const { fetchFn, seen } = stubFetcher({
    "https://example.com/a": { status: 302, location: "/b" },
    "https://example.com/b": { status: 302, location: "/a" },
  });
  await assert.rejects(fetchFollowingValidatedRedirects("https://example.com/a", fetchFn), /repeated/);
  // urllib permits four redirects to a URL and refuses the fifth.
  assert.equal(seen.length, 9);
});

test("crawlFetcher attaches a fresh AbortSignal, manual redirect and the crawler UA to every hop (#524)", async () => {
  const seen: RequestInit[] = [];
  const fetchFn = crawlFetcher(60_000, async (_u, init) => {
    seen.push(init);
    return new Response("ok");
  });
  await fetchFn("https://example.com/a");
  await fetchFn("https://example.com/b");
  assert.equal(seen.length, 2);
  for (const init of seen) {
    assert.ok(init.signal instanceof AbortSignal, "every hop must carry the timeout signal (#500)");
    assert.equal(init.signal.aborted, false);
    assert.equal(init.redirect, "manual");
    assert.equal((init.headers as Record<string, string>)["user-agent"], "LabOpsHub/0.1 (+personal crawler)");
  }
  // A fresh signal per hop: a shared one would abort later hops once the
  // first hop's deadline passes.
  assert.notEqual(seen[0].signal, seen[1].signal);
  assert.equal(CRAWL_FETCH_TIMEOUT_MS, 20_000);
});

test("crawlFetcher rejects a fetch that stalls before responding when the timeout fires (#524)", async () => {
  const fetchFn = crawlFetcher(
    30,
    (_u, init) =>
      // A server that accepts the connection and never answers: only the
      // abort signal can end this fetch.
      new Promise<Response>((_resolve, reject) => {
        init.signal?.addEventListener("abort", () => reject(init.signal?.reason), { once: true });
      }),
  );
  await assert.rejects(fetchFn("https://example.com/stall"), /abort|timeout/i);
});

test("crawlFetcher's signal bounds a slow-drip body read through readResponseTextCapped (#524)", async () => {
  const fetchFn = crawlFetcher(30, async (_u, init) => {
    // One chunk, then the body stalls: the stream errors when the hop's
    // signal fires, exactly how a real fetch ties the body to the signal.
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("<title>partial"));
        init.signal?.addEventListener("abort", () => controller.error(init.signal?.reason), { once: true });
      },
    });
    return new Response(body);
  });
  const res = await fetchFn("https://example.com/drip");
  await assert.rejects(readResponseTextCapped(res), /abort|timeout/i);
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

// --- JSON POST body validation (#2480) -------------------------------------

const post = (body: string) =>
  new Request("http://lab-ops-hub.test/api/x", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body,
  });

test("parseJsonObjectBody rejects malformed JSON (all four JSON POST routes share it)", async () => {
  for (const raw of ["{not json", "", "{", '{"a":}']) {
    const r = await parseJsonObjectBody(post(raw));
    assert.equal(r.ok, false, raw);
    if (!r.ok) assert.equal(r.error, "invalid JSON", raw);
  }
});

test("parseJsonObjectBody rejects non-object JSON", async () => {
  for (const raw of ["[]", "[1,2]", '"hello"', "42", "null", "true"]) {
    const r = await parseJsonObjectBody(post(raw));
    assert.equal(r.ok, false, raw);
    if (!r.ok) assert.equal(r.error, "JSON object required", raw);
  }
});

test("parseJsonObjectBody accepts a JSON object and returns it untouched", async () => {
  const r = await parseJsonObjectBody(post('{"name":"  padded  ","n":1}'));
  assert.equal(r.ok, true);
  if (r.ok) assert.deepEqual(r.body, { name: "  padded  ", n: 1 });
});

test("validateMachineBody requires a nonblank string name", () => {
  for (const b of [{}, { name: null }, { name: 42 }, { name: "" }, { name: "   " }, { name: ["x"] }]) {
    const r = validateMachineBody(b as Record<string, unknown>);
    assert.equal(r.ok, false, JSON.stringify(b));
    if (!r.ok) assert.equal(r.error, "name required");
  }
  const ok = validateMachineBody({ name: "  amd-6  " });
  assert.equal(ok.ok, true);
  if (ok.ok) assert.equal(ok.values.name, "  amd-6  "); // original preserved, not trimmed
});

test("validateNoteBody requires title, then body", () => {
  // Both missing: title is reported first.
  const both = validateNoteBody({});
  assert.equal(both.ok, false);
  if (!both.ok) assert.equal(both.error, "title required");
  for (const b of [
    { title: null, body: "x" },
    { title: " ", body: "x" },
    { title: 7, body: "x" },
  ]) {
    const r = validateNoteBody(b as Record<string, unknown>);
    assert.equal(r.ok, false, JSON.stringify(b));
    if (!r.ok) assert.equal(r.error, "title required");
  }
  for (const b of [
    { title: "t" },
    { title: "t", body: null },
    { title: "t", body: "" },
    { title: "t", body: "  " },
    { title: "t", body: 9 },
  ]) {
    const r = validateNoteBody(b as Record<string, unknown>);
    assert.equal(r.ok, false, JSON.stringify(b));
    if (!r.ok) assert.equal(r.error, "body required");
  }
  const ok = validateNoteBody({ title: " t ", body: " b " });
  assert.equal(ok.ok, true);
  if (ok.ok) assert.deepEqual(ok.values, { title: " t ", text: " b " });
});

test("validateTaskBody requires a nonblank string title", () => {
  for (const b of [{}, { title: null }, { title: false }, { title: "" }, { title: "\t\n " }]) {
    const r = validateTaskBody(b as Record<string, unknown>);
    assert.equal(r.ok, false, JSON.stringify(b));
    if (!r.ok) assert.equal(r.error, "title required");
  }
  const ok = validateTaskBody({ title: "do it" });
  assert.equal(ok.ok, true);
  if (ok.ok) assert.equal(ok.values.title, "do it");
});

test("uploadKey uniquifies same-millisecond uploads of the same name (#2478)", () => {
  const now = 1793100000000;
  const a = uploadKey("report.pdf", now, "rand-a");
  const b = uploadKey("report.pdf", now, "rand-b");
  assert.notEqual(a, b);
});

test("uploadKey separates filenames that sanitize to the same string (#2478)", () => {
  const now = 1793100000000;
  const a = uploadKey("my file.pdf", now, "rand-a");
  const b = uploadKey("my:file.pdf", now, "rand-b");
  assert.notEqual(a, b);
  // The sanitized tails really are identical — only the rand segment differs.
  assert.ok(a.endsWith("-my_file.pdf"));
  assert.ok(b.endsWith("-my_file.pdf"));
});

test("uploadKey is deterministic for identical inputs and keeps the uploads/ shape (#2478)", () => {
  const now = 1793100000000;
  assert.equal(uploadKey("report.pdf", now, "rand-a"), uploadKey("report.pdf", now, "rand-a"));
  assert.equal(uploadKey("report.pdf", now, "rand-a"), "uploads/1793100000000-rand-a-report.pdf");
  assert.equal(uploadKey("my file.pdf", now, "rand-a"), "uploads/1793100000000-rand-a-my_file.pdf");
});

test("uploadKey defaults rand to a UUID so bare calls never repeat a key (#2478)", () => {
  const now = 1793100000000;
  const a = uploadKey("report.pdf", now);
  const b = uploadKey("report.pdf", now);
  assert.notEqual(a, b);
  assert.match(a, /^uploads\/1793100000000-[0-9a-f-]{36}-report\.pdf$/);
});

test("crawlArchiveKey: same-millisecond derivations with distinct uniques never collide (#2503)", () => {
  const ms = 1730000000000;
  const a = crawlArchiveKey(ms, "11111111-1111-4111-8111-111111111111");
  const b = crawlArchiveKey(ms, "22222222-2222-4222-8222-222222222222");
  assert.equal(a, "crawls/1730000000000-11111111-1111-4111-8111-111111111111.json");
  assert.equal(b, "crawls/1730000000000-22222222-2222-4222-8222-222222222222.json");
  assert.notEqual(a, b);
  const seen = new Set<string>();
  for (let i = 0; i < 1000; i++) {
    const k = crawlArchiveKey(ms, `unique-${i}`);
    assert.match(k, /^crawls\/1730000000000-.+\.json$/);
    assert.ok(!seen.has(k), k);
    seen.add(k);
  }
});

test("isMachineNameConflict recognises only the machines.name UNIQUE violation (#2631)", () => {
  assert.equal(isMachineNameConflict(new Error("UNIQUE constraint failed: machines.name")), true);
  assert.equal(isMachineNameConflict(new Error("D1_ERROR: UNIQUE constraint failed: machines.name")), true);
  // Other constraints and other failures stay on the 500 path.
  assert.equal(isMachineNameConflict(new Error("UNIQUE constraint failed: files.r2_key")), false);
  assert.equal(isMachineNameConflict(new Error("no such table: machines")), false);
  assert.equal(isMachineNameConflict(new Error("UNIQUE constraint failed")), false);
  assert.equal(isMachineNameConflict("UNIQUE constraint failed: machines.name"), true);
  assert.equal(isMachineNameConflict(null), false);
});
