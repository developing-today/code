#!/usr/bin/env python3
"""Standalone validator for the 9110-R1 legacy-artifact research deliverables.

Issue: https://github.com/developing-today/code/issues/10146 (slice 9110-R1)

Run:  python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/validate_legacy_artifact_research.py

Stdlib only. Independently re-derives every claim in
pkgs/id/test-vectors/legacy-artifact-signatures.json:

* Legacy profile — rebuilds serde_json::to_vec(SignedBody) bytes from each
  vector's fields with a from-scratch serde_json-compatible serializer,
  compares byte-for-byte, recomputes SHA-256, and verifies the Ed25519
  signature with a pure-Python RFC 8032 verifier.
* New profile (design-stage) — re-encodes the deterministic-CBOR Body,
  checks the Body commits payload ID/length/domain/context and EXCLUDES the
  signature and the envelope ID from the signed preimage, verifies the
  detached signature over domain-prefix + Body, and recomputes EnvelopeId
  as SHA-256 over the complete canonical envelope including the signature.
* Report — checks doc/.../crosscut/research-results/legacy-artifact-profile.md
  carries the required findings (exact byte rule, doc/code mismatch,
  exclusion statement, manifest binding, statement IDs).

Exit code 0 iff every check passes.
"""

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
VECTORS = ROOT / "pkgs/id/test-vectors/legacy-artifact-signatures.json"
REPORT = (
    ROOT
    / "doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/research-results/legacy-artifact-profile.md"
)

FAILURES = []


def check(ok, label, detail=""):
    print(
        ("PASS " if ok else "FAIL ")
        + label
        + ((" -- " + detail) if (detail and not ok) else "")
    )
    if not ok:
        FAILURES.append(label)


# ---------------------------------------------------------------- serde_json
def sj_escape(s):
    out = []
    for ch in s:
        o = ord(ch)
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif o < 0x20:
            out.append("\\u%04x" % o)
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


def sj_value(v):
    if v is None:
        return "null"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        r = repr(v)
        return r
    if isinstance(v, str):
        return sj_escape(v)
    if isinstance(v, list):
        return "[" + ",".join(sj_value(x) for x in v) + "]"
    if isinstance(v, dict):
        # serde_json Map without preserve_order is a BTreeMap: sorted keys.
        return (
            "{" + ",".join(sj_escape(k) + ":" + sj_value(v[k]) for k in sorted(v)) + "}"
        )
    raise TypeError(repr(v))


def signed_body_bytes(author, seq, prev, kind, body, after):
    fields = [
        '"author":' + sj_escape(author),
        '"seq":' + str(seq),
        '"prev":' + sj_escape(prev),
        '"kind":' + sj_escape(kind),
        '"body":' + sj_value(body),
    ]
    if after:  # skip_serializing_if = "Vec::is_empty"
        fields.append('"after":' + sj_value(after))
    return ("{" + ",".join(fields) + "}").encode("utf-8")


