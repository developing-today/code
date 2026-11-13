#!/usr/bin/env python3
# NOTE (2026-11-12, Drew ruling): the source workbook is DEPRECATED; the
# ainventory database is the primary source of record and recovery is a DB
# backup restore. The as-run note text below ("a future workbook re-import
# supersedes this batch") is SUPERSEDED by that ruling — workbook
# re-imports must not be run. See README.md in this directory and
# Maintenance_Log row 50 in the store.
"""Generate the ainventory load SQL for the AA web pull 2026-11-12.

Reads aa-results.json (the compiled dataset) and emits one transaction that
follows the store's own conventions:
- new 'aa-web' source row in the CURRENT batch (2026-11-02 precedent),
- Rankings sheet_rows updated in place for refreshed configs, appended for
  new ones; estimates flagged with the |estimated test_run_key suffix
  (the workbook's convention), previous-index rows keyed AA|v4.1 / AA|v4.3.2,
- Test_Runs rows for measured/previous rows only (never estimates),
- Benchmark_Versions row for AA|v4.1, Maintenance_Log row for the load,
- measurement_events: 8152e746 set carried forward under the current batch,
  current (null-version) configs refreshed/added for MEASURED v5 rows,
  versioned events for v5.0.1 + previous versions; estimates get NO events,
- price_events appended for the 9 dataset models.
"""

import json, math, sys

DATA = json.load(open("/home/hatch/workspace/model-review-2026-11-12/aa-results.json"))
BATCH = "fe02783b-046c-4401-bb35-8c529b216b8d"
OLD_EVENT_BATCH = "8152e746-4805-4a11-9664-317b44ca9f1b"
SNAP = "2026-11-12"

