#!/usr/bin/env python3
"""Lab Ops Hub crawler container.

Tiny stdlib-only HTTP service. POST /crawl {"url": "..."} -> fetches the page
with a real HTTP client, extracts title / link count / readable text, returns
JSON in the same shape the Worker's fallback produces. Runs only on demand
(Worker starts it via getContainer, sleepAfter=2m kills it when idle), so it
burns container minutes only while crawling.
"""

import glob
import ipaddress
import json
import re
import socket
import ssl
import unicodedata
import urllib.error
import urllib.request
from html.parser import HTMLParser
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse

# Crawl-target policy (#441): the Worker validates before dispatching,
# and this service re-validates independently (defence in depth — it is
# the component that actually fetches). Only public http(s) targets:
# no other schemes, no localhost-style names, no non-global IP literals.
# Redirect targets are held to the same policy.
_BLOCKED_SUFFIXES = (".localhost", ".local", ".internal", ".lan", ".home.arpa")

# UTS-46 "ignored" codepoints are mapped to the empty string by IDNA
# processing, so the Worker's u.hostname simply does not contain them:
# the soft hyphen, the Mongolian vowel separators / free variation
# selectors, ZWSP, the invisible operators and the BOM/ZWNBSP. Judging
# a spelling that still carries them misjudges everything downstream —
# "local\u00adhost" is localhost to the resolver and to the Worker,
# and a label of 60 "a"s plus ten soft hyphens is a 60-character label,
# not a 77-character punycode one (#526).
# The characters UTS-46 treats as label separators: the ASCII full
# stop plus the three ideographic / fullwidth forms that
# validate_crawl_url translates to "." after mapping.
_UTS46_SEPARATORS = ".\u3002\uff0e\uff61"


def _maps_to_label_separator(c: str) -> bool:
    # True when c's compatibility mapping introduces a label
    # separator. The mapping must be tested against every separator
    # the translate step applies, not just ".": U+FE12's NFKC mapping
    # is U+3002 itself, so an ASCII-dot-only test let it slip through
    # and be judged as two innocent labels (#593).
    return any(s in unicodedata.normalize("NFKC", c) for s in _UTS46_SEPARATORS)


_UTS46_IGNORED = frozenset(
    [0x00AD, 0x034F, 0x200B, 0xFEFF]
    + list(range(0x180B, 0x1810))
    + list(range(0x2060, 0x2065))
    + list(range(0x206A, 0x2070))
)

# Codepoints UTS-46/IDNA marks disallowed that survive NFKC unchanged,
# so neither the space/control arm of the forbidden-character check
# nor any later check catches them, while the Worker's WHATWG parser
# rejects the URL outright (#516): the Ogham space mark, the line /
# paragraph separators, Arabic format marks, the bidi marks, embedding
# / override / isolate controls, ZWNJ / ZWJ outside their joining
# context, and the replacement character. (The Unicode space
# separators U+2000-200A, U+202F, U+205F and U+3000 need no entry here:
# NFKC maps them to an ordinary space, refused by the first arm.)
_IDNA_DISALLOWED = frozenset(
    [0x0600, 0x061C, 0x1680, 0x200C, 0x200D, 0x200E, 0x200F, 0x2028, 0x2029, 0xFFFD]
    + list(range(0x202A, 0x202F))
    + list(range(0x2066, 0x206A))
)


