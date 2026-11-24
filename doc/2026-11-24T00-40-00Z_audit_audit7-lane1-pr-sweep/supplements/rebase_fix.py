p = "/home/user/git/code-audit7-lane1/doc/README.md"
t = open(p).read()
mine = "- `2026-11-24T00-40-00Z_audit_audit7-lane1-pr-sweep/` — AUDIT-7 Lane 1 PR-SWEEP audit: every open PR across the four repos read and dispositioned — 103 merged, 3 duplicates marked superseded, 90 blocked with named unblockers (tracking #13872)."
# strip conflict markers and any partial copies, then re-add mine after the design-gaps line
lines = [
    l
    for l in t.splitlines()
    if not l.startswith(("<<<<<<<", "=======", ">>>>>>>"))
    and "audit7-lane1-pr-sweep" not in l
]
out = []
for l in lines:
    out.append(l)
    if "2026-11-24T00-24-54Z_audit_design-gaps" in l:
        out.append(mine)
open(p, "w").write("\n".join(out) + "\n")
print("readme fixed")
