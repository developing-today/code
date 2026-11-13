#!/usr/bin/env python3
"""D4 dominated-model detection prototype (stdlib only).

Pareto dominance on (quality up, dollars-per-completed-task down, speed up) per
tier, with speed used only in the speed-sensitive tiers (small, speed, scout) per
brief §7. Inputs: AA scores (AA v5 rankings from the pm-aggregator copy of the
ainventory source of record) and measured route_attempt rows (14 days, read-only).
Output: replacement proposals, never applied automatically.

Subcommands:
  extract   --db PATH --out PATH        read-only SQL over route_attempt -> snapshot JSON
  run       --aa PATH --measured PATH   build candidates, print frontier and proposals
  selftest                              brute-force oracle + PBT over random inputs

Run: python3 -I dominance.py selftest
     python3 -I dominance.py run --aa ~/.local/state/pm-aggregator/aa-last-good.json \
         --measured research/prototypes/d4-measured-14d.json
"""
import argparse
import collections
import datetime
import json
import random
import re
import sqlite3
import statistics
import sys

WINDOW_DAYS = 14
LIVE_MIN = 0.5    # a door below this conditional success is dead for proposals
MIN_N = 50        # measured rows with fewer attempts are not used
# Attempts a door could not serve for reasons outside the model: quota walls,
# dead credentials, unknown provider ids. Removed from the denominator of the
# conditional success rate; reported separately (brief §8: never burn walled doors).
WALL_TAGS = ("QuotaExceeded", "Authentication", "UnknownProvider")
SPEED_TIERS = ("small", "speed", "scout")

# Config pool members (.opencode/opencode.jsonc, as of 2026-11-13) with the effort
# each seat pins. Used by `run` to propose replacements for seated models only.
POOL_SEATS = {
    "workhorse-weighted": [("gpt-6-luna", "medium"), ("gpt-6-luna", "high"),
                           ("muse-spark-1.3-contributor", "high"),
                           ("gemini-4-hydrogen", "low"), ("claude-sonnet-5-5", "medium")],
    "workhorse-last-resort": [("claude-haiku-5-5", "high"), ("claude-fable-5-2", "low")],
    "speed": [("muse-spark-1.3-contributor", "xhigh"), ("deepseek-v4.1-flash", "high"),
              ("gemini-4-hydrogen", "high"), ("claude-haiku-5-5", "high")],
    "summit": [("claude-fable-5-2", "medium"), ("claude-opus-5-5", "high"),
               ("gemini-4-argon", "high"), ("gemini-4-hydrogen", "high"),
               ("muse-wonder-1.0-contributor", "xhigh"), ("gpt-6-1-sol", "xhigh")],
    "specialist": [("claude-fable-5-2", "low"), ("claude-sonnet-5-5", "high"),
                   ("gemini-4-argon", "high"), ("muse-spark-1.3-contributor", "xhigh"),
                   ("gpt-6-1-sol", "medium")],
}

# Config model id -> AA model key. Each entry is a mapping hypothesis; the
# `exact` flag says whether the AA row is the same SKU (False = nearest version).
AA_MAP = {
    "gpt-6-luna": ("openai", "gpt-6-luna", True),
    "gpt-6.1-terra": ("openai", "gpt-6-1-terra", True),
    "gpt-6.1-sol": ("openai", "gpt-6-1-sol", True),
    "deepseek-v4.1-flash": ("deepseek", "deepseek-v4.1-flash", True),
    "deepseek-v4-flash": ("deepseek", "deepseek-v4.1-flash", False),
    "deepseek-v4-1-flash-1017": ("deepseek", "deepseek-v4.1-flash-1017", True),
    "deepseek-v4-pro": ("deepseek", "deepseek-v4-pro-0813", False),
    "muse-spark-1.3-contributor": ("meta", "muse-spark-1.3", True),
    "muse-wonder-1.0-contributor": ("meta", "muse-wonder", False),
    "claude-haiku-5-5": ("anthropic", "claude-haiku-5-5", True),
    "claude-sonnet-5-5": ("anthropic", "claude-sonnet-5.5", True),
    "claude-opus-5-5": ("anthropic", "claude-opus-5.5", True),
    "claude-fable-5-2": ("anthropic", "claude-fable-5.1", False),
    "gemini-4-hydrogen": ("google", "gemini-4-hydrogen", True),
    "gemini-4-argon": ("google", "gemini-4-argon", True),
    "gemini-3.8-flash": ("google", "gemini-3.8-flash", True),
    "mistral-small-latest": ("mistral", "mistral-small-4", False),
    "beam-free": ("reflection", "reflection-beam", False),
    "mimo-v2.6-flash": ("xiaomi", "mimo-v2.6-flash", True),
    "zai-glm-5-3": ("z-ai", "glm-5.3", True),
    "zai-glm-5-3-flash": ("z-ai", "glm-5.3-flash", True),
}


