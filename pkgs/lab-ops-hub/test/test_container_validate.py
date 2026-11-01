# Tests for the container crawler's validate_crawl_url (container/app.py).
# The TypeScript validator has its own suite (lib.test.ts); the Python copy
# is the component that actually fetches, so its policy is pinned here.
# Run with plain Python: `python3 -m unittest discover -s test -p "test_*.py"`
# from pkgs/lab-ops-hub (stdlib only).
import importlib.util
import pathlib
import unittest

_APP_PATH = pathlib.Path(__file__).resolve().parent.parent / "container" / "app.py"
_spec = importlib.util.spec_from_file_location("labops_container_app", _APP_PATH)
app = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(app)

validate_crawl_url = app.validate_crawl_url


class TestValidateCrawlUrl(unittest.TestCase):
    def assertRefused(self, url):
        with self.assertRaises(ValueError, msg=url):
            validate_crawl_url(url)

    def assertAccepted(self, url):
        self.assertEqual(validate_crawl_url(url), url)

    def test_backslash_ends_the_authority_like_whatwg(self):
        # A raw backslash is a path delimiter to WHATWG for http/https,
        # so it ends the authority: the host of
        # "http://127.0.0.1\\@example.com/" is 127.0.0.1, not the
        # example.com urlparse reads after the "@". Judge the host
        # WHATWG names, in both directions (#577).
        B = chr(92)
        for url in (
            "http://127.0.0.1" + B + "@example.com/",
            "http://localhost" + B + "@example.com/",
            "http://192.168.0.1" + B + "@example.com/",
        ):
            self.assertRefused(url)
        # The mirror spelling names example.com to WHATWG; the
        # validated (and fetched) form is the normalized spelling,
        # whose urlparse host is the judged host.
        self.assertEqual(
            validate_crawl_url("http://example.com" + B + "@127.0.0.1/"),
            "http://example.com/@127.0.0.1/",
        )
        # Backslash slash-runs after the scheme name example.com too.
        self.assertEqual(
            validate_crawl_url("http:" + B + B + "example.com/"),
            "http://example.com/",
        )
        self.assertEqual(
            validate_crawl_url("http:/" + B + "example.com/"),
            "http://example.com/",
        )
        # A percent-encoded %5C is data, not a delimiter: the spelling
        # is judged and returned exactly as before.
        self.assertAccepted("http://example.com/%5c@127.0.0.1/")

    def test_backslash_in_query_and_fragment_is_data(self):
        # #620: the #577 replacement is structural only up to the first
        # "?" or "#" — WHATWG keeps a backslash in the query/fragment
        # verbatim, and the Worker fetches it that way.
        v = app.validate_crawl_url
        self.assertEqual(v(r"http://example.com/?q=a\b"), r"http://example.com/?q=a\b")
        self.assertEqual(v(r"http://example.com/#a\b"), r"http://example.com/#a\b")
        self.assertEqual(
            v(r"https://example.com/?q=a\b\c#x\y"), r"https://example.com/?q=a\b\c#x\y"
        )
        # Control: a path backslash still normalizes, and an authority
        # backslash before a query still ends the authority (#577).
        self.assertEqual(v(r"http://example.com/a\b"), "http://example.com/a/b")
        with self.assertRaises(ValueError):
            v(r"http://127.0.0.1\@example.com/?q=a\b")

    def test_redirect_handler_refuses_backslash_loopback_target(self):
        # Redirect targets are validated by this validator alone
        # (#577): a Location spelling whose WHATWG host is loopback
        # must be refused, not followed as its post-"@" tail.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        with self.assertRaises(ValueError):
            handler.redirect_request(
                req,
                None,
                302,
                "Found",
                {},
                "http://127.0.0.1" + chr(92) + "@example.com/",
            )

    def test_slash_run_authority_spellings_name_the_whatwg_host(self):
        # WHATWG consumes the whole delimiter run after the scheme
        # colon (0, 1, 3+ slashes, and backslash runs after the #577
        # replacement); urlparse only sees a netloc behind exactly
        # "//", so those spellings were refused as hostless even for
        # public targets (#586). Every run length now validates — and
        # is fetched — as the exactly-two spelling.
        B = chr(92)
        for raw in (
            "http:example.com/x",
            "http:/example.com/x",
            "http:///example.com/x",
            "http:////example.com/x",
            "http:" + B + "example.com/x",
            "http:" + B * 3 + "example.com/x",
            "http://" + B + "example.com/x",
            "https:example.com/x",
            "https:///example.com/x",
        ):
            scheme = raw.split(":", 1)[0]
            self.assertEqual(validate_crawl_url(raw), f"{scheme}://example.com/x", raw)
        # Blocked hosts in slash-run spellings are refused by the
        # ordinary checks (non-public / local), not as hostless URLs.
        for url in (
            "http:///127.0.0.1/x",
            "http:localhost/x",
            "https:" + B * 3 + "192.168.0.1/x",
            "http:/10.0.0.5/x",
        ):
            with self.assertRaises(ValueError, msg=url) as cm:
                validate_crawl_url(url)
            self.assertNotIn("no host", str(cm.exception), url)

    def test_redirect_handler_refuses_slash_run_loopback_target(self):
        # Redirect targets are validated by this validator alone: a
        # slash-run Location naming loopback must be refused, not
        # passed through as hostless-or-public (#586).
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        with self.assertRaises(ValueError):
            handler.redirect_request(
                req, None, 302, "Found", {}, "http:///127.0.0.1/admin"
            )

    def test_tab_cr_lf_are_stripped_before_judging_like_whatwg(self):
        # WHATWG removes every ASCII tab, LF and CR from the input
        # before parsing — even inside the scheme spelling — and the
        # URL it produces carries none. urlparse handed them through
        # verbatim, so accepted URLs were returned (and fetched) with
        # control characters in the host: the fetched host was never
        # the judged host (#611). The stripped spelling is now judged
        # and returned, matching the Worker.
        T, N, R = chr(9), chr(10), chr(13)
        for raw, expected in (
            (f"http://exa{T}mple.com/", "http://example.com/"),
            (f"http://exa{N}mple.com/", "http://example.com/"),
            (f"http://exa{R}mple.com/", "http://example.com/"),
            (f"http://example.com{T}.evil.com/", "http://example.com.evil.com/"),
            (f"http://example.com{R}{N}.evil.com/", "http://example.com.evil.com/"),
            (f"http://example.com/pa{T}th", "http://example.com/path"),
            (f"ht{T}tp://example.com/", "http://example.com/"),
        ):
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Blocked hosts split by tab/CR/LF are refused by the ordinary
        # checks on the stripped spelling, for the right reason.
        for url in (
            f"http://127{T}.0.0.1/",
            f"http://192.168{N}.0.1/",
            f"http://local{R}host/",
        ):
            with self.assertRaises(ValueError, msg=url):
                validate_crawl_url(url)

    def test_tab_split_slash_run_authority_spellings(self):
        # WHATWG strips tab/LF/CR before parsing, so a control
        # character inside the delimiter run after the scheme colon
        # does not split the run: the #586 slash-run collapse must
        # judge the stripped spelling, not the raw one (#589). The
        # behaviour arrived with the #611 strip (app.py removes
        # tab/CR/LF before the collapse); this test pins the
        # slash-run spellings #589 names so a reordering of those
        # two steps cannot silently reopen the divergence.
        T, N, R = chr(9), chr(10), chr(13)
        for raw, expected in (
            (f"http:{T}/example.com/x", "http://example.com/x"),
            (f"http:{N}//example.com/x", "http://example.com/x"),
            (f"http:{R}//example.com/x", "http://example.com/x"),
            (f"http:/{T}/example.com/x", "http://example.com/x"),
            (f"http:/{N}/example.com/x", "http://example.com/x"),
            (f"http:/{R}{N}/example.com/x", "http://example.com/x"),
            (f"https:{T}/example.com/", "https://example.com/"),
        ):
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Blocked targets in tab-split slash-run spellings are
        # refused for the right reason (non-public / local), not
        # as hostless URLs.
        for url in (
            f"https:/{T}/127.0.0.1/",
            f"http:{T}//127.0.0.1/x",
            f"http:{T}//localhost/x",
        ):
            with self.assertRaises(ValueError, msg=url) as cm:
                validate_crawl_url(url)
            self.assertNotIn("no host", str(cm.exception), url)

    def test_refuses_non_http_schemes_and_local_names(self):
        for url in (
            "ftp://example.com/",
            "file:///etc/passwd",
            "http://localhost:8787/",
            "http://printer.local/",
            "http://nas.internal/",
            "http://localhost./",
            # The bare home.arpa apex is itself the RFC 8375 local
            # name — the dotted suffix alone misses it (#608).
            "http://home.arpa/",
            "http://home.arpa./",
            "http://HOME.ARPA/",
            "http://router.home.arpa/",
        ):
            self.assertRefused(url)

    def test_refuses_empty_label_hosts(self):
        # A second trailing dot defeats the one-dot strip (#466):
        # "localhost.." becomes "localhost." and misses the blocked-host
        # checks. An interior or leading empty label is not a valid DNS
        # name either (#511).
        for url in (
            "http://localhost../",
            "http://127.0.0.1../",
            "http://foo.localhost../",
            "http://example..com/",
            "http://.example.com/",
        ):
            self.assertRefused(url)
        # Controls: single root dot and ordinary names still pass.
        self.assertAccepted("http://example.com./")
        self.assertAccepted("http://example.com/")

    def test_refuses_overlong_dns_names(self):
        # RFC 1035 §2.3.4: no label over 63 octets, no name over 253
        # characters of text. Neither can ever resolve; refuse up
        # front instead of certifying them as public DNS names (#513).
        overlong_name = ".".join(["a" * 63] * 4 + ["com"])  # 259 chars
        for url in (
            "http://" + "b" * 64 + ".com/",
            "http://" + overlong_name + "/",
        ):
            self.assertRefused(url)
        # Controls at the limit stay allowed: a 63-char label, and a
        # 253-char name (63+63+63+61 + 3 dots).
        self.assertAccepted("http://" + "b" * 63 + ".com/")
        at_limit_name = ".".join(["a" * 63] * 3 + ["a" * 61])  # 253 chars
        self.assertAccepted("http://" + at_limit_name + "/")

    def test_refuses_idn_names_overlong_once_encoded(self):
        # The RFC 1035 limits apply to the IDNA-encoded form, which is
        # what the Worker's u.hostname carries: a 42-char CJK label is
        # 65 chars once punycode-encoded, and five 40-char CJK labels
        # are a 204-char name that encodes to 309 — both over the
        # limits even though their character counts fit (#517).
        def cjk_label(n):
            return "".join(chr(0x4E00 + i) for i in range(n))

        self.assertRefused("http://" + cjk_label(42) + ".com/")
        overlong_idn_name = ".".join([cjk_label(40)] * 5)
        self.assertRefused("http://" + overlong_idn_name + "/")
        # Control at the encoded limit stays allowed: a 40-char CJK
        # label encodes to 61 chars, and an ordinary IDN host is fine.
        self.assertAccepted("http://" + cjk_label(40) + ".com/")
        self.assertAccepted("http://examéple.com/")

    def test_refuses_dotted_quad_non_public(self):
        for url in (
            "http://127.0.0.1/",
            "http://10.0.0.5/",
            "http://172.16.0.9/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "https://[::1]/",
            "https://[fe80::1]/",
            "https://[fd00::1]/",
        ):
            self.assertRefused(url)

    def test_refuses_inet_aton_spellings_of_non_public_addresses(self):
        # Every spelling below resolves (getaddrinfo / inet_aton semantics)
        # to the non-public address in its comment; ipaddress alone would
        # reject them all and the old code mistook them for DNS names (#468).
        for url in (
            "http://127.1/",  # 127.0.0.1
            "http://2130706433/",  # 127.0.0.1
            "http://0x7f000001/",  # 127.0.0.1
            "http://0x7f.1/",  # 127.0.0.1
            "http://0177.0.0.1/",  # 127.0.0.1 (octal)
            "http://2852039166/",  # 169.254.169.254 (cloud metadata)
            "http://3232235521/",  # 192.168.0.1
            "http://167772161/",  # 10.0.0.1
        ):
            self.assertRefused(url)

    def test_judges_translation_prefixes_by_embedded_ipv4(self):
        # NAT64 (64:ff9b::/96) and 6to4 (2002::/16) literals embed an IPv4
        # address; ipaddress marks both prefixes global as a whole, so the
        # embedded address must be judged explicitly (#472). Verdicts must
        # match the TypeScript validator's suite case for case.
        for url in (
            "https://[64:ff9b::7f00:1]/",  # NAT64 embedding 127.0.0.1
            "https://[64:ff9b::a9fe:a9fe]/",  # NAT64 embedding 169.254.169.254
            "https://[64:ff9b::c0a8:101]/",  # NAT64 embedding 192.168.1.1
            "https://[2002:7f00:1::]/",  # 6to4 embedding 127.0.0.1
            "https://[2002:a9fe:a9fe::]/",  # 6to4 embedding 169.254.169.254
            "https://[::ffff:7f00:1]/",  # IPv4-mapped loopback, hex spelling
        ):
            self.assertRefused(url)
        for url in (
            "https://[64:ff9b::808:808]/",  # NAT64 embedding 8.8.8.8
            "https://[2002:808:808::]/",  # 6to4 embedding 8.8.8.8
        ):
            self.assertAccepted(url)
        # IPv4-mapped public, dotted spelling: accepted, and returned
        # in the canonical hex form the Worker serializes (#637).
        self.assertEqual(
            validate_crawl_url("https://[::ffff:8.8.8.8]/"),
            "https://[::ffff:808:808]/",
        )

    def test_agrees_with_worker_validator_on_ipv4_ranges(self):
        # Verdicts must match the TypeScript validator's suite case for
        # case (#476): ipaddress.is_global passes multicast and
        # 192.88.99.0/24, and the /24 special ranges' public neighbours
        # stay crawlable.
        for url in (
            "http://224.0.0.1/",  # multicast 224.0.0.0/4
            "http://230.1.2.3/",  # multicast
            "http://239.255.255.255/",  # multicast
            "https://[::ffff:224.0.0.1]/",  # IPv4-mapped multicast
            "http://192.0.0.1/",  # 192.0.0.0/24
            "http://192.0.2.1/",  # TEST-NET-1
            "http://192.88.99.1/",  # 6to4 relay anycast (deprecated)
            "http://198.51.100.1/",  # TEST-NET-2
            "http://203.0.113.1/",  # TEST-NET-3
        ):
            self.assertRefused(url)
        for url in (
            "http://192.0.0.9/",  # PCP anycast exception in 192.0.0.0/24
            "http://192.0.0.10/",  # DNS anycast exception in 192.0.0.0/24
            "http://192.0.1.1/",  # public neighbour of the 192.0 /24s
            "http://192.88.0.1/",  # public neighbour of 192.88.99.0/24
            "http://198.51.0.1/",  # public neighbour of TEST-NET-2
            "http://203.0.0.1/",  # public neighbour of TEST-NET-3
            "http://8.8.8.8/",  # ordinary public
        ):
            self.assertAccepted(url)

    def test_agrees_with_worker_validator_on_special_ipv6_ranges(self):
        # Verdicts must match the TypeScript validator's suite case for
        # case (#474): ipaddress.is_global alone passes multicast,
        # site-local, SRv6 and reserved IPv6 space, and the 2001::/23
        # IANA special-purpose block is non-global except six entries.
        for url in (
            "https://[ff00::1]/",  # multicast
            "https://[4000::1]/",  # reserved, outside 2000::/3
            "https://[fec0::1]/",  # site-local (deprecated)
            "https://[5f00::1]/",  # SRv6 SIDs
            "https://[100:1::1]/",  # outside the 100::/64 discard prefix
            "https://[2001::1]/",  # Teredo 2001::/32
            "https://[2001:2::1]/",  # benchmarking 2001:2::/48
            "https://[2001:10::1]/",  # ORCHID 2001:10::/28
            "https://[2001:40::1]/",  # 2001::/23 special-purpose
            "https://[2001:50::1]/",  # 2001::/23 special-purpose
            "https://[2001:1::3]/",  # 2001:1::/48 global only at ::1/::2
            "https://[3fff::1]/",  # documentation 3fff::/20 (RFC 9637) (#519)
            "https://[3fff:0fff:ffff:ffff:ffff:ffff:ffff:ffff]/",  # last address in 3fff::/20 (#519)
        ):
            self.assertRefused(url)
        for url in (
            "https://[2001:1::1]/",  # PCP anycast
            "https://[2001:1::2]/",  # PCP anycast
            "https://[2001:3::1]/",  # AMT 2001:3::/32
            "https://[2001:4:112::1]/",  # AS112 2001:4:112::/48
            "https://[2001:20::1]/",  # ORCHIDv2 2001:20::/28
            "https://[2001:30::1]/",  # ORCHIDv2 2001:30::/28
            "https://[2001:4860:4860::8888]/",  # ordinary global unicast
            "https://[3fff:1000::1]/",  # first address past 3fff::/20 (#519)
            "https://[3ffe::1]/",  # global unicast below 3fff::/20 (#519)
        ):
            self.assertAccepted(url)

    def test_refuses_numeric_final_label_hosts_that_are_not_ipv4(self):
        # WHATWG "ends in a number" (#502): a host whose final label is
        # numeric is an IP literal in the URL grammar, never a DNS name.
        # These spellings parse as neither ipaddress nor inet_aton IPv4,
        # so they must be refused, not fall through to "public DNS name"
        # — the Worker's WHATWG validator refuses every one.
        for url in (
            "http://256.1.1.1/",
            "http://1.2.3.256/",
            "http://1.2.3.4.5/",
            "http://4294967296/",  # one past the largest integer IPv4
            "http://0.0.0.0.0/",
            "http://example.123/",
            "http://example.0x7f/",
            "http://foo.123.456/",
        ):
            self.assertRefused(url)

    def test_refuses_bare_0x_final_label(self):
        # WHATWG's IPv4-number parser returns 0 for a bare 0x/0X prefix
        # (the digit sequence after it is empty), so these hosts also
        # end in a number: `0x` alone normalises to 0.0.0.0 and the
        # Worker's validator refuses every spelling, while the #502
        # pattern required at least one hex digit after the prefix
        # (#504). Redirect targets inherit via the same function.
        for url in (
            "http://example.0x/",
            "http://example.0X/",
            "http://example.0x./",  # trailing root dot stripped first
            "http://0x/",
            "http://0X/",
        ):
            self.assertRefused(url)
        # A trailing non-hex character makes the label not an IPv4
        # number in WHATWG either, so these stay public DNS names.
        for url in (
            "http://example.0xg/",
            "http://example.0x7fg/",
        ):
            self.assertAccepted(url)

    def test_refuses_malformed_or_out_of_range_ports(self):
        # urlparse reports these only lazily via .port; the validator
        # must refuse them up front like the Worker's URL parser (#502).
        for url in (
            "http://8.8.8.8:abc/",
            "http://8.8.8.8:99999/",
            "http://8.8.8.8:-1/",
            "http://example.com:99999/",
            "http://example.com:80abc/",
        ):
            self.assertRefused(url)

    def test_refuses_percent_encoded_hosts_and_zone_ids(self):
        # WHATWG URL parsing percent-decodes the host before judging it
        # (%6cocalhost is localhost) and rejects the other % spellings
        # outright; urlparse passes them through verbatim and the old
        # code mistook them for public DNS names, while ipaddress
        # accepted the IPv6 zone id (#505). Every case here still
        # refuses under decode-and-judge (#529): the first four decode
        # to blocked or forbidden-character hosts, the rest are invalid
        # escapes or a zone id.
        for url in (
            "http://%6cocalhost/",
            "http://localhost%00/",
            "http://127.0.0.1%09/",
            "http://8.8.8.8%09/",
            "http://8.8.8.8%zz/",
            "http://example.com%09/",
            "http://example.com%/",  # malformed: bare %
            "http://example.com%2/",  # malformed: truncated escape
            "https://[2001:4860:4860::8888%25eth0]/",  # IPv6 zone id
        ):
            self.assertRefused(url)

    def test_accepts_percent_encoded_spellings_of_public_hosts(self):
        # #505's blanket refusal also refused spellings that decode to
        # public hosts, which the Worker validator accepts — the two
        # validators diverged, including on redirect targets, which
        # are validated raw (#529). The decoded form is judged, and
        # the validated URL carries the decoded host so the fetcher
        # connects to the host that was judged (urllib does not
        # percent-decode hosts).
        for url, expected in (
            ("http://%65xample.com/", "http://example.com/"),
            ("http://example%2ecom/", "http://example.com/"),
            ("http://8%2e8%2e8%2e8/", "http://8.8.8.8/"),
            ("http://%38%2e8.8.8/", "http://8.8.8.8/"),
            # Port, path and userinfo survive the host substitution.
            ("http://%65xample.com:8080/page", "http://example.com:8080/page"),
            (
                "http://user@%65xample.com/page",
                "http://user@example.com/page",
            ),
        ):
            self.assertEqual(validate_crawl_url(url), expected, url)

    def test_refuses_percent_decoded_blocked_and_invalid_hosts(self):
        # Decode-and-judge (#529): spellings that decode to blocked
        # targets refuse via the ordinary checks, and spellings WHATWG
        # rejects outright (decoded delimiters, a double-encoded %,
        # undecodable bytes) refuse at the decode step.
        for url in (
            "http://127%2e0%2e0%2e1/",  # -> 127.0.0.1
            "http://foo%2elocalhost/",  # -> foo.localhost
            "http://exam%2fple.com/",  # -> exam/ple.com (delimiter)
            "http://exam%3aple.com/",  # -> exam:ple.com (delimiter)
            "http://example%252ecom/",  # -> example%2ecom (still has %)
            "http://%ff.com/",  # decoded bytes are not valid UTF-8
            "http://%c3%28.com/",  # truncated UTF-8 sequence
        ):
            self.assertRefused(url)

    def test_refuses_percent_encoded_nfkc_mapped_delimiters(self):
        # The decode step (#529) refuses delimiters in the decoded
        # form, but a percent-encoded compatibility character decodes
        # to a codepoint whose NFKC mapping IS a delimiter, and the
        # mapped form is what gets spliced into the returned URL:
        # "localhost%ef%bc%8fexample.com" came back ALLOW as
        # "http://localhost/example.com/" — the fetcher connected to
        # localhost, a host that was never judged (#567). The Worker
        # refuses every one of these outright.
        for url in (
            "http://localhost%ef%bc%8fexample.com/",  # U+FF0F -> /
            "http://127.0.0.1%ef%bc%8fexample.com/",  # U+FF0F -> /
            "http://192.168.0.1%ef%bc%8fadmin/",  # U+FF0F -> /
            "http://exam%ef%bc%8fple.com/",  # U+FF0F -> /
            "http://exam%ef%bc%9aple.com/",  # U+FF1A -> :
            "http://exam%ef%bc%a0ple.com/",  # U+FF20 -> @
            "http://exam%ef%bc%83ple.com/",  # U+FF03 -> #
            "http://exam%ef%bc%9fple.com/",  # U+FF1F -> ?
            "http://exam%ef%bc%bcple.com/",  # U+FF3C -> backslash
            "http://exam%ef%bc%bbple.com/",  # U+FF3B -> [
            "http://exam%ef%bc%bdple.com/",  # U+FF3D -> ]
            "http://example%ef%bc%85com/",  # U+FF05 -> %
            "http://%e2%84%80.com/",  # U+2100 expands to a/c
            "http://%e2%84%85.com/",  # U+2105 expands to c/o
        ):
            self.assertRefused(url)

    def test_accepted_percent_urls_reparse_to_the_judged_host(self):
        # For every accepted percent-encoded URL, re-parsing the
        # returned URL must yield exactly the host that was judged —
        # no delimiter may be spliced in by decoding or mapping
        # (#567 acceptance criterion).
        from urllib.parse import urlparse

        for url, expected_host in (
            ("http://%65xample.com/", "example.com"),
            ("http://example%2ecom/", "example.com"),
            ("http://8%2e8%2e8%2e8/", "8.8.8.8"),
            ("http://%65xample.com:8080/page", "example.com"),
            # U+FF5E maps to ~, not a delimiter: the mapped form is
            # judged and returned, and re-parses to itself.
            ("http://exam%ef%bd%9eple.com/", "exam~ple.com"),
        ):
            returned = validate_crawl_url(url)
            self.assertEqual(urlparse(returned).hostname, expected_host, url)

    def test_redirect_handler_refuses_nfkc_mapped_delimiter_target(self):
        # Redirect targets are validated raw (#529/#567): a Location
        # whose percent-encoded host NFKC-maps into a private host
        # plus delimiter must be refused, not followed.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        with self.assertRaises(ValueError):
            handler.redirect_request(
                req, None, 302, "Found", {}, "http://192.168.0.1%ef%bc%8fadmin/"
            )

    def test_redirect_handler_follows_the_decoded_target(self):
        # Redirect targets are validated raw, with no Worker
        # normalisation in between (#529): the handler must fetch the
        # decoded form of a percent-encoded public target, and refuse
        # one that decodes to a blocked target.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://%65xample.org/final"
        )
        self.assertEqual(followed.full_url, "http://example.org/final")
        with self.assertRaises(ValueError):
            handler.redirect_request(req, None, 302, "Found", {}, "http://%6cocalhost/")

    def test_refuses_space_hosts_and_ipvfuture_literals(self):
        # A space in the host is not a DNS name: WHATWG rejects it and
        # http.client only raises for it deep inside the fetcher, so the
        # validator refuses up front. A bracketed host that is not a
        # valid IPv6 literal (IPvFuture forms) occupied the IP-literal
        # position; a DNS name never contains a colon, so it must not be
        # reclassified as a public DNS name either (#507).
        for url in (
            "http://exam ple.com/",
            "http://exa mple.com/",
            "http://[v1.fe80::]/",
            "http://[v7.a]/",
            "http://[vf.FF]/",
        ):
            self.assertRefused(url)

    def test_refuses_del_c1_nbsp_and_forbidden_host_characters(self):
        # The #507 check stopped at 0x20, one codepoint short of
        # http.client's own set and short of the rest of the WHATWG
        # forbidden-host set: DEL, the C1 controls, NBSP and < > ^ |
        # still fell through as "a public DNS name", while the
        # Worker's WHATWG validator refuses every one (#510).
        for url in (
            "http://exam\x7fple.com/",  # DEL
            "http://exam\x80ple.com/",  # C1 control, low end
            "http://exam\x85ple.com/",  # C1 control (NEL)
            "http://exam\x9fple.com/",  # C1 control, high end
            "http://exam\xa0ple.com/",  # NBSP
            "http://exam<ple.com/",
            "http://exam>ple.com/",
            "http://exam^ple.com/",
            "http://exam|ple.com/",
        ):
            self.assertRefused(url)
        # Controls: characters above NBSP and ordinary DNS punctuation
        # are not forbidden — valid IDN hosts stay crawlable.
        self.assertAccepted("http://exam_ple.com/")
        self.assertAccepted("http://examéple.com/")

    def test_refuses_mapped_spellings_of_blocked_targets(self):
        # #527: the resolver NFKC-maps these spellings onto blocked
        # targets (verified with getaddrinfo), so judge the mapped form.
        for url in (
            "http://ｌｏｃａｌｈｏｓｔ/",  # fullwidth localhost -> ::1
            "http://ｌocalhost/",  # first char fullwidth -> ::1
            "http://127。0。0。1/",  # U+3002 label separators -> 127.0.0.1
            "http://127｡0｡0｡1/",  # U+FF61 separators -> 127.0.0.1
            "http://127．0．0．1/",  # U+FF0E separators -> 127.0.0.1
            "http://１２７.０.０.１/",  # fullwidth digits -> 127.0.0.1
            "http://10。0。0。5/",  # -> 10.0.0.5 (private)
            "http://192。168。1。1/",  # -> 192.168.1.1 (private)
            "http://example。ｌｏｃａｌｈｏｓｔ/",  # -> example.localhost
            "http://foo。localhost/",  # -> foo.localhost
            "http://foo．localhost/",  # -> foo.localhost
            "http://localhost。/",  # -> localhost.
        ):
            self.assertRefused(url)
        # Controls: mapped spellings of public names stay crawlable,
        # and return in the mapped form the Worker returns (#665).
        self.assertEqual(
            validate_crawl_url("http://ｅｘａｍｐｌｅ.com/"), "http://example.com/"
        )
        # The U-label spelling is kept, but its literal separator
        # now returns as "." (#660), not the fullwidth character.
        self.assertEqual(
            validate_crawl_url("http://münchen。de/"), "http://münchen.de/"
        )

    def test_refuses_idna_disallowed_unicode_hosts(self):
        # UTS-46/IDNA disallows these codepoints in a host label, and
        # the Worker's WHATWG parser rejects the URL outright, but they
        # survive NFKC unchanged so no earlier check caught them and
        # the old code certified the host as "a public DNS name" (#516).
        disallowed = (
            "\u1680",  # Ogham space mark
            "\u2028",  # line separator
            "\u2029",  # paragraph separator
            "\u0600",  # Arabic number sign (format mark)
            "\u061c",  # Arabic letter mark
            "\u200e",  # left-to-right mark
            "\u200f",  # right-to-left mark
            "\u202a",  # left-to-right embedding
            "\u202b",  # right-to-left embedding
            "\u202c",  # pop directional formatting
            "\u202d",  # left-to-right override
            "\u202e",  # right-to-left override
            "\u2066",  # left-to-right isolate
            "\u2067",  # right-to-left isolate
            "\u2068",  # first strong isolate
            "\u2069",  # pop directional isolate
            "\u200c",  # zero-width non-joiner (no joining context here)
            "\u200d",  # zero-width joiner (no joining context here)
            "\ufffd",  # replacement character
        )
        for ch in disallowed:
            self.assertRefused("http://exam" + ch + "ple.com/")
        # The Unicode space separators are disallowed too; NFKC maps
        # them to an ordinary space, refused by the space arm.
        for ch in ("\u2000", "\u200a", "\u202f", "\u205f", "\u3000"):
            self.assertRefused("http://exam" + ch + "ple.com/")
        # Controls: the UTS-46 "ignored" codepoints are deleted by
        # IDNA processing, not disallowed — the Worker accepts these
        # hosts (as example.com) and so must the container, which
        # returns the host with the codepoint deleted (#652).
        for ch in ("\u00ad", "\u180e", "\u200b", "\u2060", "\u2064", "\ufeff"):
            self.assertEqual(
                validate_crawl_url("http://exam" + ch + "ple.com/"),
                "http://example.com/",
            )

    def test_refuses_arabic_indic_digit_hosts(self):
        # WHATWG's domain-to-ASCII rejects an Arabic-Indic digit
        # (U+0660-U+0669) outside a right-to-left label — even one
        # digit in a mixed LTR label — while urlparse passes the
        # spelling through and the ASCII-only numeric checks cannot
        # read it as an address, so the old code certified the hosts
        # below as public DNS names and fetched their punycode forms
        # (#574). Every spelling below is outside an RTL label, so
        # _judge_idna_label's AN rule refuses them all; the RTL
        # spellings the Worker accepts are pinned in the #605 test
        # below.
        self.assertRefused("http://١٢٧.٠.٠.١/")  # 127.0.0.1, Arabic-Indic digits
        self.assertRefused("http://١٢٣.com/")
        self.assertRefused("http://example١٢٣.com/")
        self.assertRefused("http://a١.com/")
        self.assertRefused("http://١a.com/")
        self.assertRefused("http://1٢٣.com/")
        self.assertRefused("http://münchen١.com/")
        # The percent-encoded spelling decodes to the same host and
        # is judged on the decoded form, so it is refused too.
        self.assertRefused("http://%d9%a1%d9%a2%d9%a3.com/")  # ١٢٣.com
        # Controls: the other decimal-digit families are ordinary IDN
        # characters to WHATWG (accepted as punycode DNS names), and
        # the container accepts them too.
        self.assertAccepted("http://१२७.०.०.१/")  # Devanagari digits
        self.assertAccepted("http://۱۲۷.۰.۰.۱/")  # Extended Arabic-Indic (U+06F0s)
        self.assertAccepted("http://১২৭.০.০.১/")  # Bengali digits
        self.assertAccepted("http://๑๒๗.๐.๐.๑/")  # Thai digits
        self.assertAccepted("http://1۲۳.com/")  # ASCII + Extended Arabic-Indic

    def test_refuses_a_label_spellings_of_arabic_indic_digit_hosts(self):
        # PR #575 refused Arabic-Indic digit hosts in raw form (#574),
        # but the A-label (punycode) spelling of the same hosts is
        # pure ASCII, carries no U+0660-U+0669 codepoint, and was
        # never decoded before judging — so it sailed through as an
        # ordinary public DNS name while the Worker's WHATWG parser
        # decodes the A-label and rejects it outright (#580). Judge
        # the decoded label with the same policy.
        for url in (
            "http://xn--9hbcp.xn--8hb.xn--8hb.xn--9hb/",  # ١٢٧.٠.٠.١ = 127.0.0.1
            "http://xn--9hbcp.xn--8hb.xn--8hb.xn--9hb./",  # trailing root dot
            "http://xn--a-bqc.com/",  # a١.com (single digit, mixed label)
            "http://xn--example-h3jjk.com/",  # example١٢٣.com
            "http://xn--mnchen-3ya761i.com/",  # münchen١.com
        ):
            self.assertRefused(url)
        # An A-label that does not decode to the non-ASCII label it
        # claims to encode is not a valid IDNA label at all, and the
        # Worker refuses those spellings too.
        for url in (
            "http://xn--999.com/",  # incomplete punycode
            "http://xn--a.com/",  # decodes to U+0080, a C1 control
            "http://xn--localhost-.com/",  # decodes to ASCII "localhost"
            "http://xn--.com/",  # decodes to the empty label
        ):
            self.assertRefused(url)
        # Controls: A-label spellings of accepted hosts stay accepted
        # — the Devanagari-digit A-label (the raw spelling is an
        # accepted control in the #574 test above), ordinary IDN
        # A-labels, and a single-character CJK A-label.
        for url in (
            "http://xn--e4bcp.xn--d4b.xn--d4b.xn--e4b/",  # १२७.०.०.१ Devanagari
            "http://xn--mnchen-3ya.de/",  # münchen.de
            "http://xn--bcher-kva.de/",  # bücher.de
            "http://xn--fsq.com/",  # 例.com
        ):
            self.assertAccepted(url)

    def test_accepts_arabic_indic_digits_inside_rtl_labels(self):
        # The Arabic-Indic digits are bidi class AN: inside a genuine
        # RTL label they are valid, and the Worker's WHATWG parser
        # accepts them — but #574's blanket forbidden-character range
        # refused every host containing one, in raw and A-label form
        # alike, overriding _judge_idna_label's contextual AN rule
        # (#595) in the one context where the digit is valid (#605).
        # The Extended Arabic-Indic control (U+06F1) was already
        # accepted in the same shape.
        self.assertAccepted("http://\u0627\u0661.com/")  # Arabic + ١
        self.assertAccepted("http://\u0645\u0631\u062d\u0628\u0627\u0661.com/")
        self.assertAccepted("http://\u05d0\u0661.com/")  # Hebrew + ١
        self.assertAccepted("http://\u0627\u0661\u0627.com/")  # AN mid-label
        self.assertAccepted("http://\u0627\u0660.com/")
        # Their A-label spellings are accepted too.
        self.assertAccepted("http://xn--mgb0j.com/")  # ا١.com
        self.assertAccepted("http://xn--mgbbop3hzh.com/")  # مرحبا١.com
        self.assertAccepted("http://xn--4db40a.com/")  # א١.com
        # Guards: the AN context rules still refuse an AN+EN mix
        # inside an RTL label and an RTL label that does not start
        # with an R/AL character (both also refused by the Worker).
        self.assertRefused("http://\u05d0\u06611.com/")
        self.assertRefused("http://\u0661\u0627.com/")

    def test_judges_u0345_by_its_uts46_mapping(self):
        # UTS-46 maps U+0345 (combining Greek ypogegrammeni) to
        # U+03B9 (ι, bidi class L), but NFKC leaves it unchanged, so
        # the container judged it as a combining mark (bidi NSM) —
        # legal mid-label anywhere — while the Worker judges the
        # mapped letter: inside an RTL label the mapped form is
        # R + L + R, a bidi violation (#596).
        self.assertRefused("http://\u05d0\u0345\u05d0.com/")
        self.assertRefused("http://\u05d0\u0345.com/")
        # The A-label spelling of the first host is refused too.
        self.assertRefused("http://xn--jua51kba.com/")
        # Controls: in LTR/Greek contexts the mapped letter is an
        # ordinary character and the hosts stay accepted.
        self.assertAccepted("http://ex\u0345ample.com/")
        self.assertAccepted("http://\u03b1\u0345.com/")
        # Pin: at label start the mapping closes the former
        # leading-mark over-refusal — U+0345 maps to a letter, so
        # the label starts with one, matching the Worker (#596).
        self.assertAccepted("http://\u0345abc.com/")

    def test_refuses_idna_invalid_hosts(self):
        # A label can punycode-encode without being a valid IDNA
        # label at all: the mapped host was never IDNA-validated, so
        # whole classes the Worker's WHATWG domain-to-ASCII refuses
        # outright were certified here as public DNS names (#583).
        # Unassigned codepoints (category Cn in this interpreter's
        # Unicode tables):
        self.assertRefused("http://\u0378ample.com/")
        self.assertRefused("http://ex\u0380ample.com/")
        self.assertRefused("http://ex\u05ebample.com/")
        # A format character (category Cf) that is neither in the
        # UTS-46 ignored set nor the #516 denylist:
        self.assertRefused("http://a\u0601b.com/")
        # Bidi-rule violations: an LTR label mixing in RTL characters
        # (the stdlib codec catches the Hebrew case; the post-3.2
        # Arabic letters and the NKo digit need the explicit
        # directionality check), and an RTL label that does not start
        # with an RTL character.
        self.assertRefused("http://ex\u05d0ample.com/")
        self.assertRefused("http://local\u05d0host/")
        self.assertRefused("http://ex\u0620ample.com/")
        self.assertRefused("http://ex\u06eeample.com/")
        self.assertRefused("http://a\u07c0.com/")
        self.assertRefused("http://1\u05d0.com/")
        self.assertRefused("http://-\u05d0.com/")
        # A label beginning with a combining mark:
        self.assertRefused("http://\u0301abc.com/")
        self.assertRefused("http://\u0301abc\u00e9.com/")
        # The Arabic decimal separator is IDNA-disallowed in a label
        # with strong LTR characters:
        self.assertRefused("http://ex\u066bample.com/")
        self.assertRefused("http://ex\u066cample.com/")
        # A character of bidi class AN (Arabic number) is only valid
        # inside an RTL label: the astral AN codepoints — Hanifi
        # Rohingya and Rumi digits — passed every other arm of this
        # check in Latin, neutral and digit-led labels (#595), and
        # AN may not mix with EN (European number) even in an RTL
        # label. The U+066B case below is the BMP instance of the
        # AN+EN rule (U+066B is itself class AN).
        self.assertRefused("http://ex\U00010d30ample.com/")
        self.assertRefused("http://ex\U00010e60ample.com/")
        self.assertRefused("http://\U00010d30abc.com/")
        self.assertRefused("http://abc\U00010d30.com/")
        self.assertRefused("http://m\u00fcnchen\U00010d30.de/")
        self.assertRefused("http://\U00010d30.com/")
        self.assertRefused("http://1\U00010d30.com/")
        self.assertRefused("http://-\U00010d30.com/")
        self.assertRefused("http://\u05d0\U00010d301.com/")
        self.assertRefused("http://\u05d01\U00010d30.com/")
        self.assertRefused("http://\u0627\u066b1.com/")
        # An AN character alone is not an RTL label either:
        self.assertRefused("http://\u066b.com/")
        # Characters whose compatibility mapping introduces a label
        # separator are disallowed (only U+3002/U+FF0E/U+FF61 are
        # UTS-46 separators); after NFKC they would be judged as two
        # innocent labels:
        self.assertRefused("http://ex\u2024ample.com/")
        self.assertRefused("http://ex\u2488ample.com/")
        # U+FE12's compatibility mapping is U+3002 — a separator
        # itself, not an ASCII dot — so the dot-only test missed it
        # and it was silently judged as two labels (#593); the
        # percent-encoded spelling even came back rewritten to the
        # two-label host.
        self.assertRefused("http://ex\ufe12ample.com/")
        self.assertRefused("http://a\ufe12b.com/")
        self.assertRefused("http://example\ufe12com/")
        self.assertRefused("http://ex%ef%b8%92ample.com/")
        # The same validity judgement applies to an A-label's decoded
        # form: this spelling decodes to a label beginning with a
        # combining mark, which PR #584's checks let through.
        self.assertRefused("http://xn--abc-dma09x.com/")
        # The A-label spelling of "ex\ufe12ample": the decoded label
        # carries U+FE12, whose mapping is the U+3002 separator (#593).
        self.assertRefused("http://xn--example-t673b.com/")
        # The A-label spelling of "ex\U00010d30ample": the decoded
        # label carries an astral AN digit in a Latin label (#595).
        self.assertRefused("http://xn--example-eh36b.com/")
        # Controls: pure RTL labels (with or without a trailing
        # European digit), the Arabic separator inside a pure Arabic
        # label, ordinary IDN, and a mid-label combining mark all stay
        # accepted, matching the Worker.
        self.assertAccepted("http://\u05d0\u05d0.com/")
        self.assertAccepted("http://\u05d01.com/")
        self.assertAccepted("http://\u0627\u066b\u0627.com/")
        # An AN digit inside a genuine RTL label (Hebrew, Arabic, or
        # its own Hanifi Rohingya script) stays accepted (#595):
        self.assertAccepted("http://\u05d0\U00010d30\u05d0.com/")
        self.assertAccepted("http://\u0627\U00010d30\u0627.com/")
        self.assertAccepted("http://\U00010d00\U00010d30.com/")
        self.assertAccepted("http://\u05d0\U00010d30.com/")
        self.assertAccepted("http://xn--4dba5833r.com/")
        self.assertAccepted("http://m\u00fcnchen.de/")
        self.assertAccepted("http://ex\u0301ample.com/")

    def test_a_label_must_be_its_uts46_mapped_form_and_disallowed_free(self):
        # #588 residual: a decoded A-label that is not its own UTS-46
        # mapped form, or that carries an IDNA-disallowed punctuation /
        # symbol / separator character, is refused — the punycode
        # round-trip alone certifies all of these.
        # Mapped-form class (the issue's headline hosts):
        self.assertRefused("http://xn--example-yoa.com/")  # ex\u00c0ample (uppercase)
        self.assertRefused("http://xn--example-rla.com/")  # ex\u00b2ample (compatibility digit)
        self.assertRefused("http://xn--ab-cja0631x.com/")  # a\uff0fb\u00e9 (fullwidth solidus)
        self.assertRefused("http://xn--mnchen-3ya5364d.com/")  # m\u00fcn\u2000chen (space class)
        # DISALLOWED class (survive NFKC, assigned, not format chars):
        self.assertRefused("http://xn--example-hja.com/")  # ex\u00a8ample (diaeresis, Sk)
        self.assertRefused("http://xn--example-7la.com/")  # ex\u00b4ample (acute accent, Sk)
        self.assertRefused("http://xn--example-f9e.com/")  # ex\u037eample (Greek question mark, Po)
        self.assertRefused("http://xn--example-3ha.com/")  # ex\u00a2ample (cent sign, Sc)
        self.assertRefused("http://xn--example-u17c.com/")  # ex\u20acample (euro sign, Sc)
        # The raw spellings of the DISALLOWED class are refused too
        # (the check sits in the shared label judge):
        self.assertRefused("http://ex\u00a8ample.com/")
        # (No raw U+037E case: NFKC folds it to an ASCII ";" before
        # judging, in both validators — only the unmapped decoded
        # A-label form above is the #588 divergence.)
        # Controls: the PR #584 accepted A-labels and ordinary IDN
        # stay accepted, and the CONTEXTO middle dot label is not
        # swept up by the punctuation arm.
        self.assertAccepted("http://xn--mnchen-3ya.de/")  # m\u00fcnchen.de
        self.assertAccepted("http://xn--bcher-kva.de/")  # b\u00fccher.de
        self.assertAccepted("http://xn--fsq.com/")  # \u4f8b.com
        self.assertAccepted("http://xn--al-0ea.com/")  # a\u00b7l (Catalan middle dot)

    def test_judges_length_and_blocked_names_on_the_mapped_form(self):
        # UTS-46 mapping deletes the "ignored" codepoints before any
        # judgement, so the length check must measure the mapped form
        # and the blocked-host check must see through them (#526):
        # 60 "a"s plus ten soft hyphens is a 60-character label to the
        # Worker, but 77 characters of raw punycode to the old check.
        # The returned form carries the judged (deleted) host (#652).
        self.assertEqual(
            validate_crawl_url("http://" + "a" * 60 + "\u00ad" * 10 + ".com/"),
            "http://" + "a" * 60 + ".com/",
        )
        # NFKC expansion runs the other way: four U+FDFA ligatures
        # expand to a long Arabic phrase whose A-label is overlong.
        self.assertRefused("http://" + "\ufdfa" * 4 + ".com/")
        # A blocked name spelled with an ignored codepoint inside it
        # is still the blocked name in its mapped form.
        self.assertRefused("http://local\u00adhost/")
        self.assertRefused("http://local\u200bhost/")
        self.assertRefused("http://local\ufeffhost/")

    def test_accepts_public_targets(self):
        for url in (
            "https://example.com/page",
            "http://8.8.8.8/",
            "https://[2606:4700:4700::1111]/",
            "http://example.com./",  # FQDN root-dot spelling of a public name
            "http://dead.beef/",  # hex-looking labels, non-numeric final label
            "http://a1.beef/",
            "http://123.example.com/",  # numeric label, but not the final one
            "http://example.com:8080/",  # an ordinary explicit port
        ):
            self.assertAccepted(url)

    def test_returns_ip_literals_in_canonical_form(self):
        # An IP literal names the same address in many spellings; the
        # returned URL carries the canonical form of the address that
        # was judged — dotted quad for inet_aton IPv4 spellings,
        # compressed lowercase for IPv6 — matching the Worker's
        # serializer (#637). (assertAccepted pins verbatim returns, so
        # these spellings cannot live in test_accepts_public_targets.)
        cases = {
            "http://134744072/": "http://8.8.8.8/",  # integer spelling
            "http://0x8080808/": "http://8.8.8.8/",  # hex integer
            "http://0x8.0x8.0x8.0x8/": "http://8.8.8.8/",  # hex parts
            "http://010.0.0.010/": "http://8.0.0.8/",  # octal parts
            "http://1.2.3/": "http://1.2.0.3/",  # short form
            "http://1.2.256/": "http://1.2.1.0/",  # last part overflows a byte
            "http://8.8.2056/": "http://8.8.8.8/",
            "http://8.16777215/": "http://8.255.255.255/",
            "http://1.2.3.04/": "http://1.2.3.4/",  # leading-zero part
            "http://[2001:4860:4860:0000:0000:0000:0000:8888]/": (
                "http://[2001:4860:4860::8888]/"
            ),
            # Userinfo and port survive the host substitution.
            "http://user:pass@134744072:8080/x": "http://user:pass@8.8.8.8:8080/x",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)

    def test_returns_dot_segments_host_case_and_ports_normalized(self):
        # The container returned dot segments, host case and default
        # ports verbatim while the Worker returns the WHATWG-normalized
        # form, so the two crawlers recorded — and fetched — different
        # URLs for the same target (#633). The returned URL now
        # resolves dot segments (literal and %2e spellings, any case),
        # lowercases the host, and canonicalizes the port (the scheme's
        # default dropped, leading zeros stripped). Verdicts are
        # unchanged; query and fragment dot spellings are data.
        cases = {
            "http://example.com/a/../b": "http://example.com/b",
            "http://example.com/a/./b": "http://example.com/a/b",
            "http://example.com/%2e%2e/admin": "http://example.com/admin",
            "http://example.com/a/%2e%2e/b": "http://example.com/b",
            "http://example.com/%2e/%2e/x": "http://example.com/x",
            "http://example.com/a/%2E/b": "http://example.com/a/b",
            "http://example.com/a/b/../../../c": "http://example.com/c",
            "http://example.com/a/.": "http://example.com/a/",
            "http://example.com/a/..": "http://example.com/",
            "http://example.com/a/%2e%2e": "http://example.com/",
            "http://example.com//../x": "http://example.com/x",
            "http://example.com//./x": "http://example.com//x",
            "http://example.com/a//../b": "http://example.com/a/b",
            "http://example.com//%2e": "http://example.com//",
            "HTTP://EXAMPLE.COM/Path": "http://example.com/Path",
            "http://example.com:80/x": "http://example.com/x",
            "https://example.com:443/x": "https://example.com/x",
            "http://example.com:080/x": "http://example.com/x",
            "https://example.com:0443/x": "https://example.com/x",
            "http://example.com:08080/x": "http://example.com:8080/x",
            "http://user:pass@EXAMPLE.com:80/": "http://user:pass@example.com/",
            "http://8.8.8.8:80/a/../b": "http://8.8.8.8/b",
            "https://[2001:4860:4860::8888]:443/a/./b": (
                "https://[2001:4860:4860::8888]/a/b"
            ),
            "http://example.com/a/../b?x=/../y#/../z": (
                "http://example.com/b?x=/../y#/../z"
            ),
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Data, not dot segments: longer or double-encoded spellings,
        # ";" tails and empty segments pass through byte-identical.
        for url in (
            "http://example.com/foo/%2e%2e%2e/bar",
            "http://example.com/%252e%252e/admin",
            "http://example.com/a/..;foo",
            "http://example.com/..;/x",
            "http://example.com//a//b",
            "http://example.com:443/x",  # non-default for http
            "https://example.com:80/x",  # non-default for https
        ):
            self.assertAccepted(url)
        # Blocked targets in these spellings are refused by the
        # ordinary checks, for the right reason.
        self.assertRefused("http://127.0.0.1/a/../b")
        self.assertRefused("http://LOCALHOST/a/../b")
        self.assertRefused("http://127.0.0.1:80/x")

    def test_returns_bare_semicolon_dot_spelled_segments_verbatim(self):
        # A trailing segment that is a dot spelling plus a bare ";"
        # ("..;", ".;", "%2e;") is an ordinary segment to WHATWG —
        # the ";" is data — and the Worker returns it unchanged.
        # urlparse splits the last segment's ";params" tail off the
        # path and a bare ";" leaves params empty, so the split parts
        # lost the ";" and the segment was resolved as a dot segment:
        # "..;" collapsed the path (popping its parent), ".;" was
        # dropped (#655). The non-empty-params spellings ("..;foo")
        # were already pinned above; these are their empty-params
        # counterparts, including before "?" / "#" and on the
        # userinfo and percent-decoded-host return paths.
        for url in (
            "http://example.com/..;",
            "http://example.com/.;",
            "http://example.com/foo/..;",
            "http://example.com/a/.;",
            "http://example.com/a/b/..;",
            "http://example.com/%2e;",
            "http://example.com/%2e%2e;",
            "http://example.com/a/%2E;",
            "http://example.com/..;?q",
            "http://example.com/..;#f",
            "http://example.com/..;?",
            "http://user@example.com/..;",
            "http://8.8.8.8/..;",
        ):
            self.assertAccepted(url)
        # The percent-decoded-host return path rebuilds the URL with
        # the decoded host (#529): the bare-";" segment survives that
        # rebuild too.
        self.assertEqual(
            validate_crawl_url("http://%65xample.com/..;"),
            "http://example.com/..;",
        )
        # The fetcher's userinfo strip rebuilds the URL too: the
        # stripped form keeps the bare-";" segment.
        stripped, _ = app._split_userinfo("http://user@example.com/foo/..;")
        self.assertEqual(stripped, "http://example.com/foo/..;")

    def test_redirect_handler_follows_the_normalized_target(self):
        # Redirect targets inherit the normalization (#633): the
        # handler validates the raw Location and fetches the form
        # validate_crawl_url returns.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://EXAMPLE.org:80/a/../b"
        )
        self.assertEqual(followed.full_url, "http://example.org/b")

    def test_returns_empty_components_normalized(self):
        # The container returned an empty path, an empty port and an
        # empty userinfo verbatim while the Worker returns the
        # WHATWG-normalized form, so the two crawlers recorded
        # different URLs for the same target (#639). The returned URL
        # now supplies "/" for an empty path and drops the empty
        # userinfo's "@" (the empty port's colon is dropped by the
        # #633 port canonicalization). Decision on the empty "?" / "#"
        # markers urlparse cannot represent: preserve them, matching
        # the Worker byte for byte — the query/fragment is present
        # but empty. Verdicts are unchanged.
        cases = {
            "http://example.com": "http://example.com/",
            "https://example.com": "https://example.com/",
            "http://example.com:8080": "http://example.com:8080/",
            "http://example.com:80": "http://example.com/",
            "http://example.com:/path": "http://example.com/path",
            "http://@example.com/": "http://example.com/",
            "http://@example.com": "http://example.com/",
            "http://@example.com/path?x#": "http://example.com/path?x#",
            "http://example.com/path?": "http://example.com/path?",
            "http://example.com/path#": "http://example.com/path#",
            "http://example.com?": "http://example.com/?",
            "http://example.com/path?#": "http://example.com/path?#",
            "http://example.com#frag?notquery": ("http://example.com/#frag?notquery"),
            "http://8.8.8.8": "http://8.8.8.8/",
            "http://134744072": "http://8.8.8.8/",
            "http://[2001:4860:4860::8888]": "http://[2001:4860:4860::8888]/",
            "http://user@example.com": "http://user@example.com/",
            "http://example.com/a/../b": "http://example.com/b",
            # The markers survive the host-substitution rebuilds too:
            # percent-decoded hosts and canonical IP literals are
            # rebuilt with geturl() before normalization (#639).
            "http://%65xample.com/path?": "http://example.com/path?",
            "http://%65xample.com?": "http://example.com/?",
            "http://8%2e8%2e8%2e8/path#": "http://8.8.8.8/path#",
            "http://8%2e8%2e8%2e8?#": "http://8.8.8.8/?#",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Non-empty userinfo, query and fragment pass through
        # byte-identical; a "?" inside the query or fragment is data.
        self.assertAccepted("http://user:pass@example.com/path?q=1#frag")
        self.assertAccepted("http://example.com/pa?th#fr?ag")
        # Blocked targets in these spellings are refused by the
        # ordinary checks, for the right reason.
        self.assertRefused("http://@127.0.0.1/")
        self.assertRefused("http://127.0.0.1")
        self.assertRefused("http://@localhost/")
        # Redirect targets inherit the normalization (#639): the
        # handler validates the raw Location and fetches the form
        # validate_crawl_url returns.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://EXAMPLE.org"
        )
        self.assertEqual(followed.full_url, "http://example.org/")

    def test_returns_empty_query_marker_before_a_nonempty_fragment(self):
        # #639's marker re-attachment appended the markers as a suffix
        # after geturl(), so an empty query in front of a non-empty
        # fragment came back after it: "path?#frag" returned as
        # "path#frag?" — a different URL (fragment corrupted, empty
        # query absent) than the Worker returns (#654). The "?"
        # belongs before the "#", at every rebuild site.
        cases = {
            "http://example.com/path?#frag": "http://example.com/path?#frag",
            "http://example.com?#frag": "http://example.com/?#frag",
            "http://example.com/a?#b?c": "http://example.com/a?#b?c",
            "http://example.com/path?#frag?x": "http://example.com/path?#frag?x",
            # Through the percent-decoded-host and canonical-IP
            # rebuilds (_url_with_host), and with userinfo / a
            # non-empty fragment that itself needs encoding.
            "http://%65xample.com/path?#frag": "http://example.com/path?#frag",
            "http://8%2e8%2e8%2e8/path?#frag": "http://8.8.8.8/path?#frag",
            "http://user@example.com/path?#frag": (
                "http://user@example.com/path?#frag"
            ),
            "http://example.com/path?#café": "http://example.com/path?#caf%C3%A9",
            # Controls: the #639 cases stay correct — both markers
            # empty, empty fragment only, non-empty query, and a "?"
            # inside the fragment staying data.
            "http://example.com/path?#": "http://example.com/path?#",
            "http://example.com/path#": "http://example.com/path#",
            "http://example.com/path?x#": "http://example.com/path?x#",
            "http://example.com/path#frag?notquery": (
                "http://example.com/path#frag?notquery"
            ),
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # The fetcher's userinfo strip rebuilds the URL too: it must
        # keep the marker in front of the fragment (#654).
        stripped, _ = app._split_userinfo("http://user@example.com/path?#frag")
        self.assertEqual(stripped, "http://example.com/path?#frag")
        # Verdicts are unchanged: blocked targets stay refused.
        self.assertRefused("http://127.0.0.1/path?#frag")
        self.assertRefused("http://localhost/path?#frag")
        # Redirect targets inherit the fix: the handler fetches the
        # validated form of a raw Location carrying "?#frag".
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://example.org/path?#frag"
        )
        self.assertEqual(followed.full_url, "http://example.org/path?#frag")

    def test_returns_the_root_dot_for_percent_encoded_spellings(self):
        # The container stripped the DNS root dot before judging and
        # rebuilt the returned URL from the stripped host, so the
        # percent-encoded spelling came back without the dot while the
        # literal spelling (returned raw) and the Worker kept it —
        # the same target was recorded under two different URLs
        # depending on the spelling (#647). The judged host is still
        # the stripped one; only the returned form re-attaches the dot.
        cases = {
            # Literal spellings (controls: unchanged).
            "http://example.com./": "http://example.com./",
            "http://example.com./path": "http://example.com./path",
            # Percent-encoded spellings now match them, and the Worker.
            "http://example.com%2e/": "http://example.com./",
            "http://example.com%2E/": "http://example.com./",
            "http://example.com%2e": "http://example.com./",
            "http://example.com%2e/path": "http://example.com./path",
            "http://sub.example.com%2e/": "http://sub.example.com./",
            "http://example.com%2e:8080/path": "http://example.com.:8080/path",
            "http://user@example.com%2e/": "http://user@example.com./",
            "http://example.com%2e?#": "http://example.com./?#",
            # A percent-encoded U+3002 maps to the same root dot.
            "http://example.com%E3%80%82/": "http://example.com./",
            # IP literals serialize without a root dot in WHATWG, and
            # both validators already agreed there — unchanged.
            "http://8.8.8.8./": "http://8.8.8.8/",
            "http://8.8.8.8%2e/": "http://8.8.8.8/",
            "http://8%2e8%2e8%2e8%2e/": "http://8.8.8.8/",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Verdicts are unchanged: the dotted spellings of blocked
        # targets stay refused, and a second dot is still an empty
        # label, in either spelling.
        self.assertRefused("http://localhost%2e/")
        self.assertRefused("http://localhost./")
        self.assertRefused("http://example.com../")
        self.assertRefused("http://example.com%2e%2e/")

    def test_returns_the_ascii_dot_for_literal_separator_spellings(self):
        # PR #657 fixed the percent-encoded root-dot spellings, but
        # the literal UTS-46 separator spellings (U+3002, U+FF0E,
        # U+FF61) still came back with the fullwidth character in
        # them: judging translated them to "." but the non-percent
        # return path returned the raw spelling verbatim, so the
        # literal and percent spellings of the same host character
        # recorded two different URLs and the literal form disagreed
        # with the Worker (#660). The return now rebuilds from the
        # mapped host whenever the raw host carried a literal
        # separator, for interior separators as well as the root dot.
        cases = {
            # Root dot, one per separator character.
            "http://example.com\u3002/": "http://example.com./",
            "http://example.com\uff0e/": "http://example.com./",
            "http://example.com\uff61/": "http://example.com./",
            "http://example.com\u3002": "http://example.com./",
            "http://example.com\u3002/path": "http://example.com./path",
            "http://sub.example.com\uff61/": "http://sub.example.com./",
            "http://example.com\u3002:8080/path": "http://example.com.:8080/path",
            "http://example.com\u3002:80/": "http://example.com./",
            "http://user@example.com\uff61/": "http://user@example.com./",
            "http://example.com\u3002?#": "http://example.com./?#",
            "http://example.com\u3002/path?#frag": ("http://example.com./path?#frag"),
            # Interior separators map to "." too.
            "http://example\u3002com/": "http://example.com/",
            "http://example\uff0ecom/path": "http://example.com/path",
            "http://sub\u3002example\u3002com/": "http://sub.example.com/",
            # An IDN host keeps its U-label spelling; only the
            # separator changes (#659 covers the A-label form).
            "http://m\u00fcnchen\u3002de/": "http://m\u00fcnchen.de/",
            # IP literals serialize without a root dot in WHATWG,
            # as before — separator spellings agree there already.
            "http://8.8.8.8\u3002/": "http://8.8.8.8/",
            "http://8\u30028\u30028\u30028/": "http://8.8.8.8/",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)

    def test_returns_the_mapped_host_for_compatibility_spellings(self):
        # Judging NFKC-maps the host (app.py: fullwidth/compatibility
        # characters fold to their ordinary forms), but the return
        # path rebuilt from the mapped host only for percent-decoded
        # and literal-separator spellings, so a fullwidth spelling
        # came back verbatim while the Worker returns the mapped
        # form it judged (#665). The return now rebuilds whenever
        # the mapping changed the host spelling.
        cases = {
            "http://\uff45\uff58\uff41\uff4d\uff50\uff4c\uff45.com/": "http://example.com/",
            "http://\uff25\uff38\uff21\uff2d\uff30\uff2c\uff25.com/": "http://example.com/",
            "http://\uff47\uff50\uff54\uff12.example.com/": "http://gpt2.example.com/",
            "http://\uff45\uff58\uff41\uff4d\uff50\uff4c\uff45.com/path": "http://example.com/path",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Verdicts are unchanged: dotted spellings of blocked
        # targets stay refused, and a second separator is still an
        # empty label, in every separator spelling.
        self.assertRefused("http://localhost\u3002/")
        self.assertRefused("http://localhost\uff61/")
        self.assertRefused("http://127.0.0.1\u3002/")
        self.assertRefused("http://example.com\u3002\u3002/")
        self.assertRefused("http://example.com\uff0e\uff0e/")
        # Redirect targets inherit the fix: the handler fetches the
        # validated (dotted) form of a raw Location carrying one.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://example.org\u3002/path"
        )
        self.assertEqual(followed.full_url, "http://example.org./path")

    def test_returns_the_host_without_uts46_ignored_codepoints(self):
        # Judging deletes the UTS-46 "ignored" codepoints from the
        # host before every check (#526), but the non-percent return
        # path returned the raw spelling verbatim, so the recorded
        # URL kept codepoints the Worker deletes in its returned
        # host — and U+180E cannot be IDNA-encoded at fetch time at
        # all (#652). The return now rebuilds from the judged host
        # whenever the deletion changed it (the issue's option (a):
        # exactly the ignored codepoints are dropped; every other
        # spelling, IDN included, keeps its established form).
        # Every codepoint in the ignored set, interior to a label:
        for cp in app._UTS46_IGNORED:
            ch = chr(cp)
            self.assertEqual(
                validate_crawl_url("http://exa" + ch + "mple.com/"),
                "http://example.com/",
                hex(cp),
            )
        cases = {
            # Leading, trailing and repeated positions, and more
            # than one label carrying a codepoint.
            "http://\u200bexample.com/": "http://example.com/",
            "http://example.com\u200b/": "http://example.com/",
            "http://ex\u200ba\u200bmple.com/": "http://example.com/",
            "http://exa\u00admple.co\ufeffm/": "http://example.com/",
            # The root dot survives the rebuild, in either order.
            "http://exa\u200bmple.com./": "http://example.com./",
            "http://example.com.\u200b/": "http://example.com./",
            # Port, userinfo, path and the empty ?/# markers are
            # preserved around the rebuilt host.
            "http://exa\u200bmple.com:8080/path?q=1#frag": (
                "http://example.com:8080/path?q=1#frag"
            ),
            "http://user@exa\u2060mple.com/": "http://user@example.com/",
            "http://example.com\u200b/path?#frag": "http://example.com/path?#frag",
            "http://example.com\u200b?#": "http://example.com/?#",
            # The percent-encoded spelling of an ignored codepoint
            # decodes to the same judged host and returns it too.
            "http://exa%e2%80%8bmple.com/": "http://example.com/",
            "http://exa%C2%ADmple.com/": "http://example.com/",
            # An IDN host keeps its U-label spelling (#659 covers
            # the A-label form); only the codepoint is deleted.
            "http://m\u00fc\u200bnchen.de/": "http://m\u00fcnchen.de/",
            # An IP literal carrying one returns the canonical
            # address, as IP literals always do.
            "http://8\u200b.8.8.8/": "http://8.8.8.8/",
            # Controls: hosts with no ignored codepoint are
            # untouched, including the #596 U+0345 spelling (judged
            # by its mapping, not deleted) and ordinary IDN.
            "http://example.com/": "http://example.com/",
            "http://m\u00fcnchen.de/": "http://m\u00fcnchen.de/",
            "http://ex\u0345ample.com/": "http://ex\u0345ample.com/",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Verdicts are unchanged: blocked names spelled with an
        # ignored codepoint inside them stay refused, and a label
        # consisting only of ignored codepoints is still an empty
        # label / a hostless URL once they are deleted.
        self.assertRefused("http://local\u200bhost/")
        self.assertRefused("http://local\u00adhost/")
        self.assertRefused("http://127\u200b.0.0.1/")
        self.assertRefused("http://\u200b.com/")
        self.assertRefused("http://\u200b/")
        # Redirect targets inherit the fix: the handler fetches the
        # validated (deleted) form of a raw Location carrying one —
        # including U+180E, which the fetcher could not have
        # IDNA-encoded in its raw form.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://example.org\u200b/path"
        )
        self.assertEqual(followed.full_url, "http://example.org/path")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://exa\u180emple.org/"
        )
        self.assertEqual(followed.full_url, "http://example.org/")

    def test_returns_the_extraction_folded_host_for_kelvin_sign_spellings(self):
        # urlparse's hostname extraction folds U+212A KELVIN SIGN to
        # ASCII "k" before the NFKC step runs, so #666's mapped-host
        # measurement never saw the original spelling and the raw
        # Kelvin host came back verbatim while the Worker returns
        # the folded form it judged (#667). The return now rebuilds
        # from the judged host whenever extraction itself folded the
        # spelling to a different ASCII form.
        cases = {
            # Kelvin sign as a full label and inside a label.
            "http://\u212a.example.com/": "http://k.example.com/",
            "http://\u212a.com/": "http://k.com/",
            "http://\u212aexample.com/": "http://kexample.com/",
            "http://exa\u212aple.com/": "http://exakple.com/",
            # Port, userinfo, path and the empty ?/# markers are
            # preserved around the rebuilt host; default ports drop.
            "http://\u212aexample.com:8080/p": "http://kexample.com:8080/p",
            "http://\u212a.example.com:80/": "http://k.example.com/",
            "https://\u212a.example.com:443/": "https://k.example.com/",
            "http://user@\u212a.example.com/": "http://user@k.example.com/",
            "http://\u212aexample.com/path?#frag": ("http://kexample.com/path?#frag"),
            "http://\u212aexample.com?#": "http://kexample.com/?#",
            # The root dot survives the rebuild.
            "http://\u212a.example.com./": "http://k.example.com./",
            # Controls: the percent-encoded and mixed spellings keep
            # the returns they already had, and spellings the
            # pipeline does not change are untouched — ordinary
            # hosts, ASCII case folds (already normalized) and IDN
            # U-label spellings (#659 covers the A-label form).
            "http://%e2%84%aa.example.com/": "http://k.example.com/",
            "http://\u212a\uff45.com/": "http://ke.com/",
            "http://example.com/": "http://example.com/",
            "http://EXAMPLE.COM/": "http://example.com/",
            "http://m\u00fcnchen.de/": "http://m\u00fcnchen.de/",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Verdicts are unchanged: a blocked name spelled with the
        # Kelvin sign as its first label stays refused.
        self.assertRefused("http://\u212a.localhost/")
        # Redirect targets inherit the fix: the handler fetches the
        # validated (folded) form of a raw Location carrying one.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://\u212a.example.org/path"
        )
        self.assertEqual(followed.full_url, "http://k.example.org/path")

    def test_returns_empty_password_userinfo_without_the_colon(self):
        # WHATWG serialises userinfo from the parsed username/password:
        # the ":" separator appears only when the password is non-empty,
        # and the whole "@" drops when both are empty. The container
        # round-tripped the raw userinfo string, so "user:@" came back
        # as "user:@" and ":@" as ":@" while the Worker returns "user@"
        # and no userinfo at all (#651) — #639 dropped the bare "@" but
        # missed the colon-bearing spellings, which are non-empty raw
        # strings. Fetch behaviour already agreed (_split_userinfo
        # treats ":@" as carrying no credentials); only the recorded
        # URL diverged.
        cases = {
            "http://user:@example.com/": "http://user@example.com/",
            "http://:@example.com/": "http://example.com/",
            "http://:@example.com:80/": "http://example.com/",
            "http://user:@example.com:8080/a": "http://user@example.com:8080/a",
            "http://:@8.8.8.8/": "http://8.8.8.8/",
            # Controls: a non-empty password keeps its separator, and
            # a ":" inside the password is data, encoded as %3A.
            "http://:pass@example.com/": "http://:pass@example.com/",
            "http://::@example.com/": "http://:%3A@example.com/",
            "http://user::@example.com/": "http://user:%3A@example.com/",
            "http://user:pass@example.com/": "http://user:pass@example.com/",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Blocked targets in these spellings stay refused.
        self.assertRefused("http://:@127.0.0.1/")
        self.assertRefused("http://user:@localhost/")

    def test_percent_encodes_non_ascii_outside_the_host(self):
        # WHATWG percent-encodes non-ASCII characters in the userinfo,
        # path, query and fragment as UTF-8; urlparse hands them through
        # verbatim, and http.client then raises UnicodeEncodeError on
        # the returned URL before any request is sent — a target both
        # validators accepted 502s in the container (#615). The returned
        # URL carries the encoded form; the host keeps its spelling
        # (judged in mapped form, fetched via IDNA) and existing %XX
        # sequences are not double-encoded.
        cases = {
            "http://example.com/café": "http://example.com/caf%C3%A9",
            "http://example.com/日本語": (
                "http://example.com/%E6%97%A5%E6%9C%AC%E8%AA%9E"
            ),
            "http://example.com/?q=café": "http://example.com/?q=caf%C3%A9",
            "http://example.com/?q=\U0001f600": ("http://example.com/?q=%F0%9F%98%80"),
            "http://example.com/x#café": "http://example.com/x#caf%C3%A9",
            "http://user:café@example.com/": ("http://user:caf%C3%A9@example.com/"),
            "http://café@example.com/": "http://caf%C3%A9@example.com/",
            "http://münchen.de/café": "http://münchen.de/caf%C3%A9",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # Already-encoded spellings pass through byte-identical.
        self.assertAccepted("http://example.com/caf%C3%A9")
        self.assertAccepted("http://example.com/?q=caf%C3%A9")

    def test_percent_encodes_c0_space_and_del_outside_the_host(self):
        # PR #612 stripped tab/LF/CR, but the rest of the C0 range,
        # space and DEL were returned verbatim outside the host, and
        # http.client then raises InvalidURL on a URL both validators
        # accepted (#614). WHATWG percent-encodes those characters
        # inside the path/query/fragment/userinfo and trims leading
        # and trailing C0-or-space from the whole input; the returned
        # URL does the same, so it is the URL the fetcher can fetch.
        cases = {
            "http://example.com/pa\x00th": "http://example.com/pa%00th",
            "http://example.com/pa\x01th": "http://example.com/pa%01th",
            "http://example.com/pa th": "http://example.com/pa%20th",
            "http://example.com/pa\x7fth": "http://example.com/pa%7Fth",
            "http://example.com/?q=a\x00b": "http://example.com/?q=a%00b",
            "http://example.com/?q=a\x7fb": "http://example.com/?q=a%7Fb",
            "http://example.com/x#a\x00b": "http://example.com/x#a%00b",
            "http://user\x00name@example.com/": ("http://user%00name@example.com/"),
            "http://example.com/pa\x00?q=1": "http://example.com/pa%00?q=1",
            # Leading/trailing C0-or-space is trimmed, not encoded:
            # the trailing NUL never becomes part of the path.
            "\x00http://example.com/": "http://example.com/",
            "\x01 http://example.com/": "http://example.com/",
            "http://example.com/\x00": "http://example.com/",
            "http://example.com/pa ": "http://example.com/pa",
        }
        for raw, expected in cases.items():
            self.assertEqual(validate_crawl_url(raw), expected, raw)
        # No accepted spelling comes back carrying a raw control,
        # space or DEL anywhere in the returned URL.
        for raw in cases:
            returned = validate_crawl_url(raw)
            self.assertFalse(
                any(ord(c) <= 0x20 or ord(c) == 0x7F for c in returned), raw
            )
        # Already-encoded spellings pass through byte-identical, and
        # verdicts are unchanged: blocked hosts wrapped in these
        # characters, and controls inside the host, stay refused.
        self.assertAccepted("http://example.com/pa%00th")
        self.assertRefused("\x01http://127.0.0.1/")
        self.assertRefused("http://127.0.0.1/pa th")
        self.assertRefused("http://exa\x00mple.com/")
        self.assertRefused("http://exam ple.com/")

    def test_redirect_handler_follows_the_c0_encoded_target(self):
        # Redirect targets are validated raw (#614, following #529):
        # the handler must fetch the percent-encoded form of a target
        # whose path carries a raw control or space.
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://example.com/")
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://example.org/pa th\x00"
        )
        self.assertEqual(followed.full_url, "http://example.org/pa%20th")

    def test_redirect_handler_derives_auth_from_the_targets_userinfo(self):
        # #622: a redirect target carrying userinfo is fetched with
        # the userinfo stripped and its own credentials as the
        # Authorization header; a target without userinfo inherits
        # nothing from the credentialed request that led to it.
        import base64
        import urllib.request

        handler = app._ValidatingRedirectHandler()
        req = urllib.request.Request("http://first:secret@example.com/start")
        app._apply_userinfo_auth(req)
        self.assertEqual(req.full_url, "http://example.com/start")
        self.assertEqual(
            req.get_header("Authorization"),
            "Basic " + base64.b64encode(b"first:secret").decode("ascii"),
        )
        followed = handler.redirect_request(
            req, None, 302, "Found", {}, "http://user:pass@example.org/land"
        )
        self.assertEqual(followed.full_url, "http://example.org/land")
        self.assertEqual(
            followed.get_header("Authorization"),
            "Basic " + base64.b64encode(b"user:pass").decode("ascii"),
        )
        for target in ("http://example.org/plain", "http://@example.org/plain"):
            followed = handler.redirect_request(req, None, 302, "Found", {}, target)
            self.assertEqual(followed.full_url, "http://example.org/plain", target)
            self.assertIsNone(followed.get_header("Authorization"), target)


class TestFetchCrawl(unittest.TestCase):
    # _fetch_crawl fetches an already-validated URL (validation is
    # validate_crawl_url's job, pinned above), so these tests point it
    # at a local server directly. Error-status pages are crawl results,
    # not container failures (#572): urllib raises HTTPError for
    # 4xx/5xx, and the Worker fallback extracts those pages and reports
    # their real status_code — the container must do the same instead
    # of turning them into a 502 that makes the Worker re-crawl and
    # record the wrong source.

    @classmethod
    def setUpClass(cls):
        import threading
        from http.server import BaseHTTPRequestHandler, HTTPServer

        body = (
            b"<html><head><title>Missing Thing</title></head>"
            b"<body><a href='/x'>x</a> not here</body></html>"
        )
        seen = []
        cls.seen = seen

        class Target(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                seen.append((self.path, self.headers.get("Authorization")))
                if self.path == "/to-userinfo":
                    port = self.server.server_address[1]
                    self.send_response(302)
                    self.send_header(
                        "Location", f"http://user:pass@127.0.0.1:{port}/landing"
                    )
                    self.send_header("content-length", "0")
                    self.end_headers()
                    return
                if self.path == "/to-plain":
                    self.send_response(302)
                    self.send_header("Location", "/landing")
                    self.send_header("content-length", "0")
                    self.end_headers()
                    return
                if self.path == "/loop":
                    self.send_response(302)
                    self.send_header("Location", "/loop")
                    self.send_header("content-length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                if self.path == "/choices":
                    self.send_response(300)
                    self.send_header("Location", "/ok")
                    self.send_header("content-length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                if self.path.startswith("/hop"):
                    n = int(self.path[4:])
                    if n < 12:
                        self.send_response(302)
                        self.send_header("Location", f"/hop{n + 1}")
                        self.send_header("content-length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                        return
                code = {"/gone": 404, "/oops": 500}.get(self.path, 200)
                self.send_response(code)
                self.send_header("content-type", "text/html")
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        cls.server = HTTPServer(("127.0.0.1", 0), Target)
        cls.port = cls.server.server_address[1]
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def fetch(self, path):
        return app._fetch_crawl(f"http://127.0.0.1:{self.port}{path}")

    def test_error_status_pages_are_results_not_502s(self):
        for path, expected in (("/gone", 404), ("/oops", 500)):
            status, raw = self.fetch(path)
            self.assertEqual(status, expected, path)
            self.assertIn(b"Missing Thing", raw, path)

    def test_ok_page_unchanged(self):
        status, raw = self.fetch("/ok")
        self.assertEqual(status, 200)
        self.assertIn(b"Missing Thing", raw)

    def test_redirect_failures_raise_not_a_302_result(self):
        # #576: urllib raises HTTPError carrying the 3xx code for
        # redirect failures (loop, chain past max_redirections, and
        # non-followed 3xx). Those are crawl failures, not results —
        # they must propagate so do_POST returns 502 and the Worker
        # fallback runs, instead of recording a successful 302 crawl.
        # The plain opener stands in for the validating one here: the
        # validating handler refuses 127.0.0.1 targets by design, which
        # would mask the HTTPError handling under test; the redirect
        # mechanics are urllib's either way.
        import urllib.error
        import urllib.request
        from unittest import mock

        plain = urllib.request.build_opener()
        with mock.patch.object(app, "_OPENER", plain):
            for path in ("/loop", "/hop0", "/choices"):
                with self.assertRaises(urllib.error.HTTPError, msg=path) as ctx:
                    self.fetch(path)
                self.assertTrue(300 <= ctx.exception.code < 400, path)
            # Control: a short chain under the limit still succeeds.
            status, raw = self.fetch("/hop10")
            self.assertEqual(status, 200)
            self.assertIn(b"Missing Thing", raw)

    def test_fetches_percent_encoded_non_ascii_targets(self):
        # #615: the form validate_crawl_url returns for a non-ASCII
        # path/query is fetchable end to end (the raw spelling raises
        # UnicodeEncodeError inside http.client before any request).
        url = app._url_with_encoded_non_ascii(
            f"http://127.0.0.1:{self.port}/café?q=café"
        )
        self.assertEqual(url, f"http://127.0.0.1:{self.port}/caf%C3%A9?q=caf%C3%A9")
        status, raw = app._fetch_crawl(url)
        self.assertEqual(status, 200)
        self.assertIn(b"Missing Thing", raw)

    def test_fetches_percent_encoded_c0_targets(self):
        # #614: the form validate_crawl_url returns for a path/query
        # carrying a raw C0 control or space is fetchable end to end
        # (the raw spelling raises InvalidURL inside http.client
        # before any request).
        url = app._url_with_encoded_components(
            f"http://127.0.0.1:{self.port}/pa\x00th?q=a b\x7f"
        )
        self.assertEqual(url, f"http://127.0.0.1:{self.port}/pa%00th?q=a%20b%7F")
        status, raw = app._fetch_crawl(url)
        self.assertEqual(status, 200)
        self.assertIn(b"Missing Thing", raw)

    def test_fetches_the_resolved_path_for_dot_segment_targets(self):
        # #633: do_POST fetches the URL validate_crawl_url returns, so
        # a dot-segment target must reach the server in its resolved
        # form — the same path the Worker's WHATWG fetch requests —
        # not the verbatim spelling urllib used to send. (Validation
        # refuses loopback targets by design, so the validated public
        # URL's origin is swapped for the local server's, keeping the
        # exact validated path the container would fetch.)
        validated = app.validate_crawl_url("http://example.com/a/../b")
        self.assertEqual(validated, "http://example.com/b")
        local = validated.replace("http://example.com", f"http://127.0.0.1:{self.port}")
        self.seen.clear()
        status, raw = app._fetch_crawl(local)
        self.assertEqual(status, 200)
        self.assertIn(b"Missing Thing", raw)
        self.assertEqual(self.seen, [("/b", None)])
        validated = app.validate_crawl_url("http://example.com/%2e%2e/admin")
        self.assertEqual(validated, "http://example.com/admin")
        local = validated.replace("http://example.com", f"http://127.0.0.1:{self.port}")
        self.seen.clear()
        status, _ = app._fetch_crawl(local)
        self.assertEqual(status, 200)
        self.assertEqual(self.seen, [("/admin", None)])

    def test_fetches_userinfo_targets_with_basic_auth(self):
        # #622: every userinfo spelling both validators accept 502'd
        # in the container — urllib kept "user@host" as the hostname
        # and getaddrinfo failed before any request was sent (the
        # #615/#618 fetcher tests only ever fetched path/query forms).
        # The fetcher now strips the userinfo and sends the
        # percent-decoded credentials as Authorization: Basic, the way
        # WHATWG fetch does; a userinfo with empty username AND empty
        # password ("@", ":@") carries no credentials and sends none.
        import base64

        def basic(credentials):
            return "Basic " + base64.b64encode(credentials).decode("ascii")

        cases = (
            ("user@", basic(b"user:")),
            ("user:pass@", basic(b"user:pass")),
            # The #619 returned form: the password colon is data.
            ("user:pa%3Ass@", basic(b"user:pa:ss")),
            # The #615 returned form: credentials decode as UTF-8.
            ("caf%C3%A9@", basic("café:".encode("utf-8"))),
            ("caf%C3%A9:p%C3%A4ss@", basic("café:päss".encode("utf-8"))),
            (":pass@", basic(b":pass")),
            ("user:@", basic(b"user:")),
            ("@", None),
            (":@", None),
            ("", None),
        )
        for userinfo, expected_auth in cases:
            self.seen.clear()
            status, raw = app._fetch_crawl(f"http://{userinfo}127.0.0.1:{self.port}/x")
            self.assertEqual(status, 200, userinfo)
            self.assertIn(b"Missing Thing", raw, userinfo)
            self.assertEqual(self.seen, [("/x", expected_auth)], userinfo)

    def test_redirect_to_userinfo_target_fetches_with_its_auth(self):
        # #622: redirect targets carrying userinfo fetch the same way
        # as userinfo start URLs, and a credentialed start URL's
        # Authorization is not carried onto a plain redirect target.
        # The validating handler refuses 127.0.0.1 targets by design,
        # so validation is stubbed to isolate the redirect/auth
        # mechanics (the same reason the #576 test uses a plain
        # opener); per-hop validation itself is pinned elsewhere.
        import base64
        import urllib.request
        from unittest import mock

        def basic(credentials):
            return "Basic " + base64.b64encode(credentials).decode("ascii")

        opener = urllib.request.build_opener(app._ValidatingRedirectHandler)
        with (
            mock.patch.object(app, "_OPENER", opener),
            mock.patch.object(app, "validate_crawl_url", lambda url: url),
        ):
            self.seen.clear()
            status, _ = app._fetch_crawl(f"http://127.0.0.1:{self.port}/to-userinfo")
            self.assertEqual(status, 200)
            self.assertEqual(
                self.seen,
                [("/to-userinfo", None), ("/landing", basic(b"user:pass"))],
            )
            self.seen.clear()
            status, _ = app._fetch_crawl(
                f"http://first:secret@127.0.0.1:{self.port}/to-plain"
            )
            self.assertEqual(status, 200)
            self.assertEqual(
                self.seen,
                [("/to-plain", basic(b"first:secret")), ("/landing", None)],
            )

    def test_ascii_punctuation_percent_encoded_per_component(self):
        # #618: WHATWG's per-component percent-encode sets for ASCII
        # punctuation — the container returned these verbatim while the
        # Worker fetches the encoded form. Path set is the widest of
        # path/query/fragment; the query set must NOT gain the path
        # extras (a raw backtick in a query is what the Worker fetches).
        v = app.validate_crawl_url
        self.assertEqual(
            v('http://example.com/a"b<c>d'), "http://example.com/a%22b%3Cc%3Ed"
        )
        self.assertEqual(
            v("http://example.com/a^b`c{d}e"), "http://example.com/a%5Eb%60c%7Bd%7De"
        )
        self.assertEqual(
            v('http://example.com/?q=a"b<c>d'), "http://example.com/?q=a%22b%3Cc%3Ed"
        )
        self.assertEqual(
            v("http://example.com/?q=a^b`c{d}e"), "http://example.com/?q=a^b`c{d}e"
        )
        self.assertEqual(
            v('http://example.com/#a"b<c>d`e'), "http://example.com/#a%22b%3Cc%3Ed%60e"
        )
        self.assertEqual(v("http://example.com/#a^b{c}"), "http://example.com/#a^b{c}")
        # Userinfo: the separator colon stays, a password colon and the
        # rest of the userinfo set encode.
        self.assertEqual(v('http://us"er@example.com/'), "http://us%22er@example.com/")
        self.assertEqual(
            v("http://user:pa:ss@example.com/"), "http://user:pa%3Ass@example.com/"
        )
        self.assertEqual(v("http://us;er@example.com/"), "http://us%3Ber@example.com/")
        # Query also encodes the apostrophe (WHATWG special-query set),
        # and userinfo brackets are data, not an IPv6 literal.
        self.assertEqual(v("http://example.com/?q=a'b"), "http://example.com/?q=a%27b")
        self.assertEqual(
            v("http://a[b]c@example.com/"), "http://a%5Bb%5Dc@example.com/"
        )
        # Existing escapes are never double-encoded.
        self.assertEqual(v("http://example.com/a%22b"), "http://example.com/a%22b")

    def test_transport_failure_still_raises(self):
        import urllib.error

        # Nothing listens on port 1: a transport failure must keep
        # propagating so do_POST returns a genuine 502.
        with self.assertRaises(urllib.error.URLError):
            app._fetch_crawl("http://127.0.0.1:1/nope")


if __name__ == "__main__":
    unittest.main()