def validate_crawl_url(raw):
    # WHATWG URL parsing treats a backslash as a path delimiter for the
    # special schemes (http/https): it ends the authority, so the host
    # of "http://127.0.0.1\\@example.com/" is 127.0.0.1. urlparse instead
    # hands the backslash through as a netloc character, reads the text
    # after the "@" as the host, and would judge — and fetch — a host
    # the WHATWG parser never named, in both directions
    # ("example.com\\@127.0.0.1" is example.com to WHATWG but reads as
    # the loopback address here) (#577). Normalize first and judge the
    # normalized spelling: every check below, and the URL this function
    # returns (and the fetcher therefore fetches), then names the same
    # host the Worker's parser names. Only a raw backslash is
    # structural — a percent-encoded %5C is data and is untouched, as
    # are non-http(s) spellings (the scheme check below refuses them).
    if "\\" in raw and re.match(r"(?i)https?:", raw):
        raw = raw.replace("\\", "/")
    # WHATWG consumes the whole delimiter run after a special scheme's
    # colon — 0, 1, 3 or more slashes name the same host as exactly 2 —
    # while urlparse only sees a netloc behind exactly "//", so the
    # other run lengths parse as hostless and were refused as "url has
    # no host" even for public targets the Worker accepts and
    # normalizes (#586). Collapse the run to exactly "//" before
    # parsing and judge (and return) the normalized spelling, as with
    # the backslash normalization above. Blocked hosts in these
    # spellings are then refused by the ordinary checks below, for the
    # right reason.
    if re.match(r"(?i)https?:", raw):
        raw = re.sub(r"(?i)^(https?:)/*", r"\g<1>//", raw)
    u = urlparse(raw)
    if u.scheme not in ("http", "https"):
        raise ValueError("only http: and https: URLs may be crawled")
    # A port that is present must parse as an integer in range: urlparse
    # only raises for it lazily (when .port is read, deep inside the
    # fetcher), so read it here and refuse up front — the Worker's
    # WHATWG URL parser rejects these spellings outright (#502).
    try:
        u.port
    except ValueError:
        raise ValueError("url has an invalid port") from None
    host = (u.hostname or "").lower()
    # WHATWG URL parsing percent-decodes the host before judging it —
    # %65xample.com is example.com, %6cocalhost is localhost — while
    # urlparse hands the spelling through verbatim (#505, #529). Decode
    # first and judge the decoded form with every check below, the way
    # the Worker does, instead of refusing the whole class: spellings
    # that decode to blocked targets stay refused by the ordinary
    # checks, and the invalid spellings WHATWG rejects outright
    # (malformed % sequences, undecodable bytes, a decoded % or a
    # decoded host delimiter) are refused inside _percent_decode_host.
    # A % in a bracketed host is an IPv6 zone id ([fe80::1%25eth0]),
    # which WHATWG also rejects outright — never decode those. When a
    # host was decoded, the URL this function returns (and therefore
    # the URL the crawler fetches) carries the decoded host: urllib
    # does not percent-decode hosts, so returning the raw spelling
    # would fetch a name that was never judged.
    percent_decoded = False
    if "%" in host:
        if u.netloc.rsplit("@", 1)[-1].startswith("["):
            raise ValueError(f"refusing to crawl host with an IPv6 zone id {host}")
        host = _percent_decode_host(host)
        percent_decoded = True
    # Judge the host in the mapped form its resolver will see: IDNA
    # processing (encodings.idna / getaddrinfo, like UTS-46 in the
    # Worker's WHATWG parser) NFKC-maps fullwidth/compatibility
    # characters and treats U+3002, U+FF0E and U+FF61 as label
    # separators, so "ｌｏｃａｌｈｏｓｔ" resolves to localhost and
    # "127。0。0。1" resolves to 127.0.0.1. Checking the raw spelling
    # would certify those as public DNS names while the fetcher
    # resolves the blocked target (#527). Map first, then run every
    # check below on the mapped form; the original URL is still what
    # gets fetched, and public controls map onto public names
    # ("ｅｘａｍｐｌｅ.com" -> example.com, "münchen。de" -> münchen.de).
    # UTS-46 also deletes its "ignored" codepoints outright, so drop
    # those too: with them still present, "local\u00adhost" misses the
    # blocked-host check below and the length check measures a label
    # the Worker never sees (#526).
    # UTS-46 treats exactly three characters as label separators
    # (U+3002, U+FF0E, U+FF61, translated just below); any other
    # character whose compatibility mapping introduces a separator —
    # U+2024 (one dot leader) and the full-stop digits U+2488-U+249B
    # map to ".", and U+FE12 maps to U+3002 itself (#593) — is
    # IDNA-disallowed, but after NFKC mapping it would be judged
    # as two innocent labels, so refuse it before mapping (#583).
    if any(
        ord(c) > 0x7F and c not in "\u3002\uff0e\uff61" and _maps_to_label_separator(c)
        for c in host
    ):
        raise ValueError(
            f"refusing to crawl host with a character that maps to a label separator {host!r}"
        )
    host = unicodedata.normalize("NFKC", host).lower()
    host = host.translate({0x3002: ".", 0xFF0E: ".", 0xFF61: "."})
    host = host.translate({cp: None for cp in _UTS46_IGNORED})
    # NFKC mapping can itself CREATE a host delimiter the earlier
    # checks never saw: _percent_decode_host refuses delimiters in
    # the decoded form, but a percent-encoded compatibility character
    # decodes to an innocent-looking codepoint whose NFKC mapping is
    # a delimiter — U+FF0F -> /, U+FF1A -> :, U+FF20 -> @, U+FF03 ->
    # #, U+FF1F -> ?, U+FF3C -> \, U+FF3B/U+FF3D -> [ ], U+FF05 -> %,
    # and expansions like U+2100 -> "a/c" and U+2105 -> "c/o" (#567).
    # Raw spellings cannot smuggle these past urlparse (its NFKC
    # netloc guard raises), but the percent-encoded spelling is pure
    # ASCII in the raw netloc, so that guard never fires — and the
    # spliced returned URL would make the fetcher connect to a host
    # that was never judged ("localhost%ef%bc%8fexample.com" came
    # back as "localhost/example.com"). Re-check the structural
    # characters on the fully mapped form: refuse any mapped
    # delimiter, including a mapped % (the decode step guarantees no
    # % survives, and mapping must not reintroduce one). A colon is
    # only legitimate inside a bracketed IPv6 literal; an unbracketed
    # mapped colon is a delimiter, not an address.
    if any(c in "%#/?@[\\]" for c in host) or (
        ":" in host and not u.netloc.rsplit("@", 1)[-1].startswith("[")
    ):
        raise ValueError(
            f"refusing to crawl host with a forbidden character once mapped {host!r}"
        )
    # A single trailing dot is the DNS root dot (the FQDN spelling of the
    # same host: "localhost." is "localhost", "127.0.0.1." is 127.0.0.1);
    # strip it before the checks so the dotted spelling cannot evade
    # them, matching the Worker's validateCrawlUrl (#466).
    if host.endswith("."):
        host = host[:-1]
    if not host:
        raise ValueError("url has no host")
    # An empty label (a second trailing dot, an interior "..", or a
    # leading dot) is not a valid DNS name at all, and the one-dot strip
    # above would let "localhost.." slip past the blocked-host checks as
    # "localhost." — refuse it instead of certifying it as a public DNS
    # name, matching the Worker's validator (#511). IPv6 literals
    # contain ":" and are judged as addresses below, not by label
    # structure.
    if ":" not in host and any(label == "" for label in host.split(".")):
        raise ValueError(f"refusing to crawl host with an empty label {host}")
    # A label over 63 octets, or a name over 253 characters, can never
    # be a DNS name at all (RFC 1035 §2.3.4: 63-octet labels, 255-octet
    # wire names = 253 characters of text without the root dot, which
    # the strip above already removed) — refuse it up front instead of
    # certifying it as a public DNS name and letting the resolver fail
    # on it deep inside the fetcher, matching the Worker's validator
    # (#513). The host here is already the UTS-46-mapped form (NFKC
    # applied and ignored codepoints deleted above), so a label that
    # mapping expands (U+FDFA) or shrinks (soft hyphens) is measured
    # the way the Worker measures it (#526). The limits apply to the
    # IDNA-encoded (punycode) form —
    # the Worker's u.hostname is already punycode — so judge each
    # label's A-label length, not its character count: character
    # length is only a lower bound, and an IDN label whose characters
    # fit can still encode past the limit (#517). IPv6 literals are
    # judged as addresses, as above.
    if ":" not in host:
        encoded_labels = [
            label
            if label.isascii()
            else "xn--" + label.encode("punycode").decode("ascii")
            for label in host.split(".")
        ]
        if len(".".join(encoded_labels)) > 253 or any(
            len(label) > 63 for label in encoded_labels
        ):
            raise ValueError(
                f"refusing to crawl host with an overlong label or name {host}"
            )
    # An A-label (an "xn--" label) must be judged by the label it
    # encodes, not by its ASCII spelling: the spelling is pure ASCII,
    # so no check above or below ever sees the decoded characters —
    # "xn--9hbcp.xn--8hb.xn--8hb.xn--9hb" is ١٢٧.٠.٠.١ (127.0.0.1 in
    # Arabic-Indic digits, refused in raw form by _judge_idna_label's
    # AN rule below) yet sailed through as an ordinary public
    # DNS name, while the Worker's WHATWG parser decodes the A-label
    # and rejects it outright (#580). Decode each A-label and hold the
    # decoded form to the same policy; an A-label that does not decode
    # to exactly the label it claims to encode is not a valid IDNA
    # label at all and is refused on the same grounds.
    if ":" not in host:
        for label in host.split("."):
            if label.startswith("xn--"):
                _judge_a_label(label, host)
    # A raw (non-A-label) label must be a valid IDNA label, not merely
    # a string that punycode-encodes: the mapped host above was never
    # IDNA-validated, so labels carrying unassigned codepoints, format
    # characters, a leading combining mark, or a bidi-rule violation
    # (an LTR label mixing in RTL characters, an RTL label that does
    # not start with one) were certified as public DNS names while
    # the Worker's WHATWG domain-to-ASCII refuses the URL outright
    # (#583). Judge every non-ASCII label with the same helper the
    # A-label path applies to its decoded form.
    if ":" not in host:
        for label in host.split("."):
            if not label.isascii():
                _judge_idna_label(label, host)
    # (Percent-encoded hosts are no longer refused wholesale here:
    # they were decoded and judged in their decoded form above, and
    # _percent_decode_host guarantees no % survives into this host —
    # #529 supersedes #505's blanket refusal.)
    # A space, control or WHATWG-forbidden character in the host is not
    # a DNS name: WHATWG rejects it outright, urlparse hands it through
    # verbatim, and http.client only raises for it deep inside the
    # fetcher — the same lazy-failure shape #502 fixed for ports. Refuse
    # up front: C0 controls and space (#507), plus DEL, the C1 controls
    # and NBSP (0x7F-0xA0) and the forbidden host codepoints < > ^ |
    # (#510), plus the IDNA-disallowed codepoints that survive NFKC
    # (#516). The Arabic-Indic digits (U+0660-U+0669) are NOT refused
    # here: they are bidi class AN, and WHATWG's domain-to-ASCII
    # refuses them only outside a right-to-left label — the blanket
    # range #574 added here refused them inside one too (ا١.com,
    # مرحبا١.com), while the Worker accepts that context (#605).
    # _judge_idna_label's AN rule, which runs on every non-ASCII
    # label above, makes the contextual judgement: it refuses the
    # digits in an LTR label, in a label with no R/AL character —
    # including ١٢٧.٠.٠.١, the Arabic-Indic spelling of 127.0.0.1
    # (#574) — and mixed with a European digit, and accepts them
    # inside a genuine RTL label, matching the Worker. The other
    # decimal-digit families (Devanagari, Extended Arabic-Indic
    # U+06F0-U+06F9, Bengali, Thai, ...) are accepted by WHATWG as
    # ordinary IDN characters and stay accepted here. Non-ASCII
    # above NBSP that is a valid IDN character (e.g. é) is untouched.
    if any(
        ord(c) <= 0x20
        or 0x7F <= ord(c) <= 0xA0
        or c in "<>^|"
        or ord(c) in _IDNA_DISALLOWED
        for c in host
    ):
        raise ValueError(f"refusing to crawl host with a forbidden character {host!r}")
    if host == "localhost" or host.endswith(_BLOCKED_SUFFIXES):
        raise ValueError(f"refusing to crawl local host {host}")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        ip = _inet_aton_ipv4(host)
        if ip is None:
            # A host whose final label is numeric is an IP literal in
            # the URL grammar (WHATWG "ends in a number"), not a DNS
            # name: no numeric TLDs exist. If it reached here it is an
            # IPv4 spelling nothing accepts (256.1.1.1, 1.2.3.4.5,
            # 4294967296) or a name like example.123 / example.0x7f —
            # refuse it instead of reclassifying it as a public DNS
            # name, matching the Worker's validator (#502).
            last_label = host.rsplit(".", 1)[-1]
            # The hex alternative allows zero digits: WHATWG's
            # IPv4-number parser returns 0 for a bare 0x/0X prefix, so
            # `example.0x` and the single-label `0x` (0.0.0.0) also end
            # in a number — refuse them too (#504). A trailing non-hex
            # character (0xg, 0x7fg) still fails the full match and
            # stays a DNS name, as in WHATWG.
            if re.fullmatch(r"0[xX][0-9a-fA-F]*|[0-9]+", last_label):
                raise ValueError(
                    f"refusing to crawl host {host}: numeric final label "
                    "but not a valid IPv4 address"
                )
            # A bracketed host occupied the IP-literal position
            # ([v1.fe80::], [v7.a] and other IPvFuture forms) but is not
            # a valid IPv6 literal — urlparse strips the brackets, so
            # look at the netloc. Fail closed instead of reclassifying
            # it as a public DNS name, matching the Worker's validator
            # (#507).
            if u.netloc.rsplit("@", 1)[-1].startswith("["):
                raise ValueError(
                    f"refusing to crawl host {host}: bracketed literal "
                    "but not a valid IPv6 address"
                )
            # A public DNS name; literal-IP policy does not apply.
            # A percent-decoded host is fetched in its decoded form.
            return _url_with_host(u, host) if percent_decoded else raw
    embedded = _embedded_ipv4(ip)
    if embedded is not None:
        # Translation/mapped prefixes are judged by the IPv4 they embed,
        # not by ipaddress's wholesale verdict on the prefix (#472).
        # is_global alone passes multicast and the 192.88.99.0/24 6to4
        # relay anycast range, so refuse those explicitly (#476).
        if (
            not embedded.is_global
            or embedded.is_multicast
            or embedded in _IPV4_6TO4_RELAY_ANYCAST
        ):
            raise ValueError(
                f"refusing to crawl address {host} embedding non-public IPv4 {embedded}"
            )
    elif isinstance(ip, ipaddress.IPv4Address) and (
        ip.is_multicast or ip in _IPV4_6TO4_RELAY_ANYCAST
    ):
        # ipaddress reports multicast (224.0.0.0/4) and 192.88.99.0/24 as
        # is_global; neither is a public unicast crawl target (#476).
        raise ValueError(f"refusing to crawl non-public address {host}")
    elif isinstance(ip, ipaddress.IPv6Address) and (
        ip not in _GLOBAL_UNICAST_V6 or ip in _IPV6_DOCUMENTATION
    ):
        # ipaddress.is_global is True for any IPv6 address outside its
        # narrow private list — multicast (ff00::/8), site-local
        # (fec0::/10), SRv6 SIDs (5f00::/16) and the reserved space
        # outside global unicast all pass it (#474). Global unicast
        # lives in 2000::/3; require that first, as the Worker's
        # validateCrawlUrl does, then let is_global judge within it
        # (it knows the 2001::/23 special-purpose block). The RFC 9637
        # documentation range 3fff::/20 sits inside 2000::/3 and
        # is_global's verdict on it depends on the interpreter's bundled
        # special-purpose registry, so refuse it explicitly (#519).
        raise ValueError(f"refusing to crawl non-public address {host}")
    elif not ip.is_global:
        raise ValueError(f"refusing to crawl non-public address {host}")
    # A percent-decoded host is fetched in its decoded form (#529).
    return _url_with_host(u, host) if percent_decoded else raw


