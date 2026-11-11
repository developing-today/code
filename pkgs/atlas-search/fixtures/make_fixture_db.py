#!/usr/bin/env python3
"""Build the atlas fixture DB (fixture_corpus.sql + the toy vectors
of fixture_spec.json) as a SQLite file.

The Rust parity tests build the same DB in-process with rusqlite
(crates/atlas-core/tests/parity.rs); this stdlib-only twin exists
for the nix derivation's installCheck smoke
(pkgs/atlas-search/default.nix) and for manual packaged-binary
verification: F32 little-endian blobs keyed by chunk id, exactly as
the Rust builder inserts them.

Usage: make_fixture_db.py [OUT.db]   (default: ./fixture.db)
Also writes OUT.qvec.json (the fixture query vector, for
ATLAS_SEARCH_QVEC_FILE) next to the DB.
"""

import json
import sqlite3
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("fixture.db")
    sql = (HERE / "fixture_corpus.sql").read_text()
    spec = json.loads((HERE / "fixture_spec.json").read_text())
    if out.exists():
        out.unlink()
    con = sqlite3.connect(out)
    con.executescript(sql)
    for cid, vec in spec["vectors"].items():
        blob = struct.pack("<%df" % len(vec), *vec)
        con.execute(
            "INSERT INTO vec_snowflake_m_v2_768_f32 (chunk_id, vec) VALUES (?, ?)",
            (int(cid), blob),
        )
    con.commit()
    con.close()
    qvec = Path(str(out) + ".qvec.json")
    qvec.write_text(json.dumps(spec["qvec_raw"]))
    print(f"fixture db: {out}")
    print(f"query vector: {qvec}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