def norm(name):
    return re.sub(r"[^a-z0-9]", "", name.lower().replace(".", "-"))


def tier_of(cost):
    if cost is None:
        return "unpriced"
    if cost <= 0.5:
        return "cheap"
    if cost <= 3.0:
        return "standard"
    return "frontier"


def dominates(a, b, use_speed):
    """a Pareto-dominates b: at least as good on every axis, strictly better on one."""
    if a["q"] < b["q"] or a["c"] > b["c"]:
        return False
    if use_speed and a["s"] < b["s"]:
        return False
    strict = a["q"] > b["q"] or a["c"] < b["c"]
    if use_speed:
        strict = strict or a["s"] > b["s"]
    return strict


def frontier_brute(points, use_speed):
    """Oracle: keep every point that no other point dominates (O(n^2))."""
    return [p for p in points
            if not any(dominates(o, p, use_speed) for o in points if o is not p)]


def frontier_fast(points, use_speed):
    """Skyline: sort so every possible dominator precedes what it dominates,
    then test each point only against the kept set."""
    if use_speed:
        order = sorted(points, key=lambda p: (-p["q"], p["c"], -p["s"]))
    else:
        order = sorted(points, key=lambda p: (-p["q"], p["c"]))
    kept = []
    for p in order:
        if not any(dominates(k, p, use_speed) for k in kept):
            kept.append(p)
    return kept


def build_candidates(aa_rows, measured_rows):
    """One candidate per (model, effort). Quality from AA; cost per completed task =
    AA cost / conditional success on the best live door; speed = measured median
    tokens/s on that door when there is one, else AA speed."""
    by_key = {}
    for r in aa_rows:
        by_key[(r["model_key"], r["effort"])] = r
    best = {}
    for m in measured_rows:
        if m["n"] < MIN_N or m["cond_rate"] is None or m["cond_rate"] < LIVE_MIN:
            continue
        k = (m["model_key"], m["variant"])
        if k not in best or m["cond_rate"] > best[k]["cond_rate"]:
            best[k] = m
    cands = []
    for (model, effort), aa in by_key.items():
        if aa.get("quality") is None:
            continue
        door = best.get((model, effort))
        cost = aa.get("cost")
        if cost is None:
            continue
        if door is not None:
            cost = cost / max(door["cond_rate"], 0.01)
        speed = door["tps_med"] if door and door.get("tps_med") else aa.get("speed")
        if speed is None:
            speed = 0.0
        cands.append({"model": model, "effort": effort, "q": aa["quality"],
                      "c": cost, "s": speed, "tier": tier_of(cost),
                      "door": door["door"] if door else None,
                      "cond_rate": door["cond_rate"] if door else None,
                      "source": "measured" if door else "aa-only"})
    return cands


def load_aa(path):
    d = json.load(open(path))["section"]["rankings"]
    rows = []
    seen = set()
    for r in d:
        if r.get("benchmarkVersionKey") != "AA|v5":
            continue
        if r.get("qualStatus") == "new release / awaiting source-backed assessment":
            continue
        creator_model = r["modelVariantKey"].split("|", 1)
        if len(creator_model) != 2:
            continue
        creator, model = creator_model
        model_key = norm(model.split("|")[0])
        effort = r.get("effort") or "reasoning"
        key = (model_key, effort, creator)
        if key in seen:
            continue
        seen.add(key)
        rows.append({"model_key": model_key, "creator": creator, "effort": effort,
                     "quality": r.get("rawScore"), "cost": r.get("costTask"),
                     "speed": r.get("speedTps")})
    return rows


def load_measured(path):
    rows = []
    for m in json.load(open(path)):
        if m["model"] not in {k for k in AA_MAP}:
            continue
        rows.append({"door": m["provider"], "model": m["model"],
                     "variant": m["variant"], "model_key": None,
                     "n": m["n"], "cond_rate": m["cond_rate"],
                     "tps_med": m["tps_med"]})
        rows[-1]["model_key"] = norm(AA_MAP[m["model"]][1])
    return rows


