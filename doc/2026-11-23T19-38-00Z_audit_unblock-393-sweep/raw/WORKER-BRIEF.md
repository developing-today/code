# UNBLOCK-393 worker brief (shared — read fully before starting)

You are one worker in a coordinator-led sweep reviewing EVERY blocked fleet-queue task,
one by one, attempting to unblock each. Your slice: a file of task ids (path given in your
spawn message). Work ONLY your slice's ids, in order. Baseline snapshot (2026-11-23 ~13:20 CST):
393 blocked tasks.

## Tools / routes
- Queue (laptop-side client only; the sandbox queue client is down):
  `~/tools/laptop-ssh.sh '~/.local/bin/fleet queue <verb> ...'`
  - Inspect: `queue show <id>` — the `blocker:` field is the recorded block reason; `issue:` gives the linked issue.
  - Unblock: `queue unblock <id> --reason "<text>"` (add `--drop-dep kind:ref,...` when only named deps block and they have resolved; dropping the last dep releases the task).
  - Cancel: `queue cancel <id> --reason "<text>"` (--reason is required).
  - Repair (deterministic packet defects only): `queue repair <id> --dry-run` first; run without --dry-run only if the dry run shows a repairable defect.
  - Enqueue a corrected packet: `queue validate <packet-path>` then `queue enqueue <packet-path>` (packet file must exist ON THE LAPTOP).
  - No stdin piping through laptop-ssh (it fails). To put a packet file on the laptop, embed base64 in the command string: build the JSON locally, base64 it, then
    `~/tools/laptop-ssh.sh 'printf %s <B64> | base64 -d > ~/rt1-packets/unblock-<oldtaskid>.json'`.
    Packet templates/examples live on the laptop in `~/rt1-packets/` (RT1's p1–p10 files) — read one and mirror its repo-task/v1 warden envelope. NEVER set `payload.requires.platform` (makes packets unclaimable).
- GitHub reads: `gh api` REST first; on rate-limit/failure fall back to `fleet gh api`. Always capture stderr (never discard it); paginated comment reads need `--paginate` with a `.[]` jq shape or per_page=100 with a count check.
- GitHub comments (decision records): `~/tools/gh-post comment --repo <owner/repo> --issue <n> --body-file <file>` (App route) or `fleet gh api` POST. Keep comments short: decision + one-line why + "UNBLOCK-393 sweep".

## Per-task procedure
1. `queue show <id>`: note blocker text, linked issue (repo + number), queue, kind.
2. Read the linked issue's CURRENT state (issue state + newest comments, count-confirmed). If the blocker names a dependency (issue/PR/CI/task), check that dependency's current state too.
3. Choose EXACTLY ONE disposition and act:
   - **DONE** — the work already landed (fix merged / PR merged) or the issue is closed: `queue cancel <id> --reason "UNBLOCK-393 DONE: <evidence — PR/merge sha or issue state>"`.
   - **STALE BLOCK** — the named blocker has since landed/resolved and nothing else blocks: `queue unblock <id> --reason "UNBLOCK-393 STALE BLOCK: <what resolved + evidence>"` (use --drop-dep when the block is purely named deps that resolved).
   - **DEFECTIVE PACKET** — the packet itself is malformed (the known semantic packet_defect class: missing/invalid payload fields, off-scale values, missing dedupe): try `queue repair --dry-run`; if repairable, repair, then unblock if the underlying work is otherwise unblocked (else leave blocked and say why). If unrepairable: build a corrected packet mirroring the RT1 template (same issue, same scope, sane priority — fix=70, triage=10, slice=20 unless the original says otherwise), validate, enqueue, then `queue cancel <oldid> --reason "UNBLOCK-393 DEFECTIVE PACKET: replaced by task <newid>"`. Record old → new.
   - **DECISION-BLOCKED** — the only blocker is an undecided question/approval: DECIDE it yourself under the posture below, post the decision + one-line reasoning as an issue comment, then unblock with a reason citing the decision. Record the decision in your slice output (task, issue, decision, why).
   - **STILL BLOCKED** — a real, current blocker remains: NO mutation. Record the current reason (refreshed, not just the stale text) and the specific thing that would unblock it.
4. After every mutation, verify by read-back (`queue show <id>` status field, or the enqueue output's new task id) before recording the line.
5. Append ONE line per task to your slice output file IMMEDIATELY as you finish each task (so partial progress survives):
   `<task_id>\t<owner/repo#N or ->\t<DISPOSITION>\t<action taken / new task id / ->\t<evidence: PR, sha, comment id, or read-back status>\t<note: decision made, or unblock condition if STILL BLOCKED>`
   Disposition strings exactly: DONE | STALE BLOCK | DEFECTIVE PACKET | DECISION-BLOCKED | STILL BLOCKED.

## Decision posture (Drew, 2026-11-23 13:17 CST — applies to DECISION-BLOCKED only)
You may approve, ratify, or decide without asking. Bias: prefer options; prefer "both" over either/or; prefer the MORE CAPABLE option — not the most locked-down security option, not the most basic option; extend and improve. Record each decision on the issue.

## HARD LIMITS (the posture does NOT lift these)
- Never touch anything in the vertexinc org.
- Task 18533 must never change state (it is not in the blocked list — if you somehow encounter it, skip it entirely).
- NEVER unblock tasks linked to these standing hard stops — disposition STILL BLOCKED with the standing reason: code#1658, code#2490 (remainder), code#9608 (remainders), code#2140, code#13178, code#10910 (close decision pending).
- The routed flag set (tasks 21416–21428) is not yours to alter (none are in the blocked list).
- No PR merges, no code changes, no branch pushes — this sweep fixes QUEUE STATE only.
- A blocker that is "a Drew-authored PR is open" (e.g. task 20726 / darwin-nix PR #2011) stays STILL BLOCKED: merging/closing Drew's own PR is out of scope; the unblock condition is the PR merging or closing.
- Design-mill / Summit program tasks whose blocker is a genuine upstream dependency or an owner ratification gate stay STILL BLOCKED unless the blocker text is purely an undecided either/or you can decide under the posture.
- No external communications (no email/messages; GitHub comments on Drew's own repos for decision records are in-scope as specified).
- If a GitHub or queue operation fails twice on the same task, disposition it STILL BLOCKED with the failure as the recorded reason and move on — do not loop.

## Return
Your final message reports: counts per disposition for your slice; every DECISION-BLOCKED decision (task, issue, decision, why); every re-issued packet (old task → new task); and confirms your slice output file is complete (line count = slice size).