def _judge_a_label(label, host):
    # Judge one A-label ("xn--...") by its punycode-decoded form
    # (#580). The Worker's WHATWG domain-to-ASCII decodes the label
    # and applies full UTS-46 validity to the result, so an A-label is
    # refused here when: it does not decode at all, it decodes to an
    # empty or all-ASCII label (a real A-label encodes a non-ASCII
    # U-label — the all-ASCII spelling is the name itself, so
    # "xn--localhost-" encodes "localhost", a name no A-label may
    # certify), re-encoding the decoded label does not reproduce the
    # spelling exactly (not the canonical encoding of anything), or
    # the decoded label carries a character the forbidden-character
    # policy in validate_crawl_url refuses in raw form (controls,
    # space, < > ^ |, and the _IDNA_DISALLOWED set). The Arabic-Indic
    # digits U+0660-U+0669 are not in that set (#605): the
    # _judge_idna_label call at the end of this function judges them
    # by their bidi AN context, exactly as in raw form.
    body = label[4:]
    try:
        decoded = body.encode("ascii").decode("punycode")
    except (UnicodeError, ValueError):
        raise ValueError(
            f"refusing to crawl host {host}: A-label {label} does not decode"
        ) from None
    if not decoded or decoded.isascii():
        raise ValueError(
            f"refusing to crawl host {host}: A-label {label} "
            "decodes to an empty or ASCII label"
        )
    if decoded.encode("punycode").decode("ascii") != body:
        raise ValueError(
            f"refusing to crawl host {host}: A-label {label} "
            "is not the canonical encoding of its decoded label"
        )
    if any(
        ord(c) <= 0x20
        or 0x7F <= ord(c) <= 0xA0
        or c in "<>^|"
        or ord(c) in _IDNA_DISALLOWED
        for c in decoded
    ):
        raise ValueError(
            f"refusing to crawl host {host}: A-label {label} "
            f"decodes to a label with a forbidden character {decoded!r}"
        )
    # The decoded label must also be a valid IDNA label in its own
    # right — the same judgement the raw-form path applies (#583).
    _judge_idna_label(decoded, host)


