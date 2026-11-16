#!/usr/bin/env python3
"""Property-based coverage for the target-identifier shape invariant (issue #6510).

Invariant: in a service definition, targets are host/class names printed
verbatim by `deploy list`, so they must match the single-line identifier regex
`^[a-z0-9][a-z0-9-]*$`. A registry with valid target identifiers must produce
no target-shape errors. Any registry containing a target string that violates
this grammar (e.g. whitespace, newlines, uppercase characters, leading hyphens,
empty strings, symbols, control characters, or non-ASCII characters) must fail
`validate(registry)` with an error containing:
"targets must be host/class names matching".

Oracle: an independent reference decision procedure `is_valid_target(s)` that
evaluates the invariant directly without using the implementation's regular
expression:
1. `s` is a `str` and `len(s) > 0`.
2. `s[0]` is an ASCII lowercase letter ('a'-'z') or ASCII digit ('0'-'9').
3. Every character in `s[1:]` is an ASCII lowercase letter, ASCII digit, or hyphen ('-').
If and only if these three conditions hold, the target is valid.

The generator produces hundreds of pseudo-random candidate target strings across
various classes:
- Valid targets: strings of length 1..64 formed by legal characters starting
  with [a-z0-9] and followed by [a-z0-9-].
- Invalid targets:
  - Empty strings
  - Leading hyphen strings (e.g. "-target", "---")
  - Strings with internal or trailing whitespace/newlines ('\n', '\r', ' ', '\t')
  - Strings with uppercase ASCII letters ('A'-'Z')
  - Strings with symbols and punctuation ('.', '_', '/', '@', ':', '!', etc.)
  - Strings with non-ASCII unicode characters (accents, emojis, Cyrillic, etc.)
  - Strings with control characters (0x00-0x1F, 0x7F)
  - Mixed valid prefixes with invalid characters appended or injected.

Stdlib-only (no external dependencies in this repo's Python tooling);
a fixed default seed keeps CI deterministic, and any failing case prints its seed and input.

Usage: test_target_shape_property.py [path-to-deploy.py] [--seed N] [--cases N]
"""

import importlib.util
import os
import random
import sys

VALID_START = [chr(c) for c in range(ord("a"), ord("z") + 1)] + [
    chr(c) for c in range(ord("0"), ord("9") + 1)
]
VALID_REST = VALID_START + ["-"]

INVALID_SYMBOLS = list(" !\"#$%&'()*+,./:;<=>?@[\\]^_`{|}~") + ["\n", "\r", "\t"]
UPPERCASE = [chr(c) for c in range(ord("A"), ord("Z") + 1)]
UNICODE_EXTRA = [
    "é",
    "ü",
    "ñ",
    "λ",
    "д",
    "€",
    "🚀",
    "—",
    "…",
    "\u0000",
    "\u001f",
    "\ufffd",
]

BASE_DEFN = {
    "package": "/nix/store/test-package-1.0",
    "entryPoint": "bin/svc",
    "args": [],
    "environment": {},
    "secrets": [],
    "user": "nobody",
    "ports": [],
    "restart": "always",
    "healthCheck": None,
    "class": "systemd",
}


def is_valid_target(s):
    """Reference oracle for target identifier validity."""
    if not isinstance(s, str) or len(s) == 0:
        return False
    first = s[0]
    if not (("a" <= first <= "z") or ("0" <= first <= "9")):
        return False
    for ch in s[1:]:
        if not (("a" <= ch <= "z") or ("0" <= ch <= "9") or ch == "-"):
            return False
    return True


def gen_valid_target(rng):
    length = rng.randint(1, 32)
    first = rng.choice(VALID_START)
    rest = "".join(rng.choice(VALID_REST) for _ in range(length - 1))
    return first + rest


