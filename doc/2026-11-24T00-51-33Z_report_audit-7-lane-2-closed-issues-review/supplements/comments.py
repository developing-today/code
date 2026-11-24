import subprocess, json, os

OUT = os.path.expanduser("~/audit7-lane2")
rows = json.load(open(os.path.join(OUT, "ledger-flags.json")))
targets = {}
for r in rows:
    fl = set(r["flags"])
    if (
        r["state_reason"] in ("not_planned", "duplicate")
        or "ZERO_COMMENTS" in fl
        or "SHORT_BODY" in fl
    ):
        targets.setdefault(r["repo"], []).append(r["number"])
print({k: len(v) for k, v in targets.items()}, flush=True)


def api(path):
    out = subprocess.run(
        ["fleet", "gh", "api", path, "--paginate"],
        capture_output=True,
        text=True,
        timeout=1200,
    )
    if out.returncode != 0:
        print("ERR", path, (out.stderr or "")[-200:], flush=True)
        return []
    txt = out.stdout.strip()
    items = []
    dec = json.JSONDecoder()
    i = 0
    while i < len(txt):
        while i < len(txt) and txt[i] in " \n\r\t":
            i += 1
        if i >= len(txt):
            break
        obj, j = dec.raw_decode(txt, i)
        i = j
        if isinstance(obj, list):
            items.extend(obj)
        else:
            items.append(obj)
    return items


result = {}
for repo, nums in targets.items():
    result[repo] = {}
    for n in sorted(nums):
        cs = api(f"repos/{repo}/issues/{n}/comments?per_page=100")
        result[repo][str(n)] = [
            {
                "user": (c.get("user") or {}).get("login"),
                "created_at": c.get("created_at"),
                "body": c.get("body") or "",
            }
            for c in cs
        ]
        print(repo, n, len(cs), flush=True)
    json.dump(result, open(os.path.join(OUT, "flagged-comments.json"), "w"))
print("COMMENTS DONE", flush=True)
