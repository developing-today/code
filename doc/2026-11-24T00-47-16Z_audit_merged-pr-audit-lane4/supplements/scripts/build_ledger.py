import json, sys, collections

sys.path.insert(0, "/home/hatch/workspace/lane4-audit7")
from verdicts_fleet import V

census = {
    p["number"]: p
    for p in json.load(open("/home/hatch/workspace/lane4-audit7/census-fleet.json"))
}
assert set(census) == set(V), set(census) ^ set(V)
rows = []
for n in sorted(census):
    iss, verdict, label, note = V[n]
    p = census[n]
    rows.append(
        {
            "number": n,
            "url": p["html_url"],
            "repo": "dezren39/fleet",
            "title": p["title"],
            "merged_at": p["merged_at"],
            "spawning_issue": iss,
            "verdict": verdict,
            "label": label,
            "note": note,
        }
    )
json.dump(
    rows, open("/home/hatch/workspace/lane4-audit7/ledger-fleet.json", "w"), indent=1
)
print(collections.Counter(r["verdict"] for r in rows), len(rows))
