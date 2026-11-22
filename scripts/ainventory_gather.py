#!/usr/bin/env python3
"""Gatherer for the append-only AA event log (issue #1248).

Pulls Artificial Analysis model data (the /models page embeds a full
structured JSON payload per variant) and models.dev/api.json, diffs
both against the event log's current views, and emits the WOULD-INSERT
event diff: new observations only, plus NEW-MODEL DETECTION (a pulled
model with no events and no identity match anywhere in ainventory).

Phase 1 was dry-run only. Phase 2 (task 16876, #9999): with --live the
diff becomes real inserts — one ainventory.ingest_batches row per
non-empty run, measurement_events / price_events rows for the diff
only (insert-only; the append-only log is never updated in place).
A live run whose diff is empty writes nothing at all. Without --live
the script is the original dry run: GETs only, zero writes.

Usage: gather_events.py [--live] [--out DIR]   (default out: ./dry_runs)
Auth: fleet-platform Turso credential via fleet secrets; never printed.
Canonicalized from task 16876 standalone gatherer. Live writes are one transaction.
"""

import argparse
import uuid
import json
from fleet_turso import Turso
import os
import re
import subprocess
import sys
from datetime import date, timedelta, datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36"
AA_MODELS_URL = "https://artificialanalysis.ai/models"
MODELSDEV_URL = "https://models.dev/api.json"
II_BENCHMARK = "AA Intelligence Index"
II_VERSION = "4.3.2"  # AA's current index version, as labelled in our data
NEW_MODEL_WINDOW_DAYS = 60

# ainventory.models provider label -> models.dev provider id
PROVIDER_MAP = {
    "OpenAI": "openai",
    "Anthropic": "anthropic",
    "DeepSeek": "deepseek",
    "Google": "google",
    "Meta": "meta",
    "Xiaomi": "xiaomi",
    "MiniMax": "minimax",
    "Mistral": "mistral",
    "Z AI": "zai",
    "Alibaba": "alibaba",
    "Kimi": "moonshotai",
    "SpaceXAI": "xai",
    "Inception": "inception",
    "NVIDIA": "nvidia",
    "Upstage": "upstage",
}