FAMILY = {
    "gemini-4-hydrogen": "Gemini 4 Hydrogen",
    "gemini-4-carbon": "Gemini 4 Carbon",
    "gemini-4-argon": "Gemini 4 Argon",
    "gemini-3.8-flash": "Gemini 3.8 Flash",
    "gemini-2.5-pro": "Gemini 2.5 Pro",
    "gemini-2.5-flash": "Gemini 2.5 Flash",
    "gemini-2.5-flash-thinking": "Gemini 2.5 Flash-Thinking",
    "gemini-2.5-flash-lite": "Gemini 2.5 Flash-Lite",
    "claude-sonnet-5-5": "Claude Sonnet 5.5",
    "claude-opus-5-5": "Claude Opus 5.5",
}
CREATOR = {m: ("Anthropic" if m.startswith("claude") else "Google") for m in FAMILY}
CSLUG = {m: ("anthropic" if m.startswith("claude") else "google") for m in FAMILY}
URL = {
    "gemini-4-hydrogen": "https://artificialanalysis.ai/models/gemini-4-hydrogen",
    "gemini-4-carbon": "https://artificialanalysis.ai/models/gemini-4-carbon",
    "gemini-4-argon": "https://artificialanalysis.ai/models/gemini-4-argon",
    "gemini-3.8-flash": "https://artificialanalysis.ai/models/gemini-3-8-flash",
    "gemini-2.5-pro": "https://artificialanalysis.ai/models/gemini-2-5-pro",
    "gemini-2.5-flash": "https://artificialanalysis.ai/models/gemini-2-5-flash",
    "gemini-2.5-flash-thinking": "https://artificialanalysis.ai/models/gemini-2-5-flash",
    "gemini-2.5-flash-lite": "https://artificialanalysis.ai/models/gemini-2-5-flash-lite",
    "claude-sonnet-5-5": "https://artificialanalysis.ai/models/claude-sonnet-5-5",
    "claude-opus-5-5": "https://artificialanalysis.ai/models/claude-opus-5-5",
}
# time per task (seconds) where the AA pull states it (aa-results.md)
TIME_TASK = {
    ("gemini-4-hydrogen", "low"): 78,
    ("gemini-4-hydrogen", "medium"): 222,
    ("gemini-4-hydrogen", "high"): 360,
    ("gemini-4-hydrogen", "xhigh"): 780,
    ("gemini-4-argon", "high"): 1812,
    ("gemini-3.8-flash", "high"): 888,
}
# total output tokens across the AA index run, where AA states it (aa-results.md)
TOTAL_OUT_TOKENS = {
    ("gemini-4-hydrogen", "xhigh"): 304000000,
    ("gemini-4-carbon", "high"): 205000000,
    ("gemini-4-argon", "high"): 113000000,
    ("gemini-3.8-flash", "high"): 126000000,
    ("claude-sonnet-5-5", "max"): 335000000,
    ("claude-opus-5-5", "max"): 218000000,
}
# previous-index version key per dataset row (ii_source AA-prev)
PREV_KEY = {  # model -> benchmark_version_key for its AA-prev rows
    "gemini-2.5-pro": "AA|v4.1",
    "gemini-2.5-flash-thinking": "AA|v4.1",
    "gemini-2.5-flash": "AA|v4.3.2",
    "gemini-2.5-flash-lite": "AA|v4.3.2",
}
PREV_VER = {"AA|v4.1": "4.1", "AA|v4.3.2": "4.3.2"}
# existing Rankings rows (row_index) refreshed in place: (model, effort) -> row_index
EXISTING_RK = {
    ("claude-sonnet-5-5", "low"): 171,
    ("claude-sonnet-5-5", "medium"): 172,
    ("claude-sonnet-5-5", "high"): 173,
    ("claude-sonnet-5-5", "xhigh"): 174,
    ("claude-sonnet-5-5", "max"): 175,
    ("claude-opus-5-5", "low"): 176,
    ("claude-opus-5-5", "medium"): 177,
    ("claude-opus-5-5", "high"): 178,
    ("claude-opus-5-5", "xhigh"): 179,
    ("claude-opus-5-5", "max"): 180,
    ("gemini-3.8-flash", "low"): 202,
    ("gemini-3.8-flash", "medium"): 203,
    ("gemini-3.8-flash", "high"): 204,
    ("gemini-4-argon", "high"): 213,
    ("gemini-4-carbon", "high"): 250,
}
# existing Rankings test_run_key per refreshed row (for Test_Runs joins)
EXISTING_KEY = {
    (
        "claude-sonnet-5-5",
        "low",
    ): "anthropic|claude-sonnet-5.5|low|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-sonnet-5-5",
        "medium",
    ): "anthropic|claude-sonnet-5.5|medium|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-sonnet-5-5",
        "high",
    ): "anthropic|claude-sonnet-5.5|high|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-sonnet-5-5",
        "xhigh",
    ): "anthropic|claude-sonnet-5.5|xhigh|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-sonnet-5-5",
        "max",
    ): "anthropic|claude-sonnet-5.5|max|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-opus-5-5",
        "low",
    ): "anthropic|claude-opus-5.5|low|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-opus-5-5",
        "medium",
    ): "anthropic|claude-opus-5.5|medium|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-opus-5-5",
        "high",
    ): "anthropic|claude-opus-5.5|high|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-opus-5-5",
        "xhigh",
    ): "anthropic|claude-opus-5.5|xhigh|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "claude-opus-5-5",
        "max",
    ): "anthropic|claude-opus-5.5|max|reasoning|fallback-true|aa-v5|snapshot-2026-10-31",
    (
        "gemini-3.8-flash",
        "low",
    ): "google|gemini-3.8-flash|low|reasoning|fallback-na|aa-v5|snapshot-2026-10-31",
    (
        "gemini-3.8-flash",
        "medium",
    ): "google|gemini-3.8-flash|medium|reasoning|fallback-na|aa-v5|snapshot-2026-10-31",
    (
        "gemini-3.8-flash",
        "high",
    ): "google|gemini-3.8-flash|high|reasoning|fallback-na|aa-v5|snapshot-2026-10-31",
    (
        "gemini-4-argon",
        "high",
    ): "google|gemini-4-argon|high|reasoning|fallback-na|aa-v5|snapshot-2026-10-31",
    (
        "gemini-4-carbon",
        "high",
    ): "google|gemini-4-carbon|high|reasoning|fallback-na|aa-v5|snapshot-2026-10-31",
}
EXISTING_VARIANT = {k: v.rsplit("|aa-v5|", 1)[0] for k, v in EXISTING_KEY.items()}
# existing null-version config events to update: (model, effort) -> event effort label
CLAUDE_EFFORT = {
    "low": "Adaptive Reasoning, Low Effort, Default Fallback",
    "medium": "Adaptive Reasoning, Medium Effort, Default Fallback",
    "high": "Adaptive Reasoning, High Effort, Default Fallback",
    "xhigh": "Adaptive Reasoning, Xhigh Effort, Default Fallback",
    "max": "Adaptive Reasoning, Max Effort, Default Fallback",
}


def event_effort(model, effort):
    return CLAUDE_EFFORT[effort] if model.startswith("claude") else effort


