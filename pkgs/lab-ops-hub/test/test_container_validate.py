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
        ):
            self.assertAccepted(url)

    def test_accepts_public_targets(self):
        for url in (
            "https://example.com/page",
            "http://8.8.8.8/",
            "http://134744072/",  # 8.8.8.8 in integer spelling: public, allowed
            "https://[2606:4700:4700::1111]/",
            "http://example.com./",  # FQDN root-dot spelling of a public name
        ):
            self.assertAccepted(url)


if __name__ == "__main__":
    unittest.main()