def _judge_idna_label(label, host):
    # Judge one non-ASCII label (a raw mapped label, or an A-label's
    # decoded form) for IDNA/UTS-46 validity (#583). The Worker's
    # WHATWG domain-to-ASCII applies full validity, so a label is
    # refused here when:
    # - it carries an unassigned codepoint (category Cn — UTS-46
    #   marks it disallowed, but the raw punycode codec happily
    #   encodes it) or a format character (category Cf — IDNA
    #   disallows them all; the CONTEXTJ pair ZWNJ/ZWJ is refused by
    #   the forbidden-character checks before this helper runs);
    # - it begins with a combining mark (UTS-46 requires a label to
    #   start with a non-mark);
    # - it violates the bidi rule (RFC 5893): a label may not mix
    #   strong LTR (bidi class L) with strong RTL (R / AL) characters,
    #   and an RTL label must start with an R / AL character. The
    #   stdlib IDNA codec's own bidi check misses post-Unicode-3.2
    #   RTL letters (e.g. U+0620, U+06EE, U+05EF pass
    #   "ex\u0620ample".encode("idna")), so the directionality check
    #   is spelled out here, and the codec call below is kept for its
    #   remaining nameprep/bidi coverage (it does catch the classic
    #   Hebrew-in-Latin mixing, e.g. "ex\u05d0ample");
    # - it carries an Arabic decimal / thousands separator
    #   (U+066B / U+066C, IDNA-disallowed) in a label with strong LTR
    #   characters — outside an Arabic-script context the Worker
    #   refuses them, while a pure Arabic label carrying one stays
    #   accepted, matching the Worker;
    # - it carries a character of bidi class AN (Arabic number)
    #   outside a right-to-left label, or mixes AN with EN (European
    #   number) in one label (#595): AN is only valid inside an RTL
    #   label, and never alongside EN. The BMP AN codepoints are the
    #   Arabic-Indic digits U+0660-U+0669 themselves (judged by this
    #   rule since #605 removed #574's blanket range — the U+066B /
    #   U+066C case above is subsumed by it too), but the 41 astral
    #   AN codepoints — the Hanifi Rohingya digits U+10D30-U+10D39
    #   and the Rumi digits U+10E60-U+10E7E — passed every other
    #   check, and an AN+EN mix inside an RTL label passed too;
    # - the stdlib IDNA codec rejects it.
    # Known residuals, measured on the #583 differential battery
    # (13,045 hosts vs the Worker's validator under Bun): this
    # interpreter's unicodedata is 15.0.0, so a few dozen codepoints
    # assigned in later Unicode versions read as Cn here while the
    # Worker's newer tables accept them (the same version skew PR
    # #584 accepted for A-labels).
    # UTS-46 mapping is not NFKC: U+0345 (combining Greek
    # ypogegrammeni) maps to U+03B9 (ι, bidi class L), but NFKC
    # leaves it unchanged (the mapping lives in the special-casing
    # data), so the mapped host validate_crawl_url hands over still
    # carries it as a combining mark (bidi NSM), legal mid-label in
    # any context. Judge the UTS-46-mapped form: inside an RTL
    # label the mapped letter makes the label R + L + R, a bidi
    # violation the Worker refuses, while at label start the mapped
    # letter is an ordinary first character, closing the former
    # leading-mark over-refusal (#596).
    label = label.translate({0x0345: "ι"})
    if any(_maps_to_label_separator(c) for c in label if ord(c) > 0x7F):
        # A decoded A-label is judged unmapped, so the mapped-dot
        # refusal in validate_crawl_url never sees this form.
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "contains a character that maps to a label separator"
        )
    if any(unicodedata.category(c) in ("Cn", "Cf") for c in label):
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "contains an unassigned or format character"
        )
    if unicodedata.category(label[0]).startswith("M"):
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "begins with a combining mark"
        )
    bidis = [unicodedata.bidirectional(c) for c in label]
    has_ltr = "L" in bidis
    has_rtl = any(b in ("R", "AL") for b in bidis)
    if has_ltr and has_rtl:
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "mixes left-to-right and right-to-left characters"
        )
    if has_rtl and bidis[0] not in ("R", "AL"):
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "is a right-to-left label that does not start with one"
        )
    if has_ltr and any(ord(c) in (0x066B, 0x066C) for c in label):
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "contains an Arabic separator outside an Arabic label"
        )
    if "AN" in bidis and (has_ltr or not has_rtl):
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "contains an Arabic number outside a right-to-left label"
        )
    if "AN" in bidis and "EN" in bidis:
        raise ValueError(
            f"refusing to crawl host {host}: label {label!r} "
            "mixes Arabic and European numbers"
        )
    try:
        label.encode("idna")
    except UnicodeError as e:
        # The codec's non-bidi verdicts are a useful backstop (its
        # stringprep tables prohibit e.g. the ideographic description
        # characters U+2FF0-U+2FFB, which every other check here
        # passes). Its bidi verdicts are IDNA-2003's, stricter than
        # RFC 5893: it refuses an RTL label ending in a European
        # digit ("label1" spelled with a Hebrew label), which the
        # Worker accepts — so a bidi-only failure is re-judged by the
        # RFC 5893 trailing rule (the last non-NSM character of an
        # RTL label is R / AL / AN / EN; of an LTR label, L / EN) on
        # top of the start/mixing rules already applied above.
        if "BIDI" not in str(e):
            raise ValueError(
                f"refusing to crawl host {host}: label {label!r} "
                "is not a valid IDNA label"
            ) from None
        trailing = list(bidis)
        while trailing and trailing[-1] == "NSM":
            trailing.pop()
        last = trailing[-1] if trailing else ""
        if (has_rtl and last not in ("R", "AL", "AN", "EN")) or (
            not has_rtl and has_ltr and last not in ("L", "EN")
        ):
            raise ValueError(
                f"refusing to crawl host {host}: label {label!r} "
                "ends with a character its directionality forbids"
            ) from None