def fetch(url):
    out = subprocess.run(
        ["curl", "-sS", "-m", "120", "-A", UA, url],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return out


# Oracle: PG DISTINCT ON ordering from ainventory v_current_* views.
CURRENT_MEASUREMENTS = """SELECT * FROM (
 SELECT *, row_number() OVER (
 PARTITION BY coalesce(model_variant_key,model_name), coalesce(effort,''), benchmark, coalesce(benchmark_version,'')
 ORDER BY source_date DESC NULLS LAST, ingested_at DESC, id DESC) AS _rank
 FROM ainventory_measurement_events) WHERE _rank=1"""
CURRENT_PRICES = """SELECT * FROM (
 SELECT *, row_number() OVER (
 PARTITION BY model_name, coalesce(provider,''), context_min_tokens, context_max_tokens, window_kind
 ORDER BY effective_date DESC NULLS LAST, source_date DESC NULLS LAST, ingested_at DESC, id DESC) AS _rank
 FROM ainventory_price_events) WHERE _rank=1"""


def read_current(db):
    return (
        db.query("SELECT id,name,provider FROM ainventory_models"),
        db.query(
            "SELECT model_name,model_variant_key,effort,benchmark,benchmark_version,value_num,ingested_at FROM ainventory_measurement_events ORDER BY ingested_at DESC,id DESC"
        ),
        db.query(CURRENT_MEASUREMENTS),
        db.query(CURRENT_PRICES),
    )


def insert_statement(table, rows):
    # Table names and fields come only from this program, never CLI/data URLs.
    if table not in {"ingest_batches", "measurement_events", "price_events"}:
        raise ValueError("unsupported event table")
    columns = list(rows[0])
    if not all(re.fullmatch(r"[a-z_][a-z_0-9]*", c) for c in columns):
        raise ValueError("invalid event column")
    if any(list(row) != columns for row in rows):
        raise ValueError("mixed event columns")
    values = ",".join("(" + ",".join("?" for _ in columns) + ")" for _ in rows)
    return (
        "INSERT INTO ainventory_"
        + table
        + " ("
        + ",".join(columns)
        + ") VALUES "
        + values,
        [value for row in rows for value in row.values()],
    )


def persist_events(db, batch, measurements, prices):
    """No retries after uncertain network outcomes; UUID appears in local intent first."""
    statements = [insert_statement("ingest_batches", [batch])]
    for table, rows in [("measurement_events", measurements), ("price_events", prices)]:
        for i in range(0, len(rows), 100):
            statements.append(insert_statement(table, rows[i : i + 100]))
    db.batch(statements, transaction=True)


def num_or_none(v):
    """Coerce an AA field to a scalar number or None.

    AA returns output_tokens_per_task as a per-phase breakdown object
    ({reasoning, answer, output}) for some variants; the events table
    column is numeric. The breakdown's own "output" figure is the
    total; the full object stays preserved in the row's raw payload.
    """
    if isinstance(v, dict):
        total = v.get("output")
        if isinstance(total, (int, float)):
            return total
        vals = [x for x in v.values() if isinstance(x, (int, float))]
        return sum(vals) if vals else None
    if isinstance(v, (int, float)):
        return v
    return None


def norm(s):
    """Identity normalization: lowercase alphanumerics only."""
    return re.sub(r"[^a-z0-9]", "", (s or "").lower())


# Known cross-source spellings from the AA coverage catalogue. These are
# deliberately narrow: a miss is safer than merging two different models.
IDENTITY_ALIASES = {
    "deepseekv4pro0813": "deepseekv4pro",
    "deepseekv4flash": "deepseekv41flash",
    "musewonder10": "musewonder",
    "qwen38max0902": "qwen38max",
    "claudehaiku45": "claude45haiku",
}


def identity_keys(value):
    """Return conservative equivalent keys for a source model identifier."""
    value = value or ""
    parts = [value]
    if "|" in value:
        parts.append(value.split("|", 1)[1])
    if "/" in value:
        parts.append(value.rsplit("/", 1)[1])
    keys = set()
    for part in parts:
        key = norm(part)
        if key:
            keys.add(IDENTITY_ALIASES.get(key, key))
    return keys


def build_identity_index(models, events):
    """Map unambiguous known spellings to the canonical inventory name."""
    candidates = {}
    for row in models:
        canonical = row["name"]
        for key in identity_keys(canonical):
            candidates.setdefault(key, set()).add(canonical)
    for row in events:
        canonical = row.get("model_name")
        if canonical:
            for key in identity_keys(canonical):
                candidates.setdefault(key, set()).add(canonical)
    return {
        key: next(iter(names)) for key, names in candidates.items() if len(names) == 1
    }


def resolve_identity(values, index):
    """Resolve source spellings only when they point to one known identity."""
    matches = {
        index[key] for value in values for key in identity_keys(value) if key in index
    }
    return next(iter(matches)) if len(matches) == 1 else None


def model_id_for(canonical, models):
    """Return the dimension row id for a resolved canonical display name."""
    return next((m.get("id") for m in models if m["name"] == canonical), None)


def canonical_key(value):
    """Use the display-name segment as the stable comparison key."""
    parts = identity_keys(value)
    if "|" in (value or ""):
        key = norm(value.split("|", 1)[1])
        return IDENTITY_ALIASES.get(key, key)
    if "/" in (value or ""):
        key = norm(value.rsplit("/", 1)[1])
        return IDENTITY_ALIASES.get(key, key)
    return next(iter(parts), "")


# ------------------------------------------------------------ AA parsing
def parse_aa_models(html):
    """Extract the embedded per-variant JSON objects from the AA page."""
    u = html.replace('\\"', '"').replace("\\\\", "\\")
    starts = [m.start() for m in re.finditer(r'\{"id":"[0-9a-f-]{36}","slug":"', u)]

    def extract(i):
        depth, instr, esc = 0, False, False
        for j in range(i, len(u)):
            c = u[j]
            if instr:
                if esc:
                    esc = False
                elif c == "\\":
                    esc = True
                elif c == '"':
                    instr = False
            else:
                if c == '"':
                    instr = True
                elif c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                    if depth == 0:
                        return u[i : j + 1]
        return None

    variants = []
    for s in starts:
        raw = extract(s)
        if not raw:
            continue
        try:
            o = json.loads(raw)
        except Exception:
            continue
        if not o.get("release") or o.get("intelligenceIndex") is None:
            continue  # creator/aggregate entries, not model variants
        perf = o.get("performanceByPromptType") or {}
        speed = None
        for k in ("short", "medium", "long"):
            if isinstance(perf.get(k), dict) and perf[k].get("medianOutputSpeed"):
                speed = perf[k]["medianOutputSpeed"]
                break
        ttft = o.get("timeToFirstAnswerToken") or {}
        cpt = o.get("intelligenceIndexCostPerTask")
        if isinstance(cpt, dict):  # current AA payload: {cost: {total, ...}}
            cpt = (cpt.get("cost") or {}).get("total")
        variants.append(
            {
                "release_slug": o["release"]["slug"],
                "release_name": o["release"]["name"],
                "creator_slug": (o.get("creator") or {}).get("slug"),
                "variant_slug": o.get("slug"),
                "effort": (o.get("effort") or {}).get("slug"),
                "release_date": o.get("releaseDate"),
                "intelligence_index": o.get("intelligenceIndex"),
                "cost_per_task_usd": cpt,
                "output_tokens_per_task": o.get("intelligenceIndexOutputTokensPerTask"),
                "ttft_seconds": ttft.get("total") if isinstance(ttft, dict) else None,
                "output_tps_short": speed,
                "price_input_per_1m": o.get("price1mInputTokens"),
                "price_output_per_1m": o.get("price1mOutputTokens"),
                "price_cached_input_per_1m": o.get("cacheHitPrice"),
                "source_ref": f"https://artificialanalysis.ai/models/{o['release']['slug']}",
            }
        )
    return variants


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--out", default=os.path.join(HERE, "dry_runs"))
    args = parser.parse_args()  # Parse before filesystem/credential/network effects.
    out_dir = args.out
    os.makedirs(out_dir, exist_ok=True)
    today = date.today().isoformat()
    db = Turso()
    models, events, current, prices = read_current(db)

    # ---- known identities. Source spellings resolve to the existing
    # inventory display name; ambiguous aliases intentionally stay unmatched.
    identity_index = build_identity_index(models, events)

    # current II (current version) keyed by (identity, effort)
    cur_ii = {}
    for c in current:
        if c["benchmark"] == II_BENCHMARK and c["benchmark_version"] == II_VERSION:
            ident = canonical_key(
                (c.get("model_variant_key") or "").split("|")[1]
                if c.get("model_variant_key")
                else c["model_name"]
            )
            cur_ii[(ident, norm(c.get("effort")))] = c
    # events arrive newest-first; the first row per key is the log's
    # current value for that (identity, effort), and it overrides the
    # view's row for the same key.
    seen_latest = set()
    for e in events:
        if (
            e.get("benchmark") == II_BENCHMARK
            and e.get("benchmark_version") == II_VERSION
        ):
            ident = canonical_key(
                (e.get("model_variant_key") or "").split("|")[1]
                if e.get("model_variant_key")
                else e.get("model_name")
            )
            key = (ident, norm(e.get("effort")))
            if key not in seen_latest:
                seen_latest.add(key)
                cur_ii[key] = {"value_num": e.get("value_num")}
    cur_price = {}
    cur_bands = {}
    for p in prices:
        if p.get("window_kind") == "flat" and not p.get("provider"):
            key = canonical_key(p["model_name"])
            cur_price[key] = p
            cur_bands[(key, p.get("context_min_tokens") or 0)] = p

    # ---- AA pull + diff
    aa = parse_aa_models(fetch(AA_MODELS_URL))
    aa_meas, aa_price, new_models = [], [], []
    seen_releases = {}
    unchanged = 0
    for v in aa:
        canonical = resolve_identity(
            [v["release_name"], v["release_slug"]], identity_index
        )
        if canonical is None:
            seen_releases.setdefault(
                v["release_slug"],
                {
                    "model": v["release_name"],
                    "creator": v["creator_slug"],
                    "release_date": v["release_date"],
                    "source": "aa",
                    "source_ref": v["source_ref"],
                    "best_intelligence_index": v["intelligence_index"],
                },
            )
            prev = seen_releases[v["release_slug"]]
            if (v["intelligence_index"] or 0) > (prev["best_intelligence_index"] or 0):
                prev["best_intelligence_index"] = v["intelligence_index"]
            continue
        ident = canonical_key(canonical)
        if v["effort"] is None:
            continue  # release-level entry duplicates the featured variant
        cur = cur_ii.get((ident, norm(v["effort"])))
        if cur is None or abs((cur["value_num"] or 0) - v["intelligence_index"]) > 0.05:
            aa_meas.append(
                {
                    "model": canonical,
                    "model_id": model_id_for(canonical, models),
                    "effort": v["effort"],
                    "benchmark": II_BENCHMARK,
                    "benchmark_version": II_VERSION,
                    "intelligence_index": v["intelligence_index"],
                    "cost_per_task_usd": v["cost_per_task_usd"],
                    "output_tokens_per_task": v["output_tokens_per_task"],
                    "ttft_seconds": v["ttft_seconds"],
                    "output_tps_short": v["output_tps_short"],
                    "current_value": None if cur is None else cur["value_num"],
                    "source_kind": "aa",
                    "source_ref": v["source_ref"],
                    "source_date": today,
                }
            )
        else:
            unchanged += 1
        cp = cur_bands.get((ident, 0)) or cur_price.get(ident)
        if v["price_input_per_1m"] is not None and (
            cp is None
            or cp.get("input_usd_per_1m") != v["price_input_per_1m"]
            or cp.get("output_usd_per_1m") != v["price_output_per_1m"]
        ):
            aa_price.append(
                {
                    "model": canonical,
                    "model_id": model_id_for(canonical, models),
                    "input_usd_per_1m": v["price_input_per_1m"],
                    "output_usd_per_1m": v["price_output_per_1m"],
                    "cached_input_usd_per_1m": v["price_cached_input_per_1m"],
                    "source_kind": "aa",
                    "source_ref": v["source_ref"],
                    "source_date": today,
                }
            )
    new_models.extend(seen_releases.values())

    # ---- models.dev pull + diff (tracked providers only)
    md = json.loads(fetch(MODELSDEV_URL))
    md_price, md_new = [], []
    cutoff = (date.today() - timedelta(days=NEW_MODEL_WINDOW_DAYS)).isoformat()
    for our_label, md_provider in PROVIDER_MAP.items():
        prov = md.get(md_provider)
        if not prov:
            continue
        for mid, m in (prov.get("models") or {}).items():
            source_names = [f"{md_provider}/{mid}", mid, m.get("name")]
            canonical = resolve_identity(source_names, identity_index)
            cost = m.get("cost") or {}
            if canonical is not None:
                # models.dev api.json costs are already USD per 1M tokens.
                # Each context band is compared against the current
                # price for ITS band: the view returns one row per
                # band, and comparing the base price against the top
                # band re-inserted every tiered price on every run.
                keys = [canonical_key(canonical)]

                def band(ctx_min, _keys=keys):
                    for k in _keys:
                        row = cur_bands.get((k, ctx_min))
                        if row is not None:
                            return row
                    return None

                def differs(row, pin, pout):
                    return (
                        row is None
                        or row.get("input_usd_per_1m") != pin
                        or row.get("output_usd_per_1m") != pout
                    )

                pin, pout = cost.get("input"), cost.get("output")
                ctx_limit = (m.get("limit") or {}).get("context")
                # Context-band dimension: a context tier splits the
                # price into [0, size) and [size, limit] bands.
                tiers = [
                    t
                    for t in (cost.get("tiers") or [])
                    if (t.get("tier") or {}).get("type") == "context"
                ]
                base_max = tiers[0]["tier"]["size"] if tiers else ctx_limit
                if pin is not None and differs(band(0), pin, pout):
                    md_price.append(
                        {
                            "model": canonical,
                            "model_id": model_id_for(canonical, models),
                            "input_usd_per_1m": pin,
                            "output_usd_per_1m": pout,
                            "cached_input_usd_per_1m": cost.get("cache_read"),
                            "cache_write_usd_per_1m": cost.get("cache_write"),
                            "context_min_tokens": 0,
                            "context_max_tokens": base_max,
                            "source_kind": "modelsdev",
                            "source_ref": MODELSDEV_URL,
                            "source_date": today,
                        }
                    )
                for t in tiers:
                    tmin = t["tier"]["size"]
                    if differs(band(tmin), t.get("input"), t.get("output")):
                        md_price.append(
                            {
                                "model": canonical,
                                "model_id": model_id_for(canonical, models),
                                "input_usd_per_1m": t.get("input"),
                                "output_usd_per_1m": t.get("output"),
                                "cached_input_usd_per_1m": t.get("cache_read"),
                                "cache_write_usd_per_1m": t.get("cache_write"),
                                "context_min_tokens": tmin,
                                "context_max_tokens": ctx_limit,
                                "source_kind": "modelsdev",
                                "source_ref": MODELSDEV_URL,
                                "source_date": today,
                            }
                        )
            else:
                rd = m.get("release_date") or ""
                if rd >= cutoff:
                    md_new.append(
                        {
                            "model": m.get("name") or mid,
                            "creator": md_provider,
                            "release_date": rd,
                            "source": "modelsdev",
                            "source_ref": MODELSDEV_URL,
                        }
                    )
    # de-dupe models.dev new flags against AA flags by normalized name
    aa_new_norms = {norm(n["model"]) for n in new_models}
    md_new = [n for n in md_new if norm(n["model"]) not in aa_new_norms]
    new_models.extend(md_new)

    live = args.live
    batch_id = None
    inserted = {"measurement_events": 0, "price_events": 0}
    if live and (aa_meas or aa_price or md_price):
        batch_id = str(uuid.uuid4())
        timestamp = (
            datetime.now(timezone.utc)
            .isoformat(timespec="milliseconds")
            .replace("+00:00", "Z")
        )
        batch = {
            "id": batch_id,
            "ran_at": timestamp,
            "created_at": timestamp,
            "source_kind": "gatherer",
            "source_ref": "task 16876 / issue #9999 Phase 2 slice 1 live gatherer",
            "inserted_counts": {},
            "note": (
                "Live gatherer batch (insert-only). New models flagged: "
                f"{len(new_models)}: "
                + ", ".join(n["model"] for n in new_models)[:1500]
            ),
        }
        meas_rows = [
            {
                "model_name": r["model"],
                "model_id": r.get("model_id"),
                "effort": r["effort"],
                "benchmark": r["benchmark"],
                "benchmark_version": r["benchmark_version"],
                "metric": "intelligence_index",
                "value_num": r["intelligence_index"],
                "intelligence_index": r["intelligence_index"],
                "ttft_seconds": num_or_none(r["ttft_seconds"]),
                "output_tps": num_or_none(r["output_tps_short"]),
                "output_tokens_per_task": num_or_none(r["output_tokens_per_task"]),
                "cost_per_task_usd": num_or_none(r["cost_per_task_usd"]),
                "source_kind": r["source_kind"],
                "source_ref": r["source_ref"],
                "source_date": r["source_date"],
                "ingest_batch_id": batch_id,
                "raw": r,
            }
            for r in aa_meas
        ]
        price_rows = [
            {
                "model_name": r["model"],
                "model_id": r.get("model_id"),
                "provider": None,
                "input_usd_per_1m": r["input_usd_per_1m"],
                "output_usd_per_1m": r["output_usd_per_1m"],
                "cached_input_usd_per_1m": r["cached_input_usd_per_1m"],
                "cache_write_usd_per_1m": r.get("cache_write_usd_per_1m"),
                "context_min_tokens": r.get("context_min_tokens", 0),
                "context_max_tokens": r.get("context_max_tokens"),
                "window_kind": "flat",
                "source_kind": r["source_kind"],
                "source_ref": r["source_ref"],
                "source_date": r["source_date"],
                "ingest_batch_id": batch_id,
                "raw": r,
            }
            for r in aa_price + md_price
        ]
        inserted = {
            "measurement_events": len(meas_rows),
            "price_events": len(price_rows),
        }
        batch["inserted_counts"] = inserted
        # Private local intent permits exact UUID read-back after an uncertain response.
        intent_path = os.path.join(out_dir, "intent-" + batch_id + ".json")
        fd = os.open(intent_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump(
                {"batch": batch, "measurements": meas_rows, "prices": price_rows},
                f,
                allow_nan=False,
            )
        persist_events(db, batch, meas_rows, price_rows)

    diff = {
        "dry_run": not live,
        "ingest_batch_id": batch_id,
        "inserted": inserted,
        "generated_at": today,
        "counts": {
            "aa_variants_pulled": len(aa),
            "aa_measurement_would_insert": len(aa_meas),
            "aa_measurement_unchanged": unchanged,
            "aa_price_would_insert": len(aa_price),
            "modelsdev_price_would_insert": len(md_price),
            "new_models_flagged": len(new_models),
        },
        "aa_measurement_would_insert": aa_meas,
        "aa_price_would_insert": aa_price,
        "modelsdev_price_would_insert": md_price,
        "new_models_flagged": new_models,
    }
    out_path = os.path.join(out_dir, f"gather_diff_{today}.json")
    with open(out_path, "w") as f:
        json.dump(diff, f, indent=2)
    print(json.dumps(diff["counts"], indent=2))
    if live:
        print(
            f"\nLIVE — batch {batch_id}: inserted {inserted}. Full diff: {out_path}"
            if batch_id
            else f"\nLIVE — empty diff, zero writes (no batch row). Full diff: {out_path}"
        )
    else:
        print(f"\nDRY RUN — no rows inserted. Full diff: {out_path}")
    if new_models:
        print("\nNEW MODELS FLAGGED:")
        for n in new_models:
            print(
                f"  - {n['model']} ({n['creator']}, {n['release_date']}, "
                f"via {n['source']})"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
