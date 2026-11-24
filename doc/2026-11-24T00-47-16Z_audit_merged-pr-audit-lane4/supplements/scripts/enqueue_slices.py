import json, subprocess, os, sys

os.environ["PATH"] = (
    os.path.expanduser("~/.local/share/hatch-recovery/bin") + ":" + os.environ["PATH"]
)
BASE = "/home/hatch/workspace/lane4-audit7"
REPOS = [
    ("developing-today/code", f"{BASE}/census-developing-today_code.json"),
    (
        "developing-today-forks/opencode",
        f"{BASE}/census-developing-today-forks_opencode.json",
    ),
    (
        "developing-today-forks/openchamber",
        f"{BASE}/census-developing-today-forks_openchamber.json",
    ),
]
BATCH = 250
results = []
for repo, fn in REPOS:
    nums = sorted(p["number"] for p in json.load(open(fn)))
    batches = [nums[i : i + BATCH] for i in range(0, len(nums), BATCH)]
    for bi, batch in enumerate(batches, 1):
        title = f"AUDIT-7 Lane 4 merged-PR audit slice: {repo} batch {bi}/{len(batches)} ({len(batch)} PRs)"
        body = (
            f"Lane 4 (MERGED-PR-AUDIT, tracking https://github.com/developing-today/code/issues/13879, umbrella https://github.com/developing-today/code/issues/13871) continuation slice.\n\n"
            f"AUDIT each of these merged PRs in {repo} (merged since 2026-10-23), ONE BY ONE, IN FULL: read the PR body and its complete diff (fleet gh api repos/{repo}/pulls/<n> and /files, per_page=100, count-confirmed), extract the spawning issue (especially Closes/closes/fixes references), read that issue in full, and judge: did the PR fully implement the issue? Check LATER merged PRs in the same repo before filing any gap — if a later PR completed it, record completed-later on both. Where a gap/bug/unfiled next step is real, FILE it in the owning repo (gh-post), prioritise it, and link it. NEVER use closing keywords in comments on issues that must stay open. Lane 2 owns reopen/correction decisions on closures — post suspect closures as pointers on the umbrella #13871, do not reopen. Apply the Lane 4 outcome labels (defined on #13879; create them in {repo} first if absent): audit:complete / audit:partial / audit:gap-filed / audit:bug-filed / audit:completed-later / audit:no-issue. Post the batch ledger (PR, spawning issue, verdict, filed links) as a comment on #13879 when the batch is done. Check #13879 and the umbrella for lane claims before acting on any item; a claim from another lane is a hard stop.\n\n"
            f"PR numbers in this batch: {batch}"
        )
        payload = {
            "version": 1,
            "kind": "feature_slice",
            "title": title,
            "repo": repo.split("/")[-1],
            "issue": {
                "number": 13879,
                "repo": "developing-today/code",
                "url": "https://github.com/developing-today/code/issues/13879",
            },
            "source": {"type": "issue", "issue": 13879},
            "work": {
                "summary": body,
                "acceptance": f"Ledger comment on code#13879 covering all {len(batch)} PRs of {repo} batch {bi} with verdicts, labels applied, and any gap/bug issues filed and linked.",
            },
            "slice": {
                "lane": "AUDIT-7 Lane 4",
                "repo": repo,
                "batch": bi,
                "batches": len(batches),
                "pr_numbers": batch,
            },
        }
        r = subprocess.run(
            [
                "fleet",
                "queue",
                "enqueue",
                "--queue",
                "repo-health-code-pilot",
                "--kind",
                "feature_slice",
                "--title",
                title,
                "--priority",
                "40",
                "--idempotency-key",
                f"audit7-lane4-{repo.replace('/', '-')}-b{bi}",
                "--payload-json",
                json.dumps(payload),
            ],
            capture_output=True,
            text=True,
            timeout=240,
        )
        out = (r.stdout + r.stderr).strip().replace("\n", " ")[-300:]
        print(repo, f"batch {bi}/{len(batches)}", "->", out, flush=True)
        results.append({"repo": repo, "batch": bi, "result": out})
json.dump(results, open(f"{BASE}/enqueue-results.json", "w"), indent=1)
print("DONE", len(results))