_GLOBAL_UNICAST_V6 = ipaddress.IPv6Network("2000::/3")


_IPV6_DOCUMENTATION = ipaddress.IPv6Network("3fff::/20")


_NAT64_WELL_KNOWN = ipaddress.IPv6Network("64:ff9b::/96")

_IPV4_6TO4_RELAY_ANYCAST = ipaddress.IPv4Network("192.88.99.0/24")


def _percent_decode_host(host):
    # Percent-decode a host the way WHATWG URL parsing does before it
    # judges the host (#529): every % must be a complete %XX hex escape,
    # the decoded bytes must be valid UTF-8, and the decoded host must
    # not still contain a % (a %25 double-encoded spelling, which
    # WHATWG's domain processing rejects) or any of the host delimiters
    # a raw spelling can never carry into u.hostname because urlparse
    # splits on them (# / : ? @ [ \ ]) — after decoding they would
    # change what the fetcher connects to. (Controls, space and the
    # < > ^ | forbidden set need no arm here: validate_crawl_url's
    # forbidden-character check refuses them on the decoded form.)
    raw_bytes = bytearray()
    i = 0
    while i < len(host):
        c = host[i]
        if c == "%":
            esc = host[i + 1 : i + 3]
            if len(esc) != 2 or any(ch not in "0123456789abcdefABCDEF" for ch in esc):
                raise ValueError(
                    f"refusing to crawl host with an invalid percent escape {host}"
                )
            raw_bytes.append(int(esc, 16))
            i += 3
        else:
            raw_bytes.extend(c.encode("utf-8"))
            i += 1
    try:
        decoded = bytes(raw_bytes).decode("utf-8")
    except UnicodeDecodeError:
        raise ValueError(
            f"refusing to crawl host that is not valid UTF-8 once decoded {host}"
        ) from None
    if "%" in decoded or any(c in "#/:?@[\\]" for c in decoded):
        raise ValueError(
            f"refusing to crawl host with a forbidden character once decoded {host}"
        )
    return decoded


