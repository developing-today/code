# lane-overlap-check — runbook

Same-lane overlap check for OpenChamber scheduled queue lanes
(`lane_overlap_check.py`, landed by PR #12165 for
[issue #11764](https://github.com/developing-today/code/issues/11764)).

## Why this exists

A scheduled lane must not claim queue work while an earlier occurrence
of the same lane is still running. Lanes used to answer that question
with `openchamber session list --dir /home/user/git/code --limit 250
--with-status --json`. That command requests the whole session list
through the OpenChamber control API, whose 4-second request budget the
250-row listing intermittently exceeds (`Request to
/api/openchamber/control timed out after 4000ms`, rc=1), and 250 rows
do not reliably reach back over the required 180-minute overlap
window. Every lane that hit the failure stopped before claiming, so
scheduled capacity sat idle (no queue item was ever lost — the lanes
stop cleanly).

This helper answers the same question per lane instead: it asks the
server for only the lane's own sessions (server-side title search,
cursor paging, newest first), stops paging once the listing is older
than the window, and reads each candidate's status with
`openchamber session status`, which answers in well under a second.

## Usage

```
lane_overlap_check.py <lane> [--dir /home/user/git/code]
                      [--self <session-id>] [--window-minutes 180]
                      [--server $OPENCHAMBER_URL|http://127.0.0.1:3000]
```

It prints one JSON verdict:

| Verdict   | Exit | Meaning                                            | Lane action |
|-----------|------|----------------------------------------------------|-------------|
| `clear`   | 0    | No earlier occurrence of this lane is running      | Claim       |
| `overlap` | 3    | An earlier occurrence is busy, retrying, or of unknown status | Stop   |
| `unknown` | 4    | The check could not be completed (`error` says why) | Stop and report |

## Decision: the overlap check is fail-closed

Recorded for #11764 (ask 2). When the check cannot produce an
authoritative verdict, the lane **stops and does not claim**. It never
fails open.

- Any failure while listing the lane's sessions — timeout, HTTP
  error, malformed response, paging that cannot reach the window —
  yields verdict `unknown`, exit 4.
- If the lane's own current occurrence is not found in the listing,
  the verdict is `unknown`, exit 4 (the check cannot distinguish this
  occurrence from a prior run).
- An earlier session whose status cannot be read counts as running:
  only an explicit `idle` status clears it, so an unreadable status
  yields `overlap`, exit 3, not `clear`.

Rationale: the failure mode fail-closed protects against is two
occurrences of the same lane claiming and working concurrently, which
can double-claim queue items and collide on shared state. The failure
mode it accepts is the one #11764 observed — idle scheduled capacity
for one wave — which loses no queue work and self-recovers on the
next wave. Idle capacity is recoverable; a double claim may not be.

## The `unknown-directory` case

Recorded for #11764 (ask 3). Sessions whose directory the text form of
`openchamber session list` renders as `unknown-directory` are a
display artifact of that text rendering: the JSON form of the same
listing populates `location.directory` for active sessions. The
overlap check never reads the text rendering — it consumes the JSON
API directly (`location.directory`, falling back to the session's
`directory` field).

A session with no directory in the JSON either is **not** filtered
out: it stays a candidate, and its status is checked using the lane's
`--dir`. An unknown directory therefore cannot hide a running earlier
occurrence from the check; at worst it makes the check conservative,
which is the fail-closed direction above.

## Post-merge observation

The fix merged 2026-11-21 (PR #12165). Post-merge evidence recorded on
#11764: on 2026-11-27 a live lane check returned `openchamber session
list --dir /home/user/git/code --limit 5 --with-status --json` with
rc=0 and `lane_overlap_check.py oc-code-02 --dir /home/user/git/code`
returned verdict `clear` (exit 0), and that run went on to claim queue
work without an overlap-check stop. No post-merge failure of the new
check is recorded on the issue; all recurrence evidence predates the
merge. Continued observation of scheduled waves remains tracked on
#11764.
