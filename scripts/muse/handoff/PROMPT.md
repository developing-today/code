# New Muse lane: Fleet enrollment and first work cycle

You are the new Muse lane **<AGENT_NAME>**. Use the one-time credential URL
below with the enrollment helper. The URL contains a short-lived claim code,
not the OP/BWS values. Do not copy token values into chat, command arguments,
logs, or repo files.

One-time URL: <HANDOFF_URL>

## Start

```bash
curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/scripts/muse/handoff/enroll-agent.py -o /tmp/enroll-agent.py
python3 /tmp/enroll-agent.py --name <AGENT_NAME> --handoff-url '<HANDOFF_URL>'
```

The helper installs/restores the Muse tools and the bridge-capable Fleet binary,
persists both credentials with mode 0600, verifies the bootstrap, enrolls this
lane, and runs `fleet agent doctor --agent <AGENT_NAME>`.

## Completion criteria

Do not report enrollment complete until all of these are true:

1. Bootstrap prints `check: all good`, including a working Fleet binary,
   BWS secret resolution, OP service-account check, and bridge registration
   under **<AGENT_NAME>**.
2. `~/tools/fleet agent enroll --kind muse --name <AGENT_NAME>` succeeds or
   the helper confirms the matching profile already exists.
3. `~/tools/fleet agent doctor --agent <AGENT_NAME>` reports at least one
   usable intake rung. Record each failed leg, its stated dependency, and the
   exact non-secret error.
4. Start work with `~/tools/fleet agent next`. Follow the returned task and
   its completion commands. Exit 5 means no task was available this pass;
   report that state and check again only when new work is expected.

## If any command fails

Do not stop at the first script error or claim success from a partial setup.
Read the failed phase and error, identify whether the problem is missing tooling,
network/reachability, credentials, or a script defect, then:

- If bootstrap failed, keep the same agent name and resume using the command
  printed by the helper:
  `python3 /tmp/enroll-agent.py --name <AGENT_NAME> --resume`.
- If a helper or bootstrap defect remains, make the smallest correction in the
  code repo on a task branch, preserve the failed command and non-secret output,
  and rerun the narrowest relevant check.
- Do not print, echo, or commit OP/BWS token values. Never paste them into a
  report. A new one-time URL is needed only if the helper failed before saving
  the two 0600 files.
- Save your report at
  `scripts/muse/feedback/YYYY-MM-DD-<AGENT_NAME>.md` using
  `scripts/muse/feedback/TEMPLATE.md`. Include what failed, the root cause,
  recovery, script changes, verification, and any remaining blocked rung.
- Keep working after documenting a blocker when other safe setup or Fleet tasks
  remain. Do not wait for the owner to choose routine recovery steps.

When finished, report the lane name, successful checks, any unavailable intake
rungs, the first task disposition, the feedback file path, and the PR or branch
containing script changes.
