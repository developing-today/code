# OpenCode Claimant/Executor

Slice-1 of #3026: claim/bind/launch/record for opencode-lane tasks.

## Files

- `claimant.py` – Main claimant/executor entry point
- `poll.py` – Candidate discovery and cap counting
- `__init__.py` – Package marker

## Usage

```bash
# Discovery only (shadow mode)
python3 pm-stack/opencode-claimant/poll.py

# Execute a fire (shadow or live)
python3 pm-stack/opencode-claimant/claimant.py
```

## Mode

Read from `~/.local/state/opencode-claimant/mode`:
- `shadow` (default) – report only, no mutations
- `live` – perform claims, binds, launches, and recoveries

Create the directory and file:
```bash
mkdir -p ~/.local/state/opencode-claimant
printf "shadow\n" > ~/.local/state/opencode-claimant/mode
```

## Fire Log

Appended to `~/.local/state/opencode-claimant/fires.jsonl` as JSON lines.

## Constants

- Queues: `repo-health-code-pilot`, `repo-health-opencode-pilot`, `repo-health-openchamber-pilot`, `repo-health-nix-pilot`
- Queue CLI: `~/fleet-brokers/bin/repo-queue --worker opencode-claimant`
- OpenCode server: `http://127.0.0.1:13420`
- Concurrency cap: 36 active root sessions
- Default route: `providerID: opencode-route`, `modelID: workhorse`, `agent: build`

## Hard Rules

- Never work a task yourself; only claim/bind/launch/record
- Never launch for tasks with hold markers (`held`, `user/owned`, `do-not-merge`, `do-not-auto-edit`)
- Never exceed the 36-session cap
- Never double-bind a task
- Never bind a session id you did not just create
- Atomic recovery/failure operations only