def _url_with_host(u, new_host):
    # Rebuild a parsed URL with new_host in place of the netloc's host,
    # preserving userinfo, port, path, query and fragment. Used for
    # percent-decoded hosts (#529): the fetcher must connect to the
    # decoded host that was judged, not the raw spelling.
    netloc = u.netloc
    if "@" in netloc:
        userinfo, _, hostport = netloc.rpartition("@")
        prefix = userinfo + "@"
    else:
        prefix, hostport = "", netloc
    if hostport.startswith("["):
        _, _, suffix = hostport.partition("]")
        return u._replace(netloc=f"{prefix}[{new_host}]{suffix}").geturl()
    if ":" in hostport:
        _, _, port = hostport.partition(":")
        return u._replace(netloc=f"{prefix}{new_host}:{port}").geturl()
    return u._replace(netloc=prefix + new_host).geturl()


def _embedded_ipv4(ip):
    # Prefixes that embed an IPv4 address are judged by that address, the
    # same policy as plain IPv4 (#472). ipaddress folds the IPv4-mapped
    # case into is_global, but its verdicts on the translation prefixes
    # are wholesale: the whole NAT64 well-known prefix 64:ff9b::/96 is
    # "global" even when it embeds 127.0.0.1, and the whole 6to4 range
    # 2002::/16 (IPv6Address.sixtofour) is "private" even when it embeds
    # a public address.
    if not isinstance(ip, ipaddress.IPv6Address):
        return None
    if ip.ipv4_mapped is not None:
        return ip.ipv4_mapped
    if ip.sixtofour is not None:
        return ip.sixtofour
    if ip in _NAT64_WELL_KNOWN:
        return ipaddress.IPv4Address(int(ip) & 0xFFFFFFFF)
    return None


