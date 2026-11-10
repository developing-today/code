#!/usr/bin/env python3
"""Packaged-binary smoke check (step 5, code#5355).

Run by pkgs/atlas-search/default.nix's installCheckPhase against
the BUILT binary, and runnable by hand against any atlas-search
binary:

    python3 fixtures/smoke_check.py /path/to/atlas-search

Three probes, all offline, in a scrubbed environment (temp HOME,
no ATLAS_* inherited):

1. FTS-only search over the fixture DB (fixtures/make_fixture_db.py)
   — stdout must equal the recorded Python envelope for the
   `fts_default` case in fixtures/core-expectations.json.
2. Embedded (hybrid) search over the same DB via the sanctioned
   ATLAS_SEARCH_QVEC_FILE fixture hook — the same monkeypatch the
   Python fixture generator uses — byte-compared with the recorded
   `hybrid_default` envelope. This is the model-free embedded
   query: the ONNX session itself is probe 3's subject and the
   packaged-artifact verification's (real model, production data).
3. The ONNX Runtime native binding loads from wherever the binary
   resolves it (the packaged build: its nix closure, baked in at
   build time): a vector search with an EMPTY model dir must fail
   at the tokenizer — which atlas-embed loads only AFTER
   ort::init_from succeeds — and must not report the runtime as
   missing. If the dylib could not be found or loaded, the error
   would name the runtime instead.

Exits 0 with PASS lines, 1 with the first failure's detail.
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
QUERY = "how do we back up the database"


def fail(msg: str) -> int:
    print(f"smoke_check: FAIL: {msg}", file=sys.stderr)
    return 1


def run(
    binary: Path, args: list, env_extra: dict, home: Path
) -> subprocess.CompletedProcess:
    env = {"PATH": os.environ.get("PATH", ""), "HOME": str(home)}
    env.update(env_extra)
    return subprocess.run(
        [str(binary), *args], env=env, capture_output=True, text=True, timeout=120
    )


def main() -> int:
    if len(sys.argv) != 2:
        return fail("usage: smoke_check.py /path/to/atlas-search")
    binary = Path(sys.argv[1])
    if not binary.is_file():
        return fail(f"binary not found: {binary}")

    expectations = json.loads((HERE / "core-expectations.json").read_text())
    cases = {c["name"]: c for c in expectations["search_cases"]}

    with tempfile.TemporaryDirectory(prefix="atlas-smoke-") as td:
        tmp = Path(td)
        home = tmp / "home"
        home.mkdir()
        db = tmp / "fixture.db"
        r = subprocess.run(
            [sys.executable, str(HERE / "make_fixture_db.py"), str(db)],
            capture_output=True,
            text=True,
        )
        if r.returncode != 0:
            return fail(f"make_fixture_db failed: {r.stderr.strip()}")
        qvec = Path(str(db) + ".qvec.json")

        tails = {"fts_default": ["--mode", "fts"], "hybrid_default": []}
        for name, tail in tails.items():
            r = run(
                binary,
                ["search", QUERY, "--backend", "local", "--db", str(db), *tail],
                {"ATLAS_SEARCH_QVEC_FILE": str(qvec)},
                home,
            )
            if r.returncode != 0:
                return fail(f"{name}: exit {r.returncode}, stderr: {r.stderr.strip()}")
            try:
                got = json.loads(r.stdout)
            except json.JSONDecodeError as e:
                return fail(f"{name}: stdout not JSON: {e}")
            if got != cases[name]["envelope"]:
                return fail(
                    f"{name}: envelope differs from the recorded Python envelope"
                )
            print(
                f"smoke_check: PASS {name} (envelope byte-equal to the Python recording)"
            )

        empty_model = tmp / "empty-model"
        empty_model.mkdir()
        r = run(
            binary,
            [
                "search",
                QUERY,
                "--backend",
                "local",
                "--db",
                str(db),
                "--mode",
                "vector",
            ],
            {"ATLAS_MODEL_DIR": str(empty_model)},
            home,
        )
        if r.returncode == 0:
            return fail(
                "ort probe: vector search unexpectedly succeeded with an empty model dir"
            )
        if "ONNX Runtime library not found" in r.stderr:
            return fail(
                "ort probe: binary could not resolve the ONNX Runtime library: "
                + r.stderr.strip()
            )
        if "tokenizer:" not in r.stderr:
            return fail(
                "ort probe: expected failure at the tokenizer (after runtime init), got: "
                + r.stderr.strip()
            )
        print(
            "smoke_check: PASS ort load (runtime initialized; failure landed on the tokenizer)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
