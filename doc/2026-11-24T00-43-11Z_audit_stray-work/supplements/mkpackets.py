import json

base = dict(
    schema="repo-health/v1",
    kind="feature_slice",
    repo="developing-today/code",
    max_attempts=2,
    timebox_minutes=120,
    risk="low: label + sub-issue metadata only, no code changes, no closes",
    tests=[
        "readback via fleet gh api: parent sub_issues list contains child; child labels contain stray-work + primary-parent/*"
    ],
)
slices = [
    (
        "A",
        "AUDIT-7 Lane 7 slice A: formalise text-linked stray issues in code (521 open)",
        "developing-today/code has 521 OPEN issues (AUDIT-7 Lane 7 census 2026-11-23, tracking #13874) that carry no epic/* or primary-parent/* label but reference another issue in the body. For each, one by one, read the issue and its comments in full (comments per_page=100, count-confirmed): if a comment claims the item for another AUDIT-7 lane, SKIP it (hard stop). Identify the referenced design/epic/parent; add labels stray-work + the matching epic/<slug> and primary-parent/<slug>; where the parent is an epic issue and the item is not already a sub-issue, add it via POST /issues/<parent>/sub_issues. If no appropriate parent exists, label stray/needs-parent instead and list it on #13874. Post per-batch progress on #13874.",
        [
            "521 items labelled stray-work with correct epic/primary-parent labels or stray/needs-parent",
            "every edited item read in full incl. comments; lane-claimed items untouched and listed",
            "progress comment on code#13874 with counts",
        ],
    ),
    (
        "B",
        "AUDIT-7 Lane 7 slice B: remaining orphaned open issues in code (~49) + needs-parent triage",
        "After Lane 7 batches 1-2 (34 orphans linked, see code#13874), ~49 OPEN issues in developing-today/code remain with no parent label and no issue reference in the body (orphan list in the Lane 7 report doc). For each, one by one, read in full incl. comments (per_page=100, count-confirmed; lane claim = hard stop): assign the appropriate maintenance epic (id #12462, fleet #12460, mcpx #12459, atlas #12457, pm-stack #12458, lab-cluster #12461, main-broken #8976, scheduled-tasks #8979, daily-maintenance #8978, design-register #8986) as sub-issue + labels stray-work, epic/<slug>, primary-parent/<slug>; or label stray/needs-parent and propose a new parent on #13874 where none fits. Also triage items labelled stray/needs-parent in slice A.",
        [
            "all remaining orphan open issues dispositioned: linked or needs-parent with a named proposed parent",
            "progress + final counts on code#13874",
        ],
    ),
    (
        "C",
        "AUDIT-7 Lane 7 slice C: forks + fleet open items stray-work pass (80 items)",
        "Open items at AUDIT-7 census (2026-11-23): developing-today-forks/opencode 39, developing-today-forks/openchamber 32, dezren39/fleet 9. Labels stray-work / stray/needs-parent / audit-7 already created in all three repos. For each item, one by one, read in full incl. comments (per_page=100; lane claim = hard stop - lanes 2-4 own fork PR scopes; where another lane owns the item, record the linkage finding as a pointer comment on umbrella code#13871 instead of editing). Label stray items; link each to its parent in developing-today/code where one exists (opencode-platform epic, epic(openchamber) code#1234, fleet maintenance epic code#12460) via a comment on the item naming the parent, and a pointer comment on the parent. dezren39/fleet has only 15 labels and 106/113 window items unlabeled: apply the code-repo label taxonomy subset (kind/*, stray-work) to its 19 window issues.",
        [
            "all 80 open fork/fleet items read and dispositioned",
            "cross-repo parent links recorded on both item and parent",
            "progress on code#13874",
        ],
    ),
]
for tag, title, body, acc in slices:
    p = {
        "title": title,
        "body": body,
        "priority": 60,
        "dedupe_key": f"audit7:lane7:feature_slice:code:13874:{tag}",
        "payload": {
            **base,
            "slice": tag,
            "issue": 13874,
            "problem": body,
            "scope": body,
            "acceptance": acc,
            "context_refs": [
                "https://github.com/developing-today/code/issues/13874",
                "https://github.com/developing-today/code/issues/13871",
            ],
            "priority_reason": "AUDIT-7 Lane 7 continuation slices ordered by the lane brief; exhaustive stray-work hygiene for the four repos",
            "dedupe": {
                "basis": "Singular ownership: Lane 7 tracking issue code#13874 is the only stray-work audit; batches 1-2 done by Lane 7 session 2026-11-23; these slices cover disjoint remaining sets (A: text-linked, B: orphans, C: forks/fleet)",
                "checked_at": "2026-11-24T00:40:00Z",
            },
            "evidence": [
                "Lane 7 census on code#13874 slice-1 comment: 18,594 items in window; 1,405 open; code open issues 587 parent-labelled / 521 text-linked / 91 orphaned"
            ],
        },
    }
    json.dump(
        p, open(f"/home/hatch/workspace/lane7-audit7/packet-{tag}.json", "w"), indent=1
    )
    print("wrote", tag)
