#!/usr/bin/env python3
"""Reproduce the length/alphabet figures in the R1 Willow URI review.

Stdlib only. Builds Willow'25-instantiated willow:// URIs from fixed dummy
32-byte values (no real keys) and reports their lengths, DNS-label fit,
QR mode, and the growth caused by percent-encoding nested hint URIs.
"""
import json
import urllib.parse

NS = bytes(range(32)).hex()            # 64 hex chars, Willow'25 NamespaceId
SS = bytes(range(32, 64)).hex()        # 64 hex chars, SubspaceId
DG = bytes(range(64, 96)).hex()        # 64 hex chars, WILLIAM3 digest
TS_NOW_US = 849_000_000_000_000        # ~2026 in TAI microseconds since J2000

UNRESERVED = set(
    "-._~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz")


def percent_encode(s: str) -> str:
    # The spec's percent_encode: every byte that is not unreserved.
    return "".join(c if c in UNRESERVED else "%%%02X" % ord(c) for c in s)


def host(communal: bool = False) -> str:
    return ("+" if communal else "-") + NS + "." + SS


QR_ALNUM = set("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:")
# Byte-mode capacity (bytes) per QR version at ECC level M, ISO/IEC 18004.
QR_BYTE_M = {5: 84, 6: 106, 7: 122, 8: 152, 9: 180, 10: 213, 11: 251,
             12: 287, 13: 331, 14: 362, 15: 412, 16: 450, 17: 504,
             18: 560, 19: 624, 20: 666}


def qr_version_m(n: int):
    for v, cap in QR_BYTE_M.items():
        if n <= cap:
            return v
    return ">20"


cases = {}
base = "willow://" + host()
cases["entry_bare"] = base + "/blog/idea.txt"
cases["entry_digest"] = cases["entry_bare"] + "?digest=" + DG
cases["entry_digest_slice"] = cases["entry_digest"] + "&from=5&to=4096"
hint1 = "wtp://relay.example.net:1235/example"
hint2 = "https://mirror.example.org/willow/" + NS
cases["entry_digest_2hints"] = (cases["entry_digest"] + "&hints="
                                + percent_encode(hint1) + ";" + percent_encode(hint2))
cases["area_time"] = base + "/blog?area&count=12&from=%d" % TS_NOW_US

out = {}
for k, u in cases.items():
    labels = urllib.parse.urlsplit(u).netloc.split(".")
    out[k] = {
        "uri": u,
        "length": len(u),
        "longest_host_label": max(len(x) for x in labels),
        "fits_dns_label_63": all(len(x) <= 63 for x in labels),
        "qr_alphanumeric_mode_possible": set(u.upper()) <= QR_ALNUM and u == u.upper(),
        "qr_byte_mode_version_ecc_m": qr_version_m(len(u)),
        "fits_one_irc_line_~400": len(u) <= 400,
    }
h = hint2
out["hint_growth"] = {"hint": h, "raw": len(h), "percent_encoded": len(percent_encode(h))}
print(json.dumps(out, indent=2))
