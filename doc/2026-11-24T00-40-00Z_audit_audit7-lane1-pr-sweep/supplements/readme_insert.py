p='/home/user/git/code-audit7-lane1/doc/README.md'
lines=open(p).read().splitlines(keepends=True)
entry='- `2026-11-24T00-40-00Z_audit_audit7-lane1-pr-sweep/` — AUDIT-7 Lane 1 PR-SWEEP audit: every open PR across the four repos read and dispositioned — 103 merged, 3 duplicates marked superseded, 90 blocked with named unblockers (tracking #13872).\n'
if any('audit7-lane1-pr-sweep' in l for l in lines):
    print('already present')
else:
    for i,l in enumerate(lines):
        if '2026-11-24T00-24-54Z_audit_design-gaps' in l:
            lines.insert(i+1, entry); break
    else: raise SystemExit('anchor not found')
    open(p,'w').write(''.join(lines)); print('inserted')