# ------------------------------------------------------------------ Ed25519
_Q = 2**255 - 19
_L = 2**252 + 27742317777372353535851937790883648493
_D = -121665 * pow(121666, _Q - 2, _Q) % _Q
_I = pow(2, (_Q - 1) // 4, _Q)


def _xrecover(y):
    xx = (y * y - 1) * pow(_D * y * y + 1, _Q - 2, _Q) % _Q
    x = pow(xx, (_Q + 3) // 8, _Q)
    if (x * x - xx) % _Q != 0:
        x = x * _I % _Q
    if x % 2 != 0:
        x = _Q - x
    return x


_BY = 4 * pow(5, _Q - 2, _Q) % _Q
_B = [_xrecover(_BY), _BY]


def _edwards(P, Q):
    x1, y1 = P
    x2, y2 = Q
    t = _D * x1 * x2 * y1 * y2 % _Q
    return [
        (x1 * y2 + x2 * y1) * pow(1 + t, _Q - 2, _Q) % _Q,
        (y1 * y2 + x1 * x2) * pow(1 - t, _Q - 2, _Q) % _Q,
    ]


def _scalarmult(P, e):
    if e == 0:
        return [0, 1]
    Q = _scalarmult(P, e // 2)
    Q = _edwards(Q, Q)
    if e & 1:
        Q = _edwards(Q, P)
    return Q


def _decodeint(data):
    return int.from_bytes(data, "little")


def _decodepoint(data):
    y = _decodeint(data) & (2**255 - 1)
    x = _xrecover(y)
    if (x & 1) != (data[31] >> 7):
        x = _Q - x
    return [x, y]


def ed25519_verify(pubkey, msg, sig):
    if len(pubkey) != 32 or len(sig) != 64:
        return False
    A = _decodepoint(pubkey)
    R = _decodepoint(sig[:32])
    S = _decodeint(sig[32:])
    if S >= _L:
        return False
    h = int.from_bytes(hashlib.sha512(sig[:32] + pubkey + msg).digest(), "little")
    return _scalarmult(_B, S) == _edwards(R, _scalarmult(A, h))


# --------------------------------------------------------------------- CBOR
def cbor_head(major, n):
    mt = major << 5
    if n < 24:
        return bytes([mt | n])
    if n < 256:
        return bytes([mt | 24, n])
    if n < 65536:
        return bytes([mt | 25]) + n.to_bytes(2, "big")
    if n < 2**32:
        return bytes([mt | 26]) + n.to_bytes(4, "big")
    return bytes([mt | 27]) + n.to_bytes(8, "big")


def cbor_uint(n):
    return cbor_head(0, n)


def cbor_bytes(data):
    return cbor_head(2, len(data)) + data


def cbor_text(s):
    data = s.encode("utf-8")
    return cbor_head(3, len(data)) + data


def cbor_map(pairs):
    pairs = sorted(pairs, key=lambda kv: kv[0])  # deterministic: sort by encoded key
    out = cbor_head(5, len(pairs))
    for k, v in pairs:
        out += k + v
    return out


# --------------------------------------------------------------------- main
def main():
    doc = json.loads(VECTORS.read_text(encoding="utf-8"))
    check(
        doc["manifest_sha256"]
        == "3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06",
        "vectors manifest_sha256 binding",
    )
    author = bytes.fromhex(doc["test_key"]["author_public_key_hex"])

    legacy = doc["legacy_profile"]
    check(
        "length-prefixed" in legacy["doc_code_mismatch"]
        and "NO length prefixes" in legacy["doc_code_mismatch"],
        "legacy doc/code mismatch recorded",
    )
    for v in legacy["vectors"]:
        rebuilt = signed_body_bytes(
            v["author"], v["seq"], v["prev"], v["kind"], v["body"], v["after"]
        )
        check(
            rebuilt.hex() == v["signed_bytes_hex"],
            f"{v['name']}: serde_json bytes byte-for-byte",
        )
        check(
            rebuilt.decode("utf-8") == v["signed_bytes_utf8"],
            f"{v['name']}: utf8 form matches",
        )
        check(
            hashlib.sha256(rebuilt).hexdigest() == v["hash"],
            f"{v['name']}: sha256 == artifact hash",
        )
        check(
            ed25519_verify(author, rebuilt, bytes.fromhex(v["signature"])),
            f"{v['name']}: Ed25519 signature verifies",
        )

    newp = doc["new_profile"]
    ex = newp["example"]
    f = ex["body_fields"]
    check(
        set(f) == {"context", "domain", "payload_id_hex", "payload_len"},
        "new profile Body commits exactly payload ID, length, domain, context",
    )
    check(
        "signature" not in f
        and "envelope_id" not in f
        and "envelope_id_sha256_hex" not in f,
        "new profile Body excludes signature and EnvelopeId",
    )
    payload = ex["payload_utf8"].encode("utf-8")
    check(
        hashlib.sha256(payload).hexdigest()
        == f["payload_id_hex"]
        == ex["payload_sha256_hex"],
        "new profile PayloadId hashes raw payload bytes",
    )
    body = cbor_map(
        [
            (cbor_text("context"), cbor_text(f["context"])),
            (cbor_text("domain"), cbor_text(f["domain"])),
            (cbor_text("payload_id"), cbor_bytes(bytes.fromhex(f["payload_id_hex"]))),
            (cbor_text("payload_len"), cbor_uint(f["payload_len"])),
        ]
    )
    check(
        body.hex() == ex["body_cbor_hex"],
        "new profile Body deterministic CBOR re-encodes identically",
    )
    preimage = f["domain"].encode("utf-8") + b"\x00" + body
    check(
        preimage.hex() == ex["preimage_hex"],
        "new profile preimage = domain-prefix + Body",
    )
    check(
        bytes.fromhex(ex["signature_hex"]) not in preimage
        and bytes.fromhex(ex["envelope_id_sha256_hex"]) not in preimage,
        "new profile preimage contains neither signature nor EnvelopeId bytes",
    )
    check(
        ed25519_verify(author, preimage, bytes.fromhex(ex["signature_hex"])),
        "new profile detached signature verifies over domain-prefix + Body",
    )
    envelope = cbor_map(
        [
            (cbor_text("body"), cbor_bytes(body)),
            (cbor_text("signature"), cbor_bytes(bytes.fromhex(ex["signature_hex"]))),
        ]
    )
    check(
        envelope.hex() == ex["envelope_cbor_hex"],
        "new profile envelope re-encodes identically",
    )
    check(
        hashlib.sha256(envelope).hexdigest() == ex["envelope_id_sha256_hex"],
        "new profile EnvelopeId = SHA-256 over complete envelope including signature",
    )

    report = REPORT.read_text(encoding="utf-8")
    for needle, label in [
        ("serde_json::to_vec(SignedBody)", "report states exact legacy byte rule"),
        (
            "length-prefixed and domain-separated",
            "report quotes the doc comment mismatch",
        ),
        ("S0.7", "report cites S0.7"),
        ("S0.13", "report cites S0.13"),
        ("S0.15", "report cites S0.15"),
        (
            "3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06",
            "report carries manifest binding",
        ),
        (
            "excludes its signature and envelope ID from its own signed preimage",
            "report states new-profile exclusion",
        ),
        ("legacy-artifact-signatures.json", "report references the vectors file"),
    ]:
        check(needle in report, label)

    print()
    if FAILURES:
        print(f"VALIDATION FAILED: {len(FAILURES)} check(s) failed")
        return 1
    print(
        "VALIDATION PASSED: legacy bytes, hashes, signatures, new-profile invariants, and report fields all verify"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