EXISTING_EVENTS = {  # (model, effort) pairs whose null-version event already exists
    ("claude-sonnet-5-5", "medium"),
    ("claude-sonnet-5-5", "high"),
    ("claude-sonnet-5-5", "xhigh"),
    ("claude-sonnet-5-5", "max"),
    ("claude-opus-5-5", "low"),
    ("claude-opus-5-5", "medium"),
    ("claude-opus-5-5", "high"),
    ("claude-opus-5-5", "xhigh"),
    ("claude-opus-5-5", "max"),
    ("gemini-3.8-flash", "medium"),
    ("gemini-3.8-flash", "high"),
}
MODEL_ID = {  # ainventory.models id used by prior events for this model
    "claude-opus-5-5": 3,
    "claude-sonnet-5-5": 4,
    "gemini-3.8-flash": 12,
    "gemini-4-argon": 35,
    "gemini-4-carbon": 775,
}
MODEL_ID_SLUG = {  # slug-generation models rows (backfill + versioned strata)
    "claude-opus-5-5": 25,
    "claude-sonnet-5-5": 27,
    "gemini-3.8-flash": 33,
    "gemini-4-argon": 35,
    "gemini-4-carbon": 775,
}
BACKFILL_BATCH = "21b09af3-66b6-40df-8f39-a58044d57271"  # code#4401 tranche 1
PRICE_ID = {  # v_current_prices model_name (models.dev style)
    "gemini-4-hydrogen": "google/gemini-4-hydrogen",
    "gemini-4-carbon": "google/gemini-4-carbon",
    "gemini-4-argon": "google/gemini-4-argon",
    "gemini-3.8-flash": "google/gemini-3.8-flash",
    "gemini-2.5-pro": "google/gemini-2.5-pro",
    "gemini-2.5-flash": "google/gemini-2.5-flash",
    "gemini-2.5-flash-lite": "google/gemini-2.5-flash-lite",
    "claude-sonnet-5-5": "anthropic/claude-sonnet-5.5",
    "claude-opus-5-5": "anthropic/claude-opus-5.5",
}


def fmt(x):
    if x is None:
        return ""
    if isinstance(x, int):
        return str(x)
    s = repr(float(x))
    return s[:-2] if s.endswith(".0") else s


def q(s):  # SQL string literal
    return "'" + str(s).replace("'", "''") + "'"


def jq(s):  # JSON string literal for jsonb_set values
    return q(json.dumps(str(s)))


rows = {(r["model"], r["effort"]): r for r in DATA}
_backup = json.load(
    open("/home/hatch/workspace/model-review-2026-11-12/store-backup-before.json")
)
OLD_CELLS = {
    e["row_index"]: e["data"]["cells"] for e in _backup[0]["snap"]["rankings_touch"]
}
out = []
out.append("BEGIN;")
out.append(f"""DO $$ BEGIN
  IF (SELECT import_batch FROM ainventory.v_latest_import_batch) <> '{BATCH}' THEN
    RAISE EXCEPTION 'latest batch drifted — aborting load';
  END IF;
END $$;""")

# --- models registry: new slugs ---------------------------------------------
out.append("""INSERT INTO ainventory.models (name, provider, first_seen, last_seen)
SELECT v.name, 'google', now(), now() FROM (VALUES
 ('google|gemini-4-hydrogen'), ('google|gemini-2.5-pro'), ('google|gemini-2.5-flash'),
 ('google|gemini-2.5-flash-thinking'), ('google|gemini-2.5-flash-lite')) AS v(name)
WHERE NOT EXISTS (SELECT 1 FROM ainventory.models m WHERE m.name = v.name);""")

# --- the aa-web source row (id captured to temp) ------------------------------
SRC_NOTE = (
    "AA Intelligence Index v5.0.1 web pull 2026-11-12 (Drew order; Supabase-only, no Drive — "
    "the 2026-11-02 aa-web route). Source: artificialanalysis.ai leaderboard + model pages, "
    "retrieved 2026-11-12 ~20:09-20:11 UTC. Loaded into this batch: NEW MODEL Gemini 4 Hydrogen "
    "(4 measured AA|v5 rows); Gemini 4 Carbon low/medium measured + high refreshed + xhigh "
    "ESTIMATED (|estimated key flag); Gemini 4 Argon high refreshed (II 50.4, $9.95/task) + "
    "low/medium/xhigh ESTIMATED; Gemini 3.8 Flash v5 refreshed to one-decimal values "
    "(22.5/30.1/30.8) + xhigh ESTIMATED; Claude Sonnet 5.5 + Opus 5.5 v5 refreshed to "
    "one-decimal values (Sonnet low config new); Gemini 2.5 family on PREVIOUS index versions "
    "only (Pro + Flash-Thinking keyed AA|v4.1, new Benchmark_Versions row; Flash + Flash-Lite "
    "keyed AA|v4.3.2) — old-scale, never comparable with v5. measurement_events: the 199 "
    "workbook-import events of batch 8152e746 carried forward under this batch (closing the "
    "post-2026-10-26 event gap), current configs refreshed/added for measured v5 rows, "
    "versioned 5.0.1 + previous-version events added; estimates carry NO workbook-import "
    "events (not measurements). ingest_batches registry row created for this batch (the FK "
    "home for the event/price loads). sheet-backfill mirror (code#4401): 15 events refreshed "
    "+ 11 new v5 events under a follow-on batch record. price_events appended for the 9 "
    "dataset models (AA list prices). "
    "Drive workbook unchanged; a future workbook re-import supersedes this batch. "
    "Full record: Maintenance_Log 2026-11-12."
)
out.append(f"""CREATE TEMP TABLE _src (id bigint) ON COMMIT DROP;
WITH ins AS (
  INSERT INTO ainventory.sources (name, kind, drive_file_id, drive_modified_at, import_batch, imported_at, note)
  VALUES ('Artificial Analysis Intelligence Index v5 — web pull 2026-11-12', 'aa-web', NULL, NULL,
          '{BATCH}', now(), {q(SRC_NOTE)})
  RETURNING id)
INSERT INTO _src SELECT id FROM ins;""")