def propose(cands, seats, dead):
    """A seated (model, effort) is replaceable when some candidate dominates it in
    the seat's tier. Live doors only: a candidate with no measured door is a trial
    proposal, never a direct swap."""
    out = []
    for pool, members in seats.items():
        use_speed = pool in SPEED_TIERS
        for model, effort in members:
            seat_key = norm(AA_MAP.get(model, (None, model, False))[1])
            seat = next((c for c in cands if c["model"] == seat_key and c["effort"] == effort), None)
            if seat is None and (seat_key, effort) in dead:
                tier = [c for c in cands if c["tier"] == "standard"]
                by = sorted(tier, key=lambda c: -c["q"])[:3]
                out.append({"pool": pool, "seat": f"{model}#{effort}", "verdict": "dead-seat",
                            "by": [f"{c['model']}#{c['effort']}" for c in by]})
                continue
            if seat is None:
                out.append({"pool": pool, "seat": f"{model}#{effort}", "verdict": "no-aa-row"})
                continue
            tier = [c for c in cands if c["tier"] == seat["tier"]]
            doms = [c for c in tier if dominates(c, seat, use_speed)]
            if not doms:
                out.append({"pool": pool, "seat": f"{model}#{effort}", "verdict": "on-frontier"})
                continue
            live = [c for c in doms if c["source"] == "measured"]
            out.append({"pool": pool, "seat": f"{model}#{effort}",
                        "verdict": "replace" if live else "trial-first",
                        "by": [f"{c['model']}#{c['effort']}" for c in (live or doms)]})
    return out


def run(args):
    aa = load_aa(args.aa)
    measured = load_measured(args.measured)
    cands = build_candidates(aa, measured)
    for tier in ("cheap", "standard", "frontier"):
        group = [c for c in cands if c["tier"] == tier]
        front = frontier_fast(group, use_speed=False)
        print(f"{tier}: {len(group)} candidates, {len(front)} on frontier (quality/cost)")
    tested = collections.defaultdict(list)
    for m in measured:
        if m["n"] >= MIN_N and m["cond_rate"] is not None:
            tested[(m["model_key"], m["variant"])].append(m["cond_rate"])
    dead = {k for k, rates in tested.items() if max(rates) < LIVE_MIN}
    for p in propose(cands, POOL_SEATS, dead):
        print(json.dumps(p))


def extract(args):
    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    cut = 1794572108000 - WINDOW_DAYS * 86400000
    rows = con.execute(
        """select provider_id, model_id, coalesce(variant,'-'), outcome, error_tag,
                  first_token_ms, tokens_per_second
           from route_attempt where time_started > ?""", (cut,)).fetchall()
    g = collections.defaultdict(list)
    for r in rows:
        g[(r[0], r[1], r[2])].append(r)
    out = []
    for (prov, model, var), rs in g.items():
        if model not in AA_MAP:
            continue
        n = len(rs)
        ok = sum(1 for r in rs if r[3] == "success")
        walled = sum(1 for r in rs if r[4] in WALL_TAGS)
        live = n - walled
        tps = [r[6] for r in rs if r[3] == "success" and r[6]]
        ft = [r[5] for r in rs if r[3] == "success" and r[5]]
        out.append({"provider": prov, "model": model, "variant": var, "n": n, "ok": ok,
                    "walled": walled,
                    "cond_rate": round(ok / live, 4) if live > 0 else None,
                    "ft_med": round(statistics.median(ft), 1) if ft else None,
                    "tps_med": round(statistics.median(tps), 1) if tps else None,
                    "tags": dict(collections.Counter(r[4] for r in rs if r[3] != "success").most_common(5))})
    out.sort(key=lambda d: -d["n"])
    json.dump(out, open(args.out, "w"), indent=1)
    print(f"{len(out)} rows written to {args.out}")


def selftest(trials=3000, seed=20261113):
    rng = random.Random(seed)
    checked = 0
    for t in range(trials):
        n = rng.randint(0, 25)
        use_speed = rng.random() < 0.5
        # Small integer grids force ties, the case where dominance bugs hide.
        pts = [{"q": rng.randint(0, 6), "c": rng.randint(0, 6), "s": rng.randint(0, 6), "id": i}
               for i in range(n)]
        brute = {p["id"] for p in frontier_brute(pts, use_speed)}
        fast = {p["id"] for p in frontier_fast(pts, use_speed)}
        assert brute == fast, (t, use_speed, pts)
        shuffled = pts[:]
        rng.shuffle(shuffled)
        assert {p["id"] for p in frontier_fast(shuffled, use_speed)} == fast
        for p in pts:
            if p["id"] not in fast:
                assert any(dominates(k, p, use_speed) for k in pts), (t, p)
        for a in pts:
            for b in pts:
                assert not (dominates(a, b, use_speed) and dominates(b, a, use_speed))
        checked += 1
    print(f"selftest ok: {checked} random point sets, brute == skyline, order-free, "
          f"every dominated point has a frontier dominator, dominance antisymmetric")


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("extract")
    e.add_argument("--db", required=True)
    e.add_argument("--out", required=True)
    r = sub.add_parser("run")
    r.add_argument("--aa", required=True)
    r.add_argument("--measured", required=True)
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    if args.cmd == "extract":
        extract(args)
    elif args.cmd == "run":
        run(args)
    else:
        selftest()


if __name__ == "__main__":
    main(sys.argv[1:])
