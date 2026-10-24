"""Mutation check for ticket-options. Each mutant is one text change to src/lib.rs, run in a scratch copy.

The real crate is never edited. The scratch copy uses its own target dir, so no artifacts are shared.
Run with: python3 -I scripts/mutate.py
"""

import os
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_CRATE = ROOT / "ticket-options"
SCRATCH = ROOT / "mutants" / "ticket-options"
TARGET = ROOT / "mutant-target"
TOOLCHAIN_BIN = "/home/user/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu/bin"

# (name, old text, new text). Each old text must occur exactly once in src/lib.rs.
MUTANTS = [
    (
        "M1 option B accepts trailing bytes (lenient parse)",
        "let (envelope, rest): (Envelope, &[u8]) = postcard::take_from_bytes(bytes)?;\n"
        "        if !rest.is_empty() || envelope.version != 1 {",
        "let (envelope, _rest): (Envelope, &[u8]) = postcard::take_from_bytes(bytes)?;\n"
        "        if envelope.version != 1 {",
    ),
    (
        "M2 option D skips the expected-signer check",
        "    if signer != *anchor {\n"
        '        return RouteStatus::Rejected("suffix not signed by the expected inviter");\n'
        "    }\n",
        "",
    ),
    (
        "M3 option C skips the ticket digest check",
        "    if event.body.ticket_digest != ticket_digest(ticket) {\n"
        '        return RouteStatus::Rejected("ticket is not the one the inviter signed");\n'
        "    }\n",
        "",
    ),
    (
        "M4 option D splits by re-encoding the ticket",
        "let payload_len = bytes.len() - suffix.len();",
        "let payload_len = ticket.encode_bytes().len();",
    ),
    (
        "M5 options B and D sign with no domain tag",
        "[DOMAIN_ENVELOPE, &ticket.encode_bytes()].concat()",
        "ticket.encode_bytes()",
    ),
    (
        "M6 option E refuses a plain ticket",
        "None => RouteStatus::Unverified,",
        'None => RouteStatus::Rejected("no event"),',
    ),
    (
        "M7 option C ignores the event author",
        "if event.body.author != *anchor {",
        "if false {",
    ),
]

# M5 also needs option D's domain removed on both sides, or the two messages still differ.
M5_EXTRA = [
    (
        "let signature = key.sign(&[DOMAIN_SUFFIX, &payload].concat());",
        "let signature = key.sign(&payload);",
    ),
    (
        "let message = [DOMAIN_SUFFIX, &split.payload].concat();",
        "let message = split.payload.clone();",
    ),
]

TEST_LINE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED)$", re.MULTILINE)


LOG_DIR = ROOT / "mutation-logs"


def run_tests(label):
    env = dict(os.environ)
    env["PATH"] = TOOLCHAIN_BIN + os.pathsep + env.get("PATH", "")
    env["RUSTC"] = TOOLCHAIN_BIN + "/rustc"
    env["CARGO_TARGET_DIR"] = str(TARGET)
    proc = subprocess.run(
        [TOOLCHAIN_BIN + "/cargo", "test", "--offline", "--test", "options"],
        cwd=SCRATCH,
        env=env,
        capture_output=True,
        text=True,
        timeout=900,
    )
    text = proc.stdout + proc.stderr
    LOG_DIR.mkdir(exist_ok=True)
    (LOG_DIR / f"{label}.txt").write_text(text)
    results = dict(TEST_LINE.findall(text))
    summary = re.search(r"test result: .*", text)
    return results, (summary.group(0) if summary else "no summary"), proc.returncode


def touch_tree(root):
    # copytree keeps the source mtimes, and cargo can then reuse a stale mutant build. Bump them.
    for dirpath, _dirs, files in os.walk(root):
        for name in files:
            os.utime(os.path.join(dirpath, name), None)


def main():
    if SCRATCH.parent.exists():
        shutil.rmtree(SCRATCH.parent)
    # A fresh target dir, so no mutant build can be reused for the baseline.
    shutil.rmtree(TARGET, ignore_errors=True)
    shutil.copytree(SRC_CRATE, SCRATCH, ignore=shutil.ignore_patterns("target"))
    touch_tree(SCRATCH)
    original = (SCRATCH / "src" / "lib.rs").read_text()

    results, summary, code = run_tests("baseline")
    print(f"BASELINE (no mutation): exit {code}; {summary}")
    failing = [name for name, state in results.items() if state == "FAILED"]
    print(f"  failing tests: {failing or 'none'}")

    for name, old, new in MUTANTS:
        text = original
        extra = M5_EXTRA if name.startswith("M5") else []
        ok = True
        for pair_old, pair_new in [(old, new)] + extra:
            if text.count(pair_old) != 1:
                ok = False
                break
            text = text.replace(pair_old, pair_new)
        if not ok:
            print(f"{name}: NOT APPLIED (pattern did not match exactly once)")
            continue
        (SCRATCH / "src" / "lib.rs").write_text(text)
        results, summary, code = run_tests(name.split(" ")[0])
        failing = [n for n, state in results.items() if state == "FAILED"]
        print(f"{name}: exit {code}; {summary}")
        print(f"  failing tests ({len(failing)}): {failing or 'none'}")
        (SCRATCH / "src" / "lib.rs").write_text(original)

    shutil.rmtree(SCRATCH.parent)
    print("scratch copy removed")


if __name__ == "__main__":
    main()