def _inet_aton_ipv4(host):
    # ipaddress only accepts canonical dotted-quad, but the resolver the
    # fetcher uses (getaddrinfo, inet_aton semantics) also interprets the
    # short (127.1), integer (2130706433), hex (0x7f.1) and octal
    # (0177.0.0.1) spellings as IPv4 (#468). Judge those spellings by the
    # address they resolve to instead of mistaking them for DNS names.
    # Only hosts made solely of numeric labels reach inet_aton, so
    # ordinary DNS names are never affected.
    if not re.fullmatch(r"[0-9a-fA-FxX.]+", host) or not any(c.isdigit() for c in host):
        return None
    try:
        return ipaddress.IPv4Address(socket.inet_aton(host))
    except (OSError, ValueError):
        return None


class _ValidatingRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # Follow the validated form, not the raw Location: a redirect
        # target is validated raw (no Worker normalisation in between),
        # and a percent-encoded target must be fetched as the decoded
        # host that was judged (#529).
        newurl = validate_crawl_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


_OPENER = None  # built after CTX below (the HTTPS handler carries it)

# The Nix image's CA bundle lives at a store path; do not rely on the
# container runtime propagating the image's SSL_CERT_FILE env var —
# pass the CA file to the TLS context explicitly.
_ca = glob.glob("/nix/store/*cacert*/etc/ssl/certs/ca-bundle.crt")
CTX = ssl.create_default_context(cafile=_ca[0]) if _ca else ssl.create_default_context()
_OPENER = urllib.request.build_opener(
    _ValidatingRedirectHandler, urllib.request.HTTPSHandler(context=CTX)
)


