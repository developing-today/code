import json, os, glob

OUT = os.path.expanduser("~/audit7-lane2")
rows = json.load(open(os.path.join(OUT, "ledger-flags.json")))
flagged = {
    (r["repo"], r["number"])
    for r in rows
    if r["state_reason"] in ("not_planned", "duplicate")
    or "ZERO_COMMENTS" in r["flags"]
    or "SHORT_BODY" in r["flags"]
}
out = []
for path in sorted(glob.glob(os.path.join(OUT, "list-*.json"))):
    repo = path.split("list-")[1][:-5].replace("__", "/", 1)
    for it in json.load(open(path)):
        if (repo, it["number"]) in flagged or repo == "dezren39/fleet":
            out.append(
                {
                    "repo": repo,
                    "number": it["number"],
                    "title": it.get("title"),
                    "url": it.get("html_url"),
                    "closed_at": it.get("closed_at"),
                    "state_reason": it.get("state_reason"),
                    "comments": it.get("comments"),
                    "labels": [l["name"] for l in it.get("labels") or []],
                    "body": it.get("body") or "",
                }
            )
out.sort(key=lambda r: (r["repo"], r["number"]))
json.dump(out, open(os.path.join(OUT, "flagged-full.json"), "w"), indent=1)
print("flagged-full", len(out))
