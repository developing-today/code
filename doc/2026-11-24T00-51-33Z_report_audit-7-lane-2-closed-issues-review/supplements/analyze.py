import json, os, re, sys, glob
OUT=os.path.expanduser("~/audit7-lane2")
rows=[]
for path in sorted(glob.glob(os.path.join(OUT,"list-*.json"))):
    repo=path.split("list-")[1][:-5].replace("__","/",1)
    data=json.load(open(path))
    for it in data:
        body=it.get("body") or ""
        labels=[l["name"] for l in it.get("labels") or []]
        flags=[]
        sr=it.get("state_reason")
        if sr=="not_planned": flags.append("NOT_PLANNED")
        if sr=="duplicate": flags.append("DUPLICATE")
        if (it.get("comments") or 0)==0: flags.append("ZERO_COMMENTS")
        if not labels: flags.append("NO_LABELS")
        if not re.search(r"#\d+|pull/|/issues/|commit", body): flags.append("NO_REFS_IN_BODY")
        if re.search(r"follow[- ]?up|separate issue|split out|later|deferred|out of scope", body, re.I): flags.append("FOLLOWUP_LANG")
        if len(body)<200: flags.append("SHORT_BODY")
        rows.append({"repo":repo,"number":it["number"],"title":it.get("title"),"url":it.get("html_url"),"closed_at":it.get("closed_at"),"state_reason":sr,"comments":it.get("comments"),"labels":labels,"flags":flags,"body_len":len(body)})
rows.sort(key=lambda r:(r["repo"],r["number"]))
json.dump(rows, open(os.path.join(OUT,"ledger-flags.json"),"w"), indent=1)
from collections import Counter
print("total", len(rows))
print(Counter(r["repo"] for r in rows))
print(Counter(f for r in rows for f in r["flags"]))
print("not_planned:", [(r["repo"],r["number"]) for r in rows if r["state_reason"]=="not_planned"][:50])
print("duplicate:", [(r["repo"],r["number"]) for r in rows if r["state_reason"]=="duplicate"][:50])