# --- ingest_batches registration for the current workbook batch ----------------
# measurement_events/price_events carry an FK to ingest_batches; the 2026-10-31
# workbook import never got a registry row (the batch index left-joins it as
# optional). Register it now so this batch can hold the event/price loads.
out.append(f"""INSERT INTO ainventory.ingest_batches (id, source_kind, source_ref, ran_at, inserted_counts, note)
VALUES ('{BATCH}', 'workbook-import',
        'ai_model_benchmark_inventory_advanced_2026-10-21 (Google Sheet) + model_value_data.csv + AA web pull 2026-11-12',
        '2026-10-31T15:00:50.512735+00',
        '{{"measurement_events": 234, "price_events": 9}}'::jsonb,
        'Registry row created 2026-11-12 (the 2026-10-31 workbook import predates the registry; the '
        'aggregator batch index treats ingest_batches as optional metadata). Counts are the event/price '
        'rows riding under this batch after the 2026-11-12 AA web pull: 199 workbook-import events carried '
        'forward from batch 8152e746 + 35 events added by the pull; 9 price events appended by the pull.')
ON CONFLICT (id) DO NOTHING;""")

# --- Rankings: in-place refreshes ---------------------------------------------
for (model, effort), ri in sorted(EXISTING_RK.items(), key=lambda kv: kv[1]):
    r = rows[(model, effort)]
    sets = [
        (6, fmt(r["ii_score"])),
        (8, fmt(r["ii_score"])),
        (16, fmt(r["ii_score"] - 39)),
    ]
    if r["cost_per_task"] is not None:
        sets.append((9, fmt(r["cost_per_task"])))
    t = TIME_TASK.get((model, effort))
    if t is not None:
        sets.append((10, fmt(t)))
    if r["output_tps"] is not None:
        sets.append((11, fmt(r["output_tps"])))
    expr = "data"
    for idx, val in sets:
        expr = f"jsonb_set({expr}, '{{cells,{idx}}}', {jq(val)})"
    out.append(
        f"UPDATE ainventory.sheet_rows SET data = {expr} WHERE import_batch = '{BATCH}' AND sheet_name = 'Rankings' AND row_index = {ri};"
    )


# --- Rankings: appended rows ----------------------------------------------------
def variant_key(model, effort):
    seg4 = "nonreasoning" if effort == "non-reasoning" else "reasoning"
    fb = "fallback-true" if model.startswith("claude") else "fallback-na"
    return f"{CSLUG[model]}|{model}|{effort}|{seg4}|{fb}"


def run_key(model, effort, vkey, estimated=False):
    seg = vkey.split("|")[1].lower()  # v5 / v4.1 / v4.3.2
    k = f"{variant_key(model, effort)}|aa-{seg}|snapshot-{SNAP}"
    return k + "|estimated" if estimated else k


def qual_avail(model, effort):
    if model in ("gemini-4-argon",):
        return "Tracked", "limited"
    if model.startswith("gemini-2.5"):
        return "Tracked", "limited"
    return "Contender", "yes"


new_rk = []
for (model, effort), r in rows.items():
    if (model, effort) in EXISTING_RK:
        continue
    estimated = r["ii_source"] == "GUESS"
    vkey = "AA|v5" if r["ii_source"] != "AA-prev" else PREV_KEY[model]
    new_rk.append((model, effort, r, vkey, estimated))
new_rk.sort(key=lambda x: (x[0], x[1]))
ri = 287
rk_keys = {}  # (model, effort) -> (run_key, variant_key, vkey, estimated)
new_cells = {}  # (model, effort) -> (row_index, cells) for the backfill mirror
for model, effort, r, vkey, estimated in new_rk:
    vk = variant_key(model, effort)
    rk = run_key(model, effort, vkey, estimated)
    rk_keys[(model, effort)] = (rk, vk, vkey, estimated)
    qual, avail = qual_avail(model, effort)
    eligible = (
        "1"
        if (
            vkey == "AA|v5"
            and qual in ("Contender", "Close")
            and avail in ("yes", "limited")
        )
        else "0"
    )
    score = r["ii_score"]
    cells = [
        rk,
        vk,
        FAMILY[model],
        CREATOR[model],
        effort,
        vkey,
        fmt(score),
        "1",
        fmt(score),
        fmt(r["cost_per_task"]),
        fmt(TIME_TASK.get((model, effort))),
        fmt(r["output_tps"]),
        qual,
        avail,
        eligible,
        "40",
        fmt(score - 39),
    ] + [""] * 21
    if r["cost_per_task"]:
        cells[29] = repr(score / r["cost_per_task"])
    t = TIME_TASK.get((model, effort))
    if t:
        cells[30] = repr(score / t)
    assert len(cells) == 38, len(cells)
    cells_json = json.dumps(cells)
    new_cells[(model, effort)] = (ri, cells)
    out.append(f"""INSERT INTO ainventory.sheet_rows (source_id, import_batch, sheet_name, row_index, data)
SELECT (SELECT id FROM _src), '{BATCH}', 'Rankings', {ri}, jsonb_build_object('cells', {q(cells_json)}::jsonb);""")
    ri += 1