def _fetch_crawl(url):
    # Fetch a validated URL and return (status, body capped at 2 MB).
    # urllib raises HTTPError for 4xx/5xx, but an HTTPError is also the
    # response: an error-status page is still a crawl result (the
    # Worker fallback extracts it and reports its real status_code),
    # not a container failure — so read its body and status instead of
    # surfacing it as a 502 (#572). Transport failures (URLError,
    # timeouts) are not HTTPError and keep propagating to the caller's
    # 502 path.
    # Only 4xx/5xx HTTPError is a result, though: urllib's redirect
    # handler also raises HTTPError carrying the 3xx code for redirect
    # failures (loop / max_repeats, chain past max_redirections, and
    # non-followed 3xx), and those are crawl failures — they must keep
    # propagating so do_POST returns 502 and the Worker fallback runs,
    # instead of being recorded as a successful 302 crawl (#576).
    req = urllib.request.Request(
        url, headers={"User-Agent": "LabOpsHub-Crawler/0.1 (container)"}
    )
    try:
        response = _OPENER.open(req, timeout=20)
    except urllib.error.HTTPError as e:
        if not 400 <= e.code < 600:
            raise
        response = e
    with response as res:
        status = getattr(res, "status", None) or res.getcode()
        return status, res.read(2_000_000)


class Extractor(HTMLParser):
    def __init__(self):
        super().__init__()
        self.title = ""
        self._in_title = False
        self.links = 0
        self.text = []
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        if tag == "title":
            self._in_title = True
        elif tag == "a":
            self.links += 1
        elif tag in ("script", "style"):
            self._skip += 1

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False
        elif tag in ("script", "style") and self._skip:
            self._skip -= 1

    def handle_data(self, data):
        if self._in_title:
            self.title += data
        elif not self._skip:
            self.text.append(data)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # keep stdout quiet
        pass

    def _send(self, code, payload):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self._send(200, {"ok": True, "service": "labops-crawler"})

    def do_POST(self):
        if self.path != "/crawl":
            return self._send(404, {"error": "POST /crawl only"})
        try:
            length = int(self.headers.get("content-length", "0"))
            url = json.loads(self.rfile.read(length))["url"]
            try:
                # Fetch the validated form: identical to the request
                # except for a percent-encoded host, which is fetched
                # decoded, as judged (#529).
                url = validate_crawl_url(url)
            except ValueError as e:
                return self._send(400, {"error": str(e)})
            # TLS uses the pinned CA bundle via the opener's HTTPS
            # handler; error-status pages come back as results (#572).
            status, raw = _fetch_crawl(url)
            html = raw.decode("utf-8", errors="replace")
            ex = Extractor()
            ex.feed(html)
            text = re.sub(r"\s+", " ", " ".join(ex.text)).strip()
            self._send(
                200,
                {
                    "title": ex.title.strip(),
                    "excerpt": text[:500],
                    "links": ex.links,
                    "status_code": status,
                    "text": text[:20000],
                },
            )
        except Exception as e:  # noqa: BLE001 - surface any crawl failure as JSON
            self._send(502, {"error": str(e)})


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