def gen_invalid_target(rng):
    kind = rng.choice(
        [
            "empty",
            "leading_hyphen",
            "whitespace",
            "newline",
            "uppercase",
            "symbols",
            "unicode",
            "mixed_valid_prefix",
        ]
    )
    if kind == "empty":
        return ""
    if kind == "leading_hyphen":
        length = rng.randint(1, 10)
        return "-" + "".join(rng.choice(VALID_REST) for _ in range(length - 1))
    if kind == "whitespace":
        base = gen_valid_target(rng)
        pos = rng.randint(0, len(base))
        return base[:pos] + " " + base[pos:]
    if kind == "newline":
        base = gen_valid_target(rng)
        sep = rng.choice(["\n", "\r", "\r\n"])
        return base + sep + gen_valid_target(rng)
    if kind == "uppercase":
        base = gen_valid_target(rng)
        chars = list(base)
        chars[rng.randint(0, len(chars) - 1)] = rng.choice(UPPERCASE)
        return "".join(chars)
    if kind == "symbols":
        base = gen_valid_target(rng)
        sym = rng.choice(INVALID_SYMBOLS)
        pos = rng.randint(0, len(base))
        return base[:pos] + sym + base[pos:]
    if kind == "unicode":
        base = gen_valid_target(rng)
        u = rng.choice(UNICODE_EXTRA)
        pos = rng.randint(0, len(base))
        return base[:pos] + u + base[pos:]
    if kind == "mixed_valid_prefix":
        return gen_valid_target(rng) + ":" + gen_valid_target(rng)
    return "bad target"


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    seed, cases = 6510, 500
    for i, a in enumerate(argv):
        if a == "--seed":
            seed = int(argv[i + 1])
        if a == "--cases":
            cases = int(argv[i + 1])
    deploy_path = (
        args[0]
        if args
        else os.path.join(os.path.dirname(os.path.abspath(__file__)), "deploy.py")
    )
    spec = importlib.util.spec_from_file_location("deploy", deploy_path)
    deploy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(deploy)

    rng = random.Random(seed)

    for case in range(cases):
        # 1. Valid target case
        valid_s = gen_valid_target(rng)
        if not is_valid_target(valid_s):
            print(f"generator error: valid candidate rejected by oracle: {valid_s!r}")
            return 1

        reg_valid = {"svc": dict(BASE_DEFN, targets=[valid_s])}
        errors_valid = deploy.validate(reg_valid)
        target_errors = [e for e in errors_valid if "targets" in e]
        if target_errors:
            print(
                f"counterexample (valid target rejected) seed={seed} case={case}: "
                f"target={valid_s!r} errors={target_errors}"
            )
            return 1

        # 2. Invalid target case
        invalid_s = gen_invalid_target(rng)
        if is_valid_target(invalid_s):
            print(
                f"generator error: invalid candidate accepted by oracle: {invalid_s!r}"
            )
            return 1

        reg_invalid = {"svc": dict(BASE_DEFN, targets=[invalid_s])}
        errors_invalid = deploy.validate(reg_invalid)
        target_errors = [e for e in errors_invalid if "targets" in e]
        if not target_errors:
            print(
                f"counterexample (invalid target accepted) seed={seed} case={case}: "
                f"target={invalid_s!r}"
            )
            return 1

        # Verify error message precision
        if invalid_s == "":
            expected = "targets must be a non-empty list of host/class names"
        else:
            expected = "targets must be host/class names matching"
        if not any(expected in err for err in target_errors):
            print(
                f"counterexample (wrong error diagnostic) seed={seed} case={case}: "
                f"target={invalid_s!r} errors={target_errors} expected={expected!r}"
            )
            return 1

        # 3. Multi-target case: mix valid target with candidate
        multi_targets = [f"host-{case}", invalid_s]
        reg_multi = {"svc": dict(BASE_DEFN, targets=multi_targets)}
        errors_multi = deploy.validate(reg_multi)
        multi_errors = [e for e in errors_multi if "targets" in e]
        if not multi_errors or not any(expected in err for err in multi_errors):
            print(
                f"counterexample (multi-target invalid missed) seed={seed} case={case}: "
                f"targets={multi_targets!r} errors={multi_errors}"
            )
            return 1

    print(
        f"property holds: {cases} cases (seed {seed}); valid targets match is_valid_target oracle, "
        f"invalid targets rejected with target-shape error"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