# --- Test_Runs: appended rows for measured + previous rows only -----------------
tri = 185
for (model, effort), r in sorted(rows.items()):
    if r["ii_source"] == "GUESS":
        continue
    if (model, effort) in EXISTING_KEY:
        rk, vk = EXISTING_KEY[(model, effort)], EXISTING_VARIANT[(model, effort)]
        vkey = "AA|v5"
        bver = "v5.0.1"
    else:
        rk, vk, vkey, _ = rk_keys[(model, effort)]
        bver = {"AA|v5": "v5.0.1", "AA|v4.1": "v4.1", "AA|v4.3.2": "v4.3.2"}[vkey]
    score = r["ii_score"]
    notes = (
        f"AA web pull {SNAP} (Supabase-only); Intelligence Index {bver}"
        if vkey == "AA|v5"
        else f"AA web pull {SNAP} (Supabase-only); previous index {bver} — not comparable with v5 scores"
    )
    cells = [
        rk,
        vk,
        "Artificial Analysis Intelligence Index",
        bver,
        vkey,
        SNAP,
        SNAP,
        fmt(score),
        fmt(round(score * score, 4)),
        fmt(r["cost_per_task"]),
        "",
        "",
        "",
        fmt(TOTAL_OUT_TOKENS.get((model, effort))),
        fmt(r["output_tps"]),
        fmt(r["ttft_s"]),
        fmt(r["ttft_s"]),
        "",
        fmt(TIME_TASK.get((model, effort))),
        fmt(r["price_in"]),
        fmt(r["price_out"]),
        "",
        "1000000",
        URL[model],
        "AA current public page",
        notes,
    ]
    cells_json = json.dumps(cells)
    out.append(f"""INSERT INTO ainventory.sheet_rows (source_id, import_batch, sheet_name, row_index, data)
SELECT (SELECT id FROM _src), '{BATCH}', 'Test_Runs', {tri}, jsonb_build_object('cells', {q(cells_json)}::jsonb);""")
    tri += 1

# --- Benchmark_Versions: AA|v4.1 row --------------------------------------------
bv_notes = (
    "Previous AA Intelligence Index generation (v4.1), superseded by v4.3.2 and then v5. "
    "Scores stand on the v4.1 scale; no empirical bridge to v4.3.2 or v5 computed — never "
    "compare across versions. Rows keyed AA|v4.1 loaded 2026-11-12 from the AA web pull: "
    "Gemini 2.5 Pro (high, II 25.8) and Gemini 2.5 Flash-Thinking (thinking, II 14.1) — "
    "models AA no longer carries on the current index, retained so the fleet's remaining "
    "2.5 references keep their last measured scores."
)
bv_cells = json.dumps(
    [
        "AA|v4.1",
        "Artificial Analysis Intelligence Index",
        "v4.1",
        "",
        "AA|v4.3.2",
        "1",
        "",
        "1",
        "1",
        "0",
        "low",
        bv_notes,
    ]
)
out.append(f"""INSERT INTO ainventory.sheet_rows (source_id, import_batch, sheet_name, row_index, data)
SELECT (SELECT id FROM _src), '{BATCH}', 'Benchmark_Versions', 103, jsonb_build_object('cells', {q(bv_cells)}::jsonb);""")

