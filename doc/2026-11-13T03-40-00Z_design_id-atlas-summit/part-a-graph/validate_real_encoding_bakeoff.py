#!/usr/bin/env python3
"""Runner and independent validator for the graph encoding bake-off.

Slice A-GRAPH-9125-03, issue #10219. Oracle: research-matrix.

``--run-and-check`` runs the exact candidate-engine harness
(``cargo test -p id-store --test graph_encoding_bakeoff``) with
``BAKEOFF_RESULTS`` set, strictly parses the raw results it writes, and then
checks them, the committed ``real-encoding-bakeoff-results.json`` and the
report ``real-encoding-bakeoff.md``:

* the fixture is regenerated here from its description (splitmix64, a
  hand-written canonical-CBOR encoder and SHA-256), and its hash and size
  must match;
* every engine/workload cell is present for every run, with engine,
  version, schema, index and query-plan metadata;
* every query, page, write and round-trip output equals this file's own
  reference model of the corpus;
* every deterministic field (counts, digests, storage bytes) is identical
  across the repeated runs, and between the fresh run and the committed
  results;
* the report's tables are exactly what the committed results say, and the
  report names no universal winner.

``--check-committed`` does the same without running cargo. Nothing here
depends on the Rust code: the corpus, encodings and expected outputs are
recomputed from first principles. Standard library only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
PKG = REPO / "pkgs" / "id"
RESULTS = HERE / "real-encoding-bakeoff-results.json"
REPORT = HERE / "real-encoding-bakeoff.md"
STORE_CARGO = PKG / "crates" / "id-store" / "Cargo.toml"

SEED = 0x9125_0003
SUBJECTS = 1000
SUBJECT_QUERIES = 32
PO_QUERIES = 16
PAGE_SIZE = 128
NEW_TRIPLES = 200
DELETE_STRIDE = 37
CHUNK_BYTES = 4096
MIN_RUNS = 3
TAG_IRI, TAG_TEXT, TAG_BYTES, TAG_I64 = 1, 2, 3, 4
PROFILE = "raw-blake3-256"
MASK = (1 << 64) - 1

ENGINES = ["chunked_binary", "redb_tuple", "sqlite_jsonb", "sqlite_blob_rows"]
WORKLOADS = [
    "load",
    "storage",
    "round_trip",
    "query_subject",
    "query_predicate_object",
    "page_stream",
    "write",
]
TIMED = [
    "load",
    "round_trip",
    "query_subject",
    "query_predicate_object",
    "page_stream",
    "write",
]


class Invalid(Exception):
    """A check failed."""


def need(cond: bool, message: str) -> None:
    if not cond:
        raise Invalid(message)


# ---------------------------------------------------------------------------
# Independent fixture model.
# ---------------------------------------------------------------------------


class Rng:
    def __init__(self, state: int) -> None:
        self.state = state & MASK

    def next(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def digest(self) -> bytes:
        return b"".join(self.next().to_bytes(8, "big") for _ in range(4))


def cbor_head(major: int, n: int) -> bytes:
    if n < 24:
        return bytes([major << 5 | n])
    for extra, size in ((24, 1), (25, 2), (26, 4), (27, 8)):
        if n < 1 << (8 * size):
            return bytes([major << 5 | extra]) + n.to_bytes(size, "big")
    raise ValueError("integer too large")


def cbor(value: object) -> bytes:
    if isinstance(value, bool):
        raise TypeError("no booleans in this fixture")
    if isinstance(value, int):
        if value < 0:
            raise TypeError("no negative integers in this fixture")
        return cbor_head(0, value)
    if isinstance(value, bytes):
        return cbor_head(2, len(value)) + value
    if isinstance(value, str):
        raw = value.encode("utf-8")
        return cbor_head(3, len(raw)) + raw
    if isinstance(value, list):
        return cbor_head(4, len(value)) + b"".join(cbor(v) for v in value)
    raise TypeError(f"unsupported value {value!r}")


def typed(tag: int, data: bytes) -> bytes:
    return cbor([0, tag, data])


def iri(text: str) -> bytes:
    return typed(TAG_IRI, text.encode())


def user(i: int) -> bytes:
    return iri(f"urn:user:{i}")


def triple(s: bytes, p: bytes, o: bytes) -> bytes:
    # A canonical three-item array: the head 0x83, then the three terms.
    return cbor_head(4, 3) + s + p + o


def corpus() -> dict[bytes, tuple[bytes, bytes, bytes]]:
    rng = Rng(SEED)
    out: dict[bytes, tuple[bytes, bytes, bytes]] = {}

    def add(s: bytes, p: str, o: bytes) -> None:
        pb = iri(p)
        out[triple(s, pb, o)] = (s, pb, o)

    for i in range(SUBJECTS):
        s = user(i)
        add(s, "name", typed(TAG_TEXT, f"User {i}".encode()))
        add(s, "score", typed(TAG_I64, rng.next().to_bytes(8, "big")))
        for _ in range(rng.next() % 6):
            add(s, "follows", user(rng.next() % SUBJECTS))
        if rng.next() % 3 == 0:
            add(s, "avatar", cbor([1, PROFILE, rng.digest()]))
        if rng.next() % 4 == 0:
            add(s, "cites", cbor([2, PROFILE, rng.digest()]))
        length = rng.next() % 38
        blob = bytes([0x00, 0xFF]) + bytes(rng.next() & 0xFF for _ in range(length))
        add(s, "blob", typed(TAG_BYTES, blob))
    return dict(sorted(out.items()))


def set_digest(items) -> str:
    h = hashlib.sha256()
    for item in sorted(set(items)):
        h.update(len(item).to_bytes(4, "big"))
        h.update(item)
    return h.hexdigest()


class Model:
    """Expected outputs for every workload, computed from the fixture."""

    def __init__(self) -> None:
        self.triples = corpus()
        rng = Rng(SEED ^ 0x5157)
        self.subjects = [user(rng.next() % SUBJECTS) for _ in range(SUBJECT_QUERIES)]
        rng = Rng(SEED ^ 0x504F)
        self.po = [
            (iri("follows"), user(rng.next() % SUBJECTS)) for _ in range(PO_QUERIES)
        ]
        new = {}
        for j in range(NEW_TRIPLES):
            s, p = iri(f"urn:new:{j}"), iri("name")
            o = typed(TAG_TEXT, f"New {j}".encode())
            new[triple(s, p, o)] = (s, p, o)
        keys = list(self.triples)
        deletes = keys[::DELETE_STRIDE]
        final = dict(self.triples)
        final.update(new)
        for k in deletes:
            final.pop(k, None)
        self.n_deletes = len(deletes)
        self.final = final

    def fixture(self) -> dict:
        return {
            "generator": "splitmix64/v1",
            "seed": SEED,
            "subjects": SUBJECTS,
            "triples": len(self.triples),
            "sha256": set_digest(self.triples),
            "subject_queries": SUBJECT_QUERIES,
            "predicate_object_queries": PO_QUERIES,
            "new_triples": NEW_TRIPLES,
            "delete_stride": DELETE_STRIDE,
            "chunk_bytes": CHUNK_BYTES,
            "type_tags": {
                "iri": TAG_IRI,
                "text": TAG_TEXT,
                "bytes": TAG_BYTES,
                "i64_be": TAG_I64,
            },
        }

    def subject_rows(self) -> list[dict]:
        rows = []
        for s in self.subjects:
            hits = [k for k, (ts, _, _) in self.triples.items() if ts == s]
            rows.append(
                {"subject": s.hex(), "count": len(hits), "sha256": set_digest(hits)}
            )
        return rows

    def po_rows(self) -> list[dict]:
        rows = []
        for p, o in self.po:
            hits = [ts for (ts, tp, to) in self.triples.values() if tp == p and to == o]
            rows.append(
                {
                    "predicate": p.hex(),
                    "object": o.hex(),
                    "count": len(hits),
                    "sha256": set_digest(hits),
                }
            )
        return rows


# ---------------------------------------------------------------------------
# Strict parsing.
# ---------------------------------------------------------------------------


def _pairs(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise Invalid(f"duplicate JSON key {key!r}")
        out[key] = value
    return out


def _no_float(text):
    raise Invalid(f"unexpected non-integer number {text}")


def load_json(path: Path) -> dict:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as err:
        raise Invalid(f"cannot read {path}: {err}") from err
    try:
        value = json.loads(
            text,
            object_pairs_hook=_pairs,
            parse_float=_no_float,
            parse_constant=_no_float,
        )
    except json.JSONDecodeError as err:
        raise Invalid(f"{path.name}: not JSON: {err}") from err
    need(isinstance(value, dict), f"{path.name}: top level must be an object")
    return value


def keys(obj: object, expected: set[str], where: str) -> dict:
    need(isinstance(obj, dict), f"{where}: must be an object")
    got = set(obj)
    need(got == expected, f"{where}: keys {sorted(got)} != {sorted(expected)}")
    return obj


def count(value: object, where: str, minimum: int = 0) -> int:
    need(
        isinstance(value, int) and not isinstance(value, bool) and value >= minimum,
        f"{where}: must be an integer >= {minimum}, got {value!r}",
    )
    return value


def redb_pin() -> str:
    text = STORE_CARGO.read_text(encoding="utf-8")
    match = re.search(r'^redb\s*=\s*"=([0-9.]+)"', text, re.MULTILINE)
    need(match is not None, "id-store Cargo.toml has no exact redb pin")
    return match.group(1)


# ---------------------------------------------------------------------------
# Checks.
# ---------------------------------------------------------------------------

ENGINE_KEYS = {
    "name",
    "engine",
    "engine_version",
    "schema",
    "indexes",
    "query_plans",
    "durability",
}
PLAN_MUST = {
    "sqlite_jsonb": {"subject": "triples_spo", "predicate_object": "triples_po"},
    "sqlite_blob_rows": {"subject": "USING", "predicate_object": "triples_pos"},
}


def check_engines(engines: object) -> None:
    need(
        isinstance(engines, list) and len(engines) == len(ENGINES),
        "engines: one entry per engine",
    )
    sqlite_versions = set()
    for meta, name in zip(engines, ENGINES):
        extra = {"select_all"} if name == "sqlite_blob_rows" else set()
        keys(meta, ENGINE_KEYS | extra, f"engines[{name}]")
        need(meta["name"] == name, f"engines: expected {name}, got {meta['name']!r}")
        for field in ("engine", "engine_version", "schema", "durability"):
            need(
                isinstance(meta[field], str) and meta[field],
                f"engines[{name}].{field}: empty",
            )
        need(
            isinstance(meta["indexes"], list)
            and meta["indexes"]
            and all(isinstance(i, str) and i for i in meta["indexes"]),
            f"engines[{name}].indexes: must list the indexes",
        )
        plans = keys(
            meta["query_plans"],
            {"subject", "predicate_object"},
            f"engines[{name}].query_plans",
        )
        for kind, plan in plans.items():
            need(isinstance(plan, str) and plan, f"engines[{name}]: empty {kind} plan")
            must = PLAN_MUST.get(name, {}).get(kind)
            if must:
                need(
                    must in plan,
                    f"engines[{name}]: {kind} plan {plan!r} does not use {must}",
                )
                need(
                    not re.search(r"\bSCAN triples\b(?! USING)", plan),
                    f"engines[{name}]: {kind} plan scans the whole table: {plan!r}",
                )
        if name == "redb_tuple":
            need(meta["engine"] == "redb", "redb_tuple: engine must be redb")
            pin = redb_pin()
            need(
                meta["engine_version"] == pin,
                f"redb_tuple: version {meta['engine_version']} != pin {pin}",
            )
        if name.startswith("sqlite_"):
            need(
                re.fullmatch(r"3\.\d+\.\d+", meta["engine_version"]) is not None,
                f"{name}: SQLite version {meta['engine_version']!r}",
            )
            need("CREATE TABLE" in meta["schema"], f"{name}: schema must be the DDL")
            sqlite_versions.add(meta["engine_version"])
    need(
        len(sqlite_versions) == 1, "both SQLite engines must report one SQLite version"
    )


def deterministic(cell: dict) -> dict:
    """A cell without its timings."""
    out = {}
    for name, value in cell.items():
        if isinstance(value, dict):
            out[name] = {k: v for k, v in value.items() if k != "micros"}
        elif name != "run":
            out[name] = value
    return out


def check_cell(cell: object, model: Model, expected: dict, where: str) -> None:
    keys(cell, {"engine", "run"} | set(WORKLOADS), where)
    n = len(model.triples)
    keys(cell["load"], {"triples", "micros"}, f"{where}.load")
    need(
        cell["load"]["triples"] == n,
        f"{where}.load: triples {cell['load']['triples']} != {n}",
    )
    keys(cell["storage"], {"file_bytes", "compacted_bytes"}, f"{where}.storage")
    for field in ("file_bytes", "compacted_bytes"):
        count(cell["storage"][field], f"{where}.storage.{field}", 1)

    rt = keys(
        cell["round_trip"], {"checked", "mismatches", "micros"}, f"{where}.round_trip"
    )
    need(rt["checked"] == n and rt["mismatches"] == 0, f"{where}.round_trip: {rt}")

    qs = keys(cell["query_subject"], {"queries", "micros"}, f"{where}.query_subject")
    need(
        qs["queries"] == expected["subject"],
        f"{where}.query_subject: outputs differ from the reference model",
    )
    qp = keys(
        cell["query_predicate_object"],
        {"queries", "micros"},
        f"{where}.query_predicate_object",
    )
    need(
        qp["queries"] == expected["po"],
        f"{where}.query_predicate_object: outputs differ from the reference model",
    )

    page = keys(
        cell["page_stream"],
        {"page_size", "pages", "count", "duplicates", "sha256", "micros"},
        f"{where}.page_stream",
    )
    want_pages = -(-n // PAGE_SIZE)
    need(
        page["page_size"] == PAGE_SIZE
        and page["pages"] == want_pages
        and page["count"] == n
        and page["duplicates"] == 0
        and page["sha256"] == expected["all"],
        f"{where}.page_stream: {page}",
    )

    write = keys(
        cell["write"],
        {
            "inserts",
            "deletes",
            "transactions",
            "micros",
            "final_count",
            "final_sha256",
            "file_bytes_after",
        },
        f"{where}.write",
    )
    need(
        write["inserts"] == NEW_TRIPLES
        and write["deletes"] == model.n_deletes
        and write["transactions"] == NEW_TRIPLES + model.n_deletes
        and write["final_count"] == len(model.final)
        and write["final_sha256"] == expected["final"],
        f"{where}.write: {write}",
    )
    count(write["file_bytes_after"], f"{where}.write.file_bytes_after", 1)
    for workload in TIMED:
        count(cell[workload]["micros"], f"{where}.{workload}.micros", 1)


def check_results(results: dict, model: Model, label: str) -> None:
    keys(
        results,
        {
            "schema",
            "issue",
            "slice",
            "build",
            "fixture",
            "engines",
            "workloads",
            "runs",
            "cells",
            "limits",
        },
        label,
    )
    need(results["schema"] == "id-graph-encoding-bakeoff/v1", f"{label}: schema")
    need(
        results["issue"] == 10219 and results["slice"] == "A-GRAPH-9125-03",
        f"{label}: issue/slice",
    )
    build = keys(results["build"], {"profile"}, f"{label}.build")
    need(build["profile"] in ("debug", "release"), f"{label}: build profile")
    need(
        results["fixture"] == model.fixture(),
        f"{label}: fixture does not match the regenerated corpus",
    )
    need(results["workloads"] == WORKLOADS, f"{label}: workloads")
    check_engines(results["engines"])
    runs = count(results["runs"], f"{label}.runs", MIN_RUNS)
    limits = results["limits"]
    need(
        isinstance(limits, list)
        and len(limits) >= 4
        and all(isinstance(x, str) and x for x in limits),
        f"{label}: limits must list at least four limits",
    )

    expected = {
        "subject": model.subject_rows(),
        "po": model.po_rows(),
        "all": set_digest(model.triples),
        "final": set_digest(model.final),
    }
    cells = results["cells"]
    need(
        isinstance(cells, list) and len(cells) == runs * len(ENGINES),
        f"{label}: matrix size",
    )
    seen = {}
    for i, cell in enumerate(cells):
        where = f"{label}.cells[{i}]"
        need(isinstance(cell, dict), f"{where}: must be an object")
        engine, run = cell.get("engine"), cell.get("run")
        need(engine in ENGINES, f"{where}: engine {engine!r}")
        count(run, f"{where}.run")
        need(run < runs, f"{where}: run {run} out of range")
        need((engine, run) not in seen, f"{where}: duplicate cell {engine}/{run}")
        check_cell(cell, model, expected, f"{where}[{engine}/{run}]")
        seen[(engine, run)] = cell
    need(len(seen) == runs * len(ENGINES), f"{label}: missing cells")
    for engine in ENGINES:
        first = deterministic(seen[(engine, 0)])
        for run in range(1, runs):
            need(
                deterministic(seen[(engine, run)]) == first,
                f"{label}: {engine} run {run} is not repeatable (deterministic fields differ from run 0)",
            )


def comparable(results: dict) -> dict:
    """Everything that must match between two runs of the same pinned harness."""
    by = {c["engine"]: deterministic(c) for c in results["cells"] if c["run"] == 0}
    return {
        "fixture": results["fixture"],
        "workloads": results["workloads"],
        "engines": results["engines"],
        "cells": by,
    }


# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------

COLUMNS = [
    ("load µs", "load", "micros"),
    ("subject µs", "query_subject", "micros"),
    ("p/o µs", "query_predicate_object", "micros"),
    ("page µs", "page_stream", "micros"),
    ("write µs", "write", "micros"),
    ("bytes", "storage", "file_bytes"),
    ("compacted", "storage", "compacted_bytes"),
    ("bytes after writes", "write", "file_bytes_after"),
]


def table_values(results: dict) -> dict[str, list[int]]:
    rows = {}
    for engine in ENGINES:
        cells = [c for c in results["cells"] if c["engine"] == engine]
        row = []
        for _, workload, field in COLUMNS:
            values = [c[workload][field] for c in cells]
            row.append(
                int(statistics.median(values)) if field == "micros" else values[0]
            )
        rows[engine] = row
    return rows


def render_table(results: dict) -> str:
    rows = table_values(results)
    out = ["| engine | " + " | ".join(name for name, _, _ in COLUMNS) + " |"]
    out.append("|---|" + "---:|" * len(COLUMNS))
    for engine in ENGINES:
        out.append(f"| `{engine}` | " + " | ".join(str(v) for v in rows[engine]) + " |")
    return "\n".join(out)


def render_leaders(results: dict) -> str:
    rows = table_values(results)
    out = []
    for i, (name, _, _) in enumerate(COLUMNS):
        best = min(rows[e][i] for e in ENGINES)
        leaders = [e for e in ENGINES if rows[e][i] == best]
        out.append(f"- {name}: " + ", ".join(f"`{e}`" for e in leaders))
    return "\n".join(out)


def block(text: str, name: str) -> str:
    match = re.search(
        rf"<!-- bakeoff-{name}:begin -->\n(.*?)\n<!-- bakeoff-{name}:end -->",
        text,
        re.DOTALL,
    )
    need(match is not None, f"report: missing bakeoff-{name} block")
    return match.group(1)


def table_cells(markdown: str) -> list[list[str]]:
    """The cells of a Markdown table, row by row, without its layout.

    The repository's formatter pads cells, widens the rule row and adds
    blank lines around a block. None of that changes what the table says,
    so the comparison is on cell text: the rule row is dropped and each
    cell is stripped. A row that is not a table row is kept as one cell,
    so stray text in the block still makes the comparison fail.
    """
    rows: list[list[str]] = []
    for line in markdown.splitlines():
        line = line.strip()
        if not line:
            continue
        if not (line.startswith("|") and line.endswith("|") and len(line) > 1):
            rows.append([line])
            continue
        cells = [cell.strip() for cell in line[1:-1].split("|")]
        if all(re.fullmatch(r":?-+:?", cell) for cell in cells):
            continue
        rows.append(cells)
    return rows


def list_items(markdown: str) -> list[str]:
    """The non-blank lines of a block, each stripped."""
    return [line.strip() for line in markdown.splitlines() if line.strip()]


def check_report(results: dict) -> None:
    text = REPORT.read_text(encoding="utf-8")
    need(
        table_cells(block(text, "table")) == table_cells(render_table(results)),
        "report: table differs from the committed results",
    )
    need(
        list_items(block(text, "leaders")) == list_items(render_leaders(results)),
        "report: leaders differ from the committed results",
    )
    need(results["fixture"]["sha256"] in text, "report: must name the fixture SHA-256")
    need(
        f"{results['fixture']['triples']} triples" in text,
        "report: must state the triple count",
    )
    for meta in results["engines"]:
        need(
            meta["engine_version"] in text,
            f"report: must name {meta['name']} version {meta['engine_version']}",
        )
    need(
        "No universal winner is asserted." in text,
        "report: must say that no universal winner is asserted",
    )
    lowered = text.lower()
    for phrase in (
        "overall winner",
        "best overall",
        "clear winner",
        "universal winner is",
    ):
        need(
            phrase not in lowered.replace("no universal winner is asserted", ""),
            f"report: claims {phrase!r}",
        )
    for limit in results["limits"]:
        need(limit in text, f"report: must list the limit {limit!r}")


# ---------------------------------------------------------------------------
# Runner.
# ---------------------------------------------------------------------------

CARGO = [
    "cargo",
    "test",
    "-q",
    "-p",
    "id-store",
    "--test",
    "graph_encoding_bakeoff",
    "--",
    "--exact",
    "graph_encoding_bakeoff",
]


def run_harness(out: Path) -> None:
    env = dict(os.environ, BAKEOFF_RESULTS=str(out))
    proc = subprocess.run(
        CARGO, cwd=PKG, env=env, capture_output=True, text=True, check=False
    )
    tail = (proc.stdout + proc.stderr)[-4000:]
    need(proc.returncode == 0, f"harness failed with exit {proc.returncode}:\n{tail}")
    need(
        re.search(r"^test result: ok\. 1 passed; 0 failed;", proc.stdout, re.MULTILINE)
        is not None,
        f"harness did not report exactly one passing test:\n{tail}",
    )
    need(out.is_file(), "harness wrote no results")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0], allow_abbrev=False
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--run-and-check",
        action="store_true",
        help="run the harness, then check everything",
    )
    mode.add_argument(
        "--check-committed",
        action="store_true",
        help="check the committed results and report only",
    )
    mode.add_argument(
        "--render-report-blocks",
        action="store_true",
        help="print the report's table and leaders blocks",
    )
    args = parser.parse_args(argv)

    model = Model()
    try:
        committed = load_json(RESULTS)
        if args.render_report_blocks:
            print(render_table(committed))
            print()
            print(render_leaders(committed))
            return 0
        check_results(committed, model, RESULTS.name)
        check_report(committed)
        if args.run_and_check:
            with tempfile.TemporaryDirectory() as tmp:
                fresh_path = Path(tmp) / "results.json"
                run_harness(fresh_path)
                fresh = load_json(fresh_path)
            check_results(fresh, model, "fresh run")
            need(
                comparable(fresh) == comparable(committed),
                "fresh run: deterministic fields differ from the committed results",
            )
    except Invalid as err:
        print(f"FAIL: {err}", file=sys.stderr)
        return 1
    cells = committed["runs"] * len(ENGINES) * len(WORKLOADS)
    what = (
        "fresh run and committed results" if args.run_and_check else "committed results"
    )
    print(
        f"OK: {what}: {len(model.triples)} triples, fixture {model.fixture()['sha256'][:16]}..., "
        f"{len(ENGINES)} engines x {len(WORKLOADS)} workloads x {committed['runs']} runs = {cells} cells"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
