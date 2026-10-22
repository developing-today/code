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

    def test_refuses_non_http_schemes_and_local_names(self):
        for url in (
            "ftp://example.com/",
            "file:///etc/passwd",
            "http://localhost:8787/",
            "http://printer.local/",
            "http://nas.internal/",
            "http://localhost./",
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
            "https://[::ffff:8.8.8.8]/",  # IPv4-mapped public
        ):
            self.assertAccepted(url)

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
        # accepted the IPv6 zone id (#505).
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

    def test_accepts_public_targets(self):
        for url in (
            "https://example.com/page",
            "http://8.8.8.8/",
            "http://134744072/",  # 8.8.8.8 in integer spelling: public, allowed
            "https://[2606:4700:4700::1111]/",
            "http://example.com./",  # FQDN root-dot spelling of a public name
            "http://dead.beef/",  # hex-looking labels, non-numeric final label
            "http://a1.beef/",
            "http://123.example.com/",  # numeric label, but not the final one
            "http://example.com:8080/",  # an ordinary explicit port
        ):
            self.assertAccepted(url)


if __name__ == "__main__":
    unittest.main()