# --- Maintenance_Log row ----------------------------------------------------------
ml_details = (
    "AA web pull 2026-11-12 loaded Supabase-only (Drew order; route of the 2026-11-02 aa-web pull; "
    "Drive workbook untouched). Dataset: ~/workspace/model-review-2026-11-12/aa-results.md + .json "
    "(AA leaderboard + model pages, retrieved 2026-11-12 ~20:09-20:11 UTC; Intelligence Index v5.0.1). "
    "Rankings: 13 rows refreshed in place (Sonnet 5.5 x5, Opus 5.5 x5, Gemini 3.8 Flash x3 v5 values to "
    "one decimal; Argon high 50->50.4 with AA cost $7.68->$9.95 and time 1362s->1812s; Carbon high 59->60.0); "
    "15 rows appended 287-301 (Gemini 4 Hydrogen low/medium/high/xhigh NEW; Carbon low/medium; 2.5 Pro, "
    "2.5 Flash, 2.5 Flash-Thinking, 2.5 Flash-Lite on previous versions; 5 ESTIMATED rows — Carbon xhigh, "
    "Argon low/medium/xhigh, 3.8 Flash xhigh — flagged by the |estimated test_run_key suffix per the "
    "2026-10-31 convention, methods in aa-results.md). Test_Runs: 25 rows appended 185-209 (measured + "
    "previous only; v5 rows gain TTFT/timing the 2026-10-31 load lacked). Benchmark_Versions: AA|v4.1 row "
    "appended (factor 1, own scale, no bridge). measurement_events: all 199 workbook-import events of batch "
    "8152e746 carried forward under this batch (post-2026-10-26 event gap), 11 current configs superseded by corrected re-emissions of "
    "the refreshed values, 10 current configs inserted (Hydrogen x4, Carbon low/medium/high, Argon high, "
    "Sonnet low, 3.8 Flash low), 25 versioned events added (21 x v5.0.1, 2 x 4.1, 2 x 4.3.2); estimates "
    "carry NO workbook-import events. ingest_batches registry row created for this batch "
    "(measurement_events 234, price_events 9). sheet-backfill stratum (code#4401): 15 events updated in "
    "place + 11 inserted under a follow-on batch record; previous-version rows reconcile to the "
    "workbook-import events per tranche 1. price_events: 9 appended (AA list prices for the dataset models). "
    "Derived Rankings cells beyond the input columns recompute in the aggregator views (v_rankings maths); "
    "stored copies on refreshed rows updated for raw/adjusted/cost/time/speed/usable only."
)
ml_cells = json.dumps(
    [
        SNAP,
        "AA refresh — web pull (Supabase-only)",
        ml_details,
        "muse (Drew order 2026-11-12)",
    ]
)
out.append(f"""INSERT INTO ainventory.sheet_rows (source_id, import_batch, sheet_name, row_index, data)
VALUES (41, '{BATCH}', 'Maintenance_Log', 49, jsonb_build_object('cells', {q(ml_cells)}::jsonb));""")

# --- measurement_events: carry-forward emission -------------------------------------
# The store is append-only (#1248: UPDATE/DELETE refused by trigger). A refresh
# therefore re-emits the winning event set under the new batch id, with the 11
# superseded current configs EXCLUDED here and re-emitted corrected below.
_superseded = ",\n    ".join(
    f"({q(FAMILY[m])}, {q(event_effort(m, e))})" for (m, e) in sorted(EXISTING_EVENTS)
)
out.append(f"""INSERT INTO ainventory.measurement_events
  (model_id, model_name, model_variant_key, run_key, effort, benchmark, benchmark_version, metric,
   value_num, intelligence_index, ttft_seconds, output_tps, output_tokens_per_task, cost_per_task_usd,
   task_time_seconds, source_kind, source_ref, source_date, ingested_at, ingest_batch_id, raw)
SELECT model_id, model_name, model_variant_key, run_key, effort, benchmark, benchmark_version, metric,
   value_num, intelligence_index, ttft_seconds, output_tps, output_tokens_per_task, cost_per_task_usd,
   task_time_seconds, source_kind, source_ref, source_date, now(), '{BATCH}', raw
FROM ainventory.measurement_events
WHERE ingest_batch_id = '{OLD_EVENT_BATCH}'
  AND NOT (benchmark_version IS NULL AND metric = 'intelligence_index'
           AND (model_name, effort) IN ({_superseded}));""")


# --- measurement_events: new rows ---------------------------------------------------
def model_id_sql(model):
    if model in MODEL_ID:
        return str(MODEL_ID[model])
    return f"(SELECT id FROM ainventory.models WHERE name = {q(CSLUG[model] + '|' + model)})"


def event_row(model, effort, r, bver):
    if (model, effort) in EXISTING_KEY:
        rk, vk = EXISTING_KEY[(model, effort)], EXISTING_VARIANT[(model, effort)]
    else:
        rk, vk = rk_keys[(model, effort)][0], rk_keys[(model, effort)][1]
    raw = {
        "url": URL[model],
        "cost": fmt(r["cost_per_task"]),
        "score": fmt(r["ii_score"]),
        "speed": fmt(r["output_tps"]),
        "terminal": "",
        "ii_source": r["ii_source"],
        "pull": f"aa-web {SNAP}",
        "test_run_key": rk,
        "model": f"{FAMILY[model]} ({event_effort(model, effort)})",
    }
    t = TIME_TASK.get((model, effort))
    return (
        f"({model_id_sql(model)}, {q(FAMILY[model])}, {q(vk)}, {q(rk)}, {q(event_effort(model, effort))}, "
        f"'AA Intelligence Index', {q(bver) if bver else 'NULL'}, 'intelligence_index', "
        f"{r['ii_score']}, {r['ii_score']}, "
        f"{r['ttft_s'] if r['ttft_s'] is not None else 'NULL'}, "
        f"{r['output_tps'] if r['output_tps'] is not None else 'NULL'}, NULL, "
        f"{r['cost_per_task'] if r['cost_per_task'] is not None else 'NULL'}, "
        f"{t if t is not None else 'NULL'}, 'workbook-import', {q(URL[model])}, DATE '{SNAP}', now(), "
        f"'{BATCH}', {q(json.dumps(raw))}::jsonb)"
    )


