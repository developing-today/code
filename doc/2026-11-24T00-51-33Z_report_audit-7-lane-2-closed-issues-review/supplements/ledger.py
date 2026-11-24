import json, os, csv, re

OUT = os.path.expanduser("~/audit7-lane2")
rows = json.load(open(os.path.join(OUT, "ledger-flags.json")))
DTT = {
    2912,
    2913,
    2914,
    2915,
    3100,
    3101,
    3137,
    3138,
    3139,
    3140,
    3141,
    3147,
    3148,
    3152,
    3153,
    3155,
    3156,
    3157,
    3158,
    3162,
    3163,
    3494,
    3495,
    3496,
    3497,
    3498,
    3499,
    3500,
    3501,
    3502,
    3503,
    3504,
    3863,
    3864,
    4029,
    4030,
    4037,
    4038,
    4039,
    4040,
    4042,
    4043,
    4228,
    4229,
    4237,
    4239,
    4244,
    4245,
    4247,
    4248,
    4343,
    4344,
    4348,
    4349,
    4531,
    4532,
    4859,
    4860,
    4892,
    4893,
    4894,
    4895,
    4896,
    4897,
    4898,
    4899,
    4923,
    4924,
    4926,
    4927,
    4928,
    4929,
    4930,
    4931,
    4936,
    4937,
    4938,
    4939,
}
BOT = re.compile(
    r"^(Scheduled |Daily |Weekly |Manual pm-dispatch|Fleet lane probe|Persistent fleet lane)",
    re.I,
)


def verdict(r):
    repo, n = r["repo"], r["number"]
    sr = r["state_reason"]
    fl = set(r["flags"])
    t = r["title"] or ""
    if sr == "duplicate":
        if (repo, n) in [
            ("developing-today/code", 5284),
            ("developing-today/code", 5285),
        ]:
            return "clean — duplicate of #4942; fix landed via PR #4976 (verified in #5285 comment thread)"
        if (repo, n) == ("developing-today/code", 4982):
            return "clean — duplicate; remainder delivered under #4909 (closing comment, verified read)"
        return "queued-slice-2 — duplicate close, comment adjudication enqueued (task 21596)"
    if sr == "not_planned":
        if n in DTT and repo == "developing-today/code":
            return "clean (class) — pm-dispatch placeholder ('do the thing'); not_planned is the correct housekeeping close"
        if BOT.match(t):
            return "clean (class) — scheduled-bot/automation issue; not_planned housekeeping close"
        if t.startswith("main-broken"):
            return "flagged — main-broken closed not_planned instead of completed; pointer posted on umbrella #13871 for Lane 4; adjudication queued-slice-1/2"
        if "Preserved branch" in t:
            return "clean (class, provisional) — preserved-branch decision issue; not_planned = drop decision; comment confirmation queued-slice-1/2"
        return "queued-slice-1/2 — substantive not_planned close; full comment adjudication enqueued (tasks 21595/21596)"
    if "ZERO_COMMENTS" in fl:
        return "queued-slice-3 — completed close with zero comments; implementation verification enqueued (task 21598)"
    if "NO_LABELS" in fl:
        return (
            "queued-slice-4 — no labels at close; labelling pass enqueued (task 21600)"
        )
    return "clean — completed close with discussion on record; no gap signal in full-corpus triage"


with open(os.path.join(OUT, "ledger.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(
        [
            "repo",
            "number",
            "url",
            "closed_at",
            "state_reason",
            "comments",
            "labels",
            "flags",
            "title",
            "verdict",
        ]
    )
    for r in rows:
        w.writerow(
            [
                r["repo"],
                r["number"],
                r["url"],
                r["closed_at"],
                r["state_reason"],
                r["comments"],
                "|".join(r["labels"]),
                "|".join(r["flags"]),
                r["title"],
                verdict(r),
            ]
        )
from collections import Counter

c = Counter(verdict(r).split(" — ")[0].split(" (")[0] for r in rows)
print(c)
print("ledger rows", len(rows))
