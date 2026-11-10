"""Mutation check: break one step of the crate in a scratch copy and record which tests fail.

Each mutation must be applied exactly once to the copy, or the run is reported as not applied.
"""
import os
import re
import shutil
import subprocess

ROOT = "/home/user/work/spikes-next/s9-seitan"
BASE = os.path.join(ROOT, "seitan-v2")
SCRATCH = os.path.join(ROOT, "mutants")
CARGO = subprocess.check_output(["rustup", "which", "--toolchain", "1.97.0", "cargo"], text=True).strip()
# Scratch copies have the same package name and relative path as the real crate, so they must not share its
# target dir: cargo would reuse a mutant's artifacts for the real crate (and the reverse).
ENV = dict(os.environ, CARGO_TARGET_DIR=os.path.join(ROOT, "mutants", "target"))

MUTATIONS = [
    ("M1 eddsa stage label", "src/derive.rs", 'hmac_sha512(&sikey.0, &stage_payload("eddsa"))', 'hmac_sha512(&sikey.0, &stage_payload("eddsb"))'),
    ("M2 invite-ID tag byte", "src/derive.rs", "INVITE_ID_TAG: u8 = 0x27;", "INVITE_ID_TAG: u8 = 0x28;"),
    ("M3 scrypt r", "src/derive.rs", "SCRYPT_R: u32 = 8;", "SCRYPT_R: u32 = 1;"),
    ("M4 no stale check", "src/admit.rs", "if fields.ctime_ms.saturating_add(SKEW_MS) < self.created_ms {", "if false {"),
    ("M5 no use limit", "src/admit.rs", "if self.admitted.len() >= self.max_uses {", "if false {"),
    ("M6 no expiry check", "src/admit.rs", "if now_ms > self.expires_ms || fields.ctime_ms > self.expires_ms {", "if false {"),
    ("M7 signature not checked", "src/accept.rs", "key.verify_strict(&proof.message, &signature)\n        .map_err(|_| ProofError::BadSignature)", "let _ = (&key, &signature);\n        Ok(())"),
    ("M8 ctime before eldest_seqno", "src/accept.rs", "    eldest_seqno: u64,\n    ctime: u64,", "    ctime: u64,\n    eldest_seqno: u64,"),
    ("M9 use not recorded", "src/admit.rs", "self.admitted\n            .push((fields.uid.clone(), fields.eldest_seqno));", ""),
    ("M10 no revocation check", "src/admit.rs", "if self.revoked {", "if false {"),
]


def main():
    if os.path.exists(SCRATCH):
        shutil.rmtree(SCRATCH)
    shutil.copytree(BASE, os.path.join(SCRATCH, "base"))
    print("baseline (unmutated copy):")
    shutil.copytree(os.path.join(SCRATCH, "base"), os.path.join(SCRATCH, "work"))
    failed, passed, err = run_tests_at(os.path.join(SCRATCH, "work"))
    print(f"  passed={passed} failed={len(failed)} compile_error={err}")
    for t in failed:
        print(f"    FAILED {t}")

    for name, rel, old, new in MUTATIONS:
        work = os.path.join(SCRATCH, "work")
        shutil.rmtree(work)
        shutil.copytree(os.path.join(SCRATCH, "base"), work)
        path = os.path.join(work, rel)
        with open(path) as fh:
            text = fh.read()
        count = text.count(old)
        if count != 1:
            print(f"{name}: NOT APPLIED (pattern found {count} times)")
            continue
        with open(path, "w") as fh:
            fh.write(text.replace(old, new))
        failed, passed, err = run_tests_at(work)
        print(f"{name}: passed={passed} failed={len(failed)} compile_error={err}")
        for t in failed:
            print(f"    FAILED {t}")


def run_tests_at(path):
    proc = subprocess.run(
        [CARGO, "test", "--no-fail-fast"], cwd=path, env=ENV, capture_output=True, text=True
    )
    out = proc.stdout + proc.stderr
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED$", out, re.M)))
    passed = len(re.findall(r"^test \S+ \.\.\. ok$", out, re.M))
    # cargo prints "error: test failed" when tests fail, so match only real compile errors.
    compile_error = "could not compile" in out or re.search(r"^error\[E\d+\]", out, re.M) is not None
    return failed, passed, compile_error


if __name__ == "__main__":
    main()