def slug_model_id_sql(model):
    if model in MODEL_ID_SLUG:
        return str(MODEL_ID_SLUG[model])
    return f"(SELECT id FROM ainventory.models WHERE name = {q(CSLUG[model] + '|' + model)})"


def versioned_row(model, effort, r, bver, vkey):
    """Rankings-derived versioned event, mirroring the 8152e746 stratum shape:
    model_name = slug base, plain effort, raw = named-field object."""
    if (model, effort) in EXISTING_KEY:
        rk, vk = EXISTING_KEY[(model, effort)], EXISTING_VARIANT[(model, effort)]
        row_index = EXISTING_RK[(model, effort)]
    else:
        rk, vk = rk_keys[(model, effort)][0], rk_keys[(model, effort)][1]
        row_index = new_cells[(model, effort)][0]
    qual, avail = qual_avail(model, effort)
    if (model, effort) in EXISTING_RK:  # keep the stored row's status cells
        old = OLD_CELLS[EXISTING_RK[(model, effort)]]
        qual, avail = old[12], old[13]
    eligible = (
        1
        if (
            vkey == "AA|v5"
            and qual in ("Contender", "Close")
            and avail in ("yes", "limited")
        )
        else 0
    )
    score = r["ii_score"]
    t = TIME_TASK.get((model, effort))
    raw = {
        "test_run_key": rk,
        "model_variant_key": vk,
        "family": FAMILY[model],
        "creator": CREATOR[model],
        "effort": effort,
        "benchmark_version_key": vkey,
        "raw_score": score,
        "version_level_factor": 1,
        "adjusted_score": score,
        "cost_task": r["cost_per_task"],
        "time_task_sec": t,
        "speed_tps": r["output_tps"],
        "qual_status": qual,
        "availability": avail,
        "eligible": eligible,
        "target": 40,
        "usable_units": score - 39,
        "points_per_dollar": (score / r["cost_per_task"])
        if r["cost_per_task"]
        else None,
        "points_per_second": (score / t) if t else None,
        "row_index": row_index,
        "ii_source": r["ii_source"],
    }
    return (
        f"({slug_model_id_sql(model)}, {q(CSLUG[model] + '|' + model)}, {q(vk)}, {q(rk)}, {q(effort)}, "
        f"'AA Intelligence Index', {q(bver)}, 'intelligence_index', "
        f"{score}, {score}, "
        f"{r['ttft_s'] if r['ttft_s'] is not None else 'NULL'}, "
        f"{r['output_tps'] if r['output_tps'] is not None else 'NULL'}, NULL, "
        f"{r['cost_per_task'] if r['cost_per_task'] is not None else 'NULL'}, "
        f"{t if t is not None else 'NULL'}, 'workbook-import', "
        f"{q('aa-web:Rankings (AA web pull 2026-11-12; artificialanalysis.ai)')}, DATE '{SNAP}', now(), "
        f"'{BATCH}', {q(json.dumps(raw))}::jsonb)"
    )


new_current, versioned = [], []
for (model, effort), r in rows.items():
    if r["ii_source"] == "GUESS":
        continue
    if r["ii_source"] == "AA-prev":
        versioned.append(
            versioned_row(model, effort, r, PREV_VER[PREV_KEY[model]], PREV_KEY[model])
        )
        continue
    versioned.append(versioned_row(model, effort, r, "5.0.1", "AA|v5"))
    # every measured v5 config is (re-)emitted as a current (null-version) event
    # under this batch: 10 genuinely new + 11 corrected superseding the old rows
    new_current.append(event_row(model, effort, r, None))
COLS = (
    "(model_id, model_name, model_variant_key, run_key, effort, benchmark, benchmark_version, metric, "
    "value_num, intelligence_index, ttft_seconds, output_tps, output_tokens_per_task, cost_per_task_usd, "
    "task_time_seconds, source_kind, source_ref, source_date, ingested_at, ingest_batch_id, raw)"
)
out.append(
    f"INSERT INTO ainventory.measurement_events {COLS} VALUES\n"
    + ",\n".join(new_current)
    + ";"
)
out.append(
    f"INSERT INTO ainventory.measurement_events {COLS} VALUES\n"
    + ",\n".join(versioned)
    + ";"
)

