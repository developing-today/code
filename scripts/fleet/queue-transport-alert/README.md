# Queue transport alert (`queue-transport-alert`)

Implements code#6369: an outage alert for the repo-queue broker
transport. When the broker is unreachable every worker lane exits at
its Step 0 `repo-queue health` gate, by design, so a total queue outage
used to produce no signal at all (#5708 on 2026-11-04 was filed by
hand, and the 2026-11-18 evening outage, with the broker host's
Tailscale stopped during a rebuild, was only noticed from lane reports).

## What it does

`check_transport.py` runs one probe per pass and keeps a small state
file (`~/.local/state/queue-transport-alert/state.json` by default):

- **Probe:** `fleet queue health` by default, `--probe-cmd` to change
  it, or `--tcp host:port` for a bare connect that needs no fleet
  binary. Bounded by `--probe-timeout` (60s). It never uses the queue
  it watches.
- **Alert:** on `--threshold` consecutive failures (default 2) it alerts
  once per outage window. With `--apply` it searches open issues whose
  title starts `Queue transport incident:` and comments on that one if
  it exists, and otherwise files exactly one, labelled `kind/bug`,
  `priority/high`, `area/infra`, `source/fleet-monitor`.
- **Recovery:** the first passing probe after an alerted window comments
  once with the outage span. It never closes the incident (automation
  may label and comment but must not close an issue, #10136); a human
  closes it.
- A failed window that recovers below the threshold ends quietly.

Every pass prints one JSON document. Its `message` field is the one
line to deliver to the main chat: empty when nothing changed, the alert
line on the alerting pass, the recovery line on the recovering pass.
`--format text` prints only that line, so a scheduled run can relay its
stdout as is.

## Running it

```bash
# Observe-only (default): keeps the state file, writes nothing to GitHub
python3 check_transport.py

# File/comment on GitHub as the current gh login
python3 check_transport.py --apply

# Probe the broker listener directly, print only the chat line
python3 check_transport.py --tcp 100.99.113.103:4801 --format text --apply
```

Exit status is 0 on a completed pass whatever the probe found, 1 when
an `--apply` GitHub write failed (the window stays un-alerted, so the
next pass retries the write), and 2 on a usage error.

## Wiring

Two placements cover different failures, and both can run at once
because the incident search dedupes across them:

- **Off the broker host** (a scheduled sandbox run or another tailnet
  machine), every 10 minutes, `--apply --format text`, relaying stdout
  to the main chat. This is the only placement that sees the broker host
  itself being off.
- **On the broker host**, `systemd/queue-transport-alert.{service,timer}`
  as user units. They probe the Tailscale listener with `--tcp`, which
  catches tailscaled or the broker being stopped while the host is up,
  as on 2026-11-18. Observe-only until
  `~/.config/queue-transport-alert/mode` reads `apply`.

## Tests

```bash
python3 -I -m unittest discover -s . -p 'test_check_transport.py' -v
```

Offline: probe results and the clock are pinned, and GitHub is a
recording fake. Covered: one incident per window, dedupe onto an open
incident, quiet continuing outage, one recovery note and no close,
quiet sub-threshold blips, retry after a failed write,
observe mode, corrupt state, and the command/TCP probes.
