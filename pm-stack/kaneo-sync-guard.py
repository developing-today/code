#!/usr/bin/env python3
"""Kaneo GitHub sync guard (close-authority program code#10136, slice #10137).

Asserts that every Kaneo GitHub integration's statusTransitions config can
never move a task into a closing state from a PR/branch event — the #7610
defect class. The expected map is declared HERE, in the repo, because the
setting otherwise lives only in Kaneo's Postgres, and both the integration
connect flow and the Automations UI can silently restore the dangerous
upstream default (onPRMerge = "done"; see pm-stack/KANEO-GITHUB-SYNC.md).

Behaviour: reads all type='github' rows from the integration table via
docker exec + psql against the pm-stack Postgres container. Conforming
config is a no-op. On drift it re-applies the safe map, prints DRIFT lines
with before/after, and exits 1 — the heal lands immediately, but callers
(deploy.sh, the hourly timer's health surface) still fail loudly so the
drift itself is never silent.

Run on the laptop (where the pm-stack containers live):
  python3 pm-stack/kaneo-sync-guard.py
"""

import json
import re
import subprocess
import sys

EXPECTED = {
    "onPROpen": "in-review",
    "onPRMerge": "in-review",
    "onBranchPush": "in-progress",
}
# A transition targeting the final workflow column is what closes the
# GitHub issue through Kaneo's status sync. No event-driven transition
# may ever point there; Done is reached only from a real close (the
# issue_closed fallback) or a human moving the task in Kaneo.
FORBIDDEN_TARGETS = {"done"}

CONTAINER = "pm-stack-postgres-1"
PSQL = [
    "docker",
    "exec",
    CONTAINER,
    "psql",
    "-U",
    "kaneo",
    "-d",
    "kaneo",
    "-tA",
    "-v",
    "ON_ERROR_STOP=1",
]


def psql(sql):
    out = subprocess.run(PSQL + ["-c", sql], capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"psql failed: {out.stderr.strip()[:300]}")
    return out.stdout.strip()


def main():
    rows = psql("SELECT id || E'\\t' || config FROM integration WHERE type='github'")
    if not rows:
        print("GUARD ERROR: no github integrations found")
        return 2
    drifted = 0
    for line in rows.splitlines():
        integ_id, _, raw = line.partition("\t")
        integ_id = integ_id.strip()
        if not re.fullmatch(r"[A-Za-z0-9_-]+", integ_id):
            print(f"GUARD ERROR: unexpected integration id {integ_id!r}")
            return 2
        cfg = json.loads(raw)
        repo = f"{cfg.get('repositoryOwner')}/{cfg.get('repositoryName')}" or integ_id
        st = cfg.get("statusTransitions") or {}
        problems = []
        for key, want in EXPECTED.items():
            if st.get(key) != want:
                problems.append(f"{key}={st.get(key)!r} (want {want!r})")
        for key, val in st.items():
            if val in FORBIDDEN_TARGETS and key not in EXPECTED:
                problems.append(f"{key} targets forbidden state {val!r}")
        if not problems:
            print(f"OK {repo}: statusTransitions conform")
            continue
        drifted += 1
        print(f"DRIFT {repo}: " + "; ".join(problems))
        healed = dict(st)
        healed.update(EXPECTED)
        for key in [
            k for k, v in healed.items() if v in FORBIDDEN_TARGETS and k not in EXPECTED
        ]:
            del healed[key]
        # Re-apply in SQL so the write is atomic per integration.
        # Persist the COMPLETE healed map computed above (#10528): setting
        # only the three expected keys would leave a forbidden transition
        # the heal deleted in memory still live in the stored config.
        # jsonb_set at the top-level path also creates statusTransitions
        # when it is missing entirely.
        healed_json = json.dumps(healed, sort_keys=True).replace("'", "''")
        psql(
            "UPDATE integration SET config = jsonb_set("
            "config::jsonb, '{statusTransitions}', "
            f"'{healed_json}'::jsonb)::text WHERE id='{integ_id}'"
        )
        after = json.loads(
            psql(f"SELECT config FROM integration WHERE id='{integ_id}'")
        )
        after_st = after.get("statusTransitions") or {}
        bad = [k for k, v in EXPECTED.items() if after_st.get(k) != v]
        bad += [
            f"{k} targets forbidden state {v!r}"
            for k, v in after_st.items()
            if v in FORBIDDEN_TARGETS and k not in EXPECTED
        ]
        if bad:
            print(f"GUARD ERROR {repo}: heal did not stick for {bad}")
            return 2
        print(f"HEALED {repo}: statusTransitions now {after_st}")
    if drifted:
        print(f"GUARD: {drifted} integration(s) had drifted and were healed")
        return 1
    print("GUARD: all integrations conform")
    return 0


if __name__ == "__main__":
    sys.exit(main())
