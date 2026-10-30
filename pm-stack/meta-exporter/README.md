# Meta quota exporter — Phase 1 (code#1301)

Laptop-side exporter for the Meta "Muse Code High Usage" plan quota,
built on the #1167 investigation (report: usage-billing-hub goal,
`hidden_files/meta-console-investigation-2026-10-26.md`). Part of the
Stage-2 quota plane (epic #1149); the sandbox gatherer is #1151.

## Why laptop-side

The dev.meta.ai console is cookie-auth only; the session
(`llama_dev_sess`, expires 2027-10-27) lives in the laptop Helium
browser and Meta exposes a `sessionBindingId`, so Phase 1 replays the
console's own API calls **from the laptop session's own context**:
the exporter talks CDP (127.0.0.1:19222 on the laptop) and evaluates
`fetch()` inside the open dev.meta.ai page. Requests therefore carry
the session cookie and leave from the laptop's egress/device. The
cookie value never enters the exporter process, a file, or a log.

Endpoints read per tick (both cookie-auth GETs, JSON):

- `/api/portal/teams/{team_id}/subscription-quota` — both windows
  (weighted used/limit + reset epochs; % = used/limit).
- `/api/portal/teams/{team_id}/billing-banner` — remaining free
  credits (minor units).
- Best-effort: the PAYG "Current balance" is read from the billing
  iframe's DOM text when a billing tab is open (the balance itself is
  behind a GraphQL/embed flow). Absent tab → `paygBalanceUsd: null`,
  never an invented 0.

## Export format (the gatherer file-export convention)

The gatherer (#1151) had no file-export ingest convention yet, so
this exporter defines it; #1151 should adopt it as-is:

- Location (on the laptop):
  `~/atlas/meta-quota-export/latest.json` (written atomically,
  tmp + rename) plus an append-only `history.jsonl` of every run.
  The sandbox side reads it over the existing multiplexed SSH channel
  (`~/tools/laptop-ssh.sh 'cat ~/atlas/meta-quota-export/latest.json'`).
- Success document:

```json
{
  "service": "meta",
  "source": "file-export/meta-laptop",
  "status": "ok",
  "measuredAt": "<ISO run time>",
  "consoleAsOf": "<ISO from payload as_of>",
  "tier": "Muse Code High Usage",
  "windows": [
    { "name": "5h", "usedPct": 1.02, "resetAt": "<ISO>" },
    { "name": "weekly", "usedPct": 8.83, "resetAt": "<ISO>" }
  ],
  "freeCreditsUsd": 148.57,
  "paygBalanceUsd": 0
}
```

A window with no usage in it has not started yet and has no reset:
the console returns its `*_resets_at` as null or omits the field, and
the exporter emits that window with `"resetAt": null` instead of
failing the whole export (#1301, the 2026-10-29 regression). A missing
reset on a window WITH recorded usage — and any malformed reset
value — still fails closed as `unparseable`.

## Failure states (fail-closed)

A failed tick writes a failure document to `latest.json` (and
`history.jsonl`) with `status`, `attemptedAt`, `detail`, and
`measuredAt: null` — and **no** windows or values, so nothing stale
can render as a current reading:

| status            | meaning                                                                         | exit |
| ----------------- | ------------------------------------------------------------------------------- | ---- |
| `session-expired` | API answered 401/403 or bounced to auth — laptop session needs a human re-login | 3    |
| `cdp-unavailable` | browser debug port did not answer                                               | 4    |
| `no-console-page` | browser up, no dev.meta.ai tab open                                             | 5    |
| `unparseable`     | endpoint answered an unrecognised shape                                         | 6    |
| `fetch-failed`    | network/other error inside the page                                             | 7    |

`laptop-unreachable` is a consumer-side derivation, by design: if the
laptop is down nothing runs, so the gatherer/hub treats a
`latest.json` older than 2 ticks (> 30 min) as laptop-unreachable.

## Install / schedule (laptop)

```sh
bash pm-stack/meta-exporter/install-laptop.sh   # run ON the laptop
```

Installs the script to `~/atlas/meta-quota-export/`, renders
`meta-quota-export.service` (with the laptop's node path) +
`meta-quota-export.timer` into `~/.config/systemd/user/`, and enables
a user timer firing every 15 minutes (`OnCalendar=*:0/15`,
`Persistent=true`). Note: user timers run while the user has a
session; `loginctl enable-linger user` (needs sudo) would make them
fully boot-persistent — the laptop's graphical session is the normal
state, and staleness is visible via the age rule above either way.

Manual run / debugging:

```sh
node ~/atlas/meta-quota-export/meta-quota-export.mjs
journalctl --user -u meta-quota-export.service
```

## Tests

```sh
node --test pm-stack/meta-exporter/meta-quota-export.test.mjs
```
