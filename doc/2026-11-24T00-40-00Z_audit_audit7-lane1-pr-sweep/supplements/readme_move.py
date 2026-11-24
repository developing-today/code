p = "/home/user/git/code-audit7-lane1/doc/README.md"
t = open(p).read()
mine = "- `2026-11-24T00-40-00Z_audit_audit7-lane1-pr-sweep/` — AUDIT-7 Lane 1 PR-SWEEP audit: every open PR across the four repos read and dispositioned — 103 merged, 3 duplicates marked superseded, 90 blocked with named unblockers (tracking #13872)."
lines = [
    l
    for l in t.splitlines()
    if not l.startswith(("<<<<<<<", "=======", ">>>>>>>"))
    and "audit7-lane1-pr-sweep" not in l
]
out = []
for l in lines:
    if l.startswith("This list is the workflow"):
        out.append(mine)
        out.append("")
    out.append(l)
open(p, "w").write("\n".join(out) + "\n")
print("moved")