# --- sheet-backfill stratum follow-on (code#4401 mapping rule) ----------------------
out.append(f"""CREATE TEMP TABLE _bf (id uuid) ON COMMIT DROP;
WITH ins AS (
  INSERT INTO ainventory.ingest_batches (source_kind, source_ref, ran_at, inserted_counts, note)
  VALUES ('sheet-backfill',
          'code#4401 follow-on: AA web pull 2026-11-12 — Rankings refresh + new v5 rows (batch {BATCH})',
          now(), '{{"measurement_events": 26}}'::jsonb,
          'Follow-on to tranche 1 (ingest batch {BACKFILL_BATCH}): the 2026-11-12 AA web pull refreshed '
          '15 Rankings rows in place and appended 15 more (11 of them AA|v5). The store is append-only '
          '(#1248), so this record carries 26 NEW events under the tranche-1 mapping (model_variant_key '
          'verbatim, model_name = family + effort in parens, benchmark_version 5, raw = Rankings cells): '
          '15 supersede the tranche-1 events for the same run_keys with the refreshed values, 11 cover '
          'the new AA|v5 Rankings rows. The 4 previous-version rows (Gemini 2.5 family) get no backfill '
          'events — per tranche 1, versioned rows reconcile against the workbook-import stratum instead.')
  RETURNING id)
INSERT INTO _bf SELECT id FROM ins;""")


def backfill_insert(model, effort, ri, cells, source_ref_sql):
    r = rows[(model, effort)]

    def numcell(i):
        v = cells[i]
        return v if v not in ("", None) else "NULL"

    return f"""INSERT INTO ainventory.measurement_events
  (model_id, model_name, model_variant_key, run_key, effort, benchmark, benchmark_version, metric,
   value_num, intelligence_index, ttft_seconds, output_tps, output_tokens_per_task, cost_per_task_usd,
   task_time_seconds, source_kind, source_ref, source_date, ingested_at, ingest_batch_id, raw)
SELECT {slug_model_id_sql(model)}, {q(FAMILY[model] + " (" + effort + ")")}, {q(cells[1])}, {q(cells[0])}, {q(effort)},
  'AA Intelligence Index', '5', 'intelligence_index', {r["ii_score"]}, {r["ii_score"]}, NULL,
  {numcell(11)}, NULL, {numcell(9)}, {numcell(10)}, 'sheet-backfill',
  {source_ref_sql},
  DATE '{SNAP}', now(), (SELECT id FROM _bf), {q(json.dumps(cells))}::jsonb;"""


for (model, effort), ri in sorted(EXISTING_RK.items(), key=lambda kv: kv[1]):
    r = rows[(model, effort)]
    cells = list(OLD_CELLS[ri])
    cells[6] = fmt(r["ii_score"])
    cells[8] = fmt(r["ii_score"])
    cells[16] = fmt(r["ii_score"] - 39)
    if r["cost_per_task"] is not None:
        cells[9] = fmt(r["cost_per_task"])
    t = TIME_TASK.get((model, effort))
    if t is not None:
        cells[10] = fmt(t)
    if r["output_tps"] is not None:
        cells[11] = fmt(r["output_tps"])
    out.append(
        backfill_insert(
            model,
            effort,
            ri,
            cells,
            f"'sheet_rows source_id=41 import_batch={BATCH} row_index={ri} sheet=Rankings'",
        )
    )

for (model, effort), (ri, cells) in sorted(new_cells.items(), key=lambda kv: kv[1][0]):
    if cells[5] != "AA|v5":
        continue
    out.append(
        backfill_insert(
            model,
            effort,
            ri,
            cells,
            f"'sheet_rows source_id=' || (SELECT id FROM _src)::text || ' import_batch={BATCH} row_index={ri} sheet=Rankings'",
        )
    )

# --- price_events ---------------------------------------------------------------------
seen_price = {}
for r in DATA:
    seen_price[r["model"]] = r
price_rows = []
for model, r in sorted(seen_price.items()):
    if model not in PRICE_ID:
        continue
    raw = {
        "note": "AA-published list price (USD per 1M tokens), web pull 2026-11-12",
        "ii_source": "AA",
    }
    price_rows.append(
        f"(NULL, {q(PRICE_ID[model])}, NULL, NULL, {r['price_in']}, {r['price_out']}, NULL, NULL, "
        f"0, 1000000, 'flat', NULL, NULL, DATE '{SNAP}', 'aa-web', "
        f"{q('https://artificialanalysis.ai/leaderboards/models')}, DATE '{SNAP}', now(), '{BATCH}', "
        f"{q(json.dumps(raw))}::jsonb)"
    )
out.append(
    """INSERT INTO ainventory.price_events
  (model_id, model_name, model_variant_key, provider, input_usd_per_1m, output_usd_per_1m,
   cached_input_usd_per_1m, cache_write_usd_per_1m, context_min_tokens, context_max_tokens,
   window_kind, window_hours, window_tz, effective_date, source_kind, source_ref, source_date,
   ingested_at, ingest_batch_id, raw) VALUES
"""
    + ",\n".join(price_rows)
    + ";"
)

out.append("COMMIT;")
sql = "\n\n".join(out)
open("/home/hatch/workspace/model-review-2026-11-12/aa-load-2026-11-12.sql", "w").write(
    sql
)
print(
    f"wrote {len(sql)} bytes; new rankings rows: {len(new_rk)}; test runs to 184+{tri - 185}"
)
