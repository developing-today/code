# S11: relay operation (iroh-relay 1.3.0)

Local only. Nothing deployed, no public address used, no nix, no host network change. All relays and clients ran on 127.0.0.1 in ports 19101–19251. A leftover `iroh-relay` on `/tmp/c2-run.OUtvGR` is another spike's process, and I left it alone.

## Question

1. Restart the relay with SIGTERM and with SIGKILL while two peers have an open connection homed on it. How long until both peers reconnect? Does the QUIC connection survive? Is an in-flight message lost? (median of 3)
2. Relay key rotation: what does a client with the old relay URL or ticket see? Is the relay key pinned?
3. TLS certificate rotation: does the running relay pick up a replaced certificate, or does it need a restart?
4. Idle behaviour: does a non-home relay connection close after 60 s idle? What does a home relay do?
5. Operations: metrics, log levels, and a minimal health check.

## Setup

- Relay: `/home/user/work/spikes/tools/bin/iroh-relay`, `--version` 1.3.0, sha256 `2f6f6d0e…`. Its source is `iroh-relay-1.3.0` in the cargo registry, which I read.
- Configs in `config/`. `config/tunnel.kit.toml` is a verbatim copy of the kit's `tunnel.toml` (sha256 `4c3f8c…`, matching the kit). `config/local.toml` is the same file with ports changed to 19101/19201. The other configs are derived the same way.
- Driver: `s11-relay/src/main.rs`, built with the pinned toolchain 1.97.0 into `/home/user/work/spikes/target`. First build 1m27s, rebuilds 9–105 s. `Cargo.lock` is copied from the spikes workspace and pruned by cargo to 416 packages. The pinned crates are unchanged (iroh, iroh-relay, tokio, libc, tracing, rustls-pki-types).
- Endpoints are relay-only: `Endpoint::builder(Minimal).clear_ip_transports()` and `RelayConfig::new(url, None)`, so QUIC address discovery is off. This matches the Tunnel shape. The probe data flow is a 1 KiB echo frame with a sequence number, sent every 10 ms, plus a 2 MiB unpaced burst just before each kill.
- PKI: two test CAs (CA1, CA2), and leaves A and B from CA1 and C from CA2, all EC P-256 and valid for localhost and 127.0.0.1.

## Method and commands

```
export CARGO_TARGET_DIR=/home/user/work/spikes/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"; "$CARGO" build
cd /home/user/work/spikes-next/s11-relay
S=/home/user/work/spikes/target/debug/s11
$S restart TERM 3 1000      # scenario 1, 1 s outage, 3 runs
$S restart KILL 3 1000
$S restart INT 3 1000       # control: the one graceful signal
$S restart KILL 3 3000      # outage equal to the kit's RestartSec=3
RUST_LOG='s11=info,iroh::socket::transports::relay=debug,iroh::_events::relay=debug,warn' \
  $S restart KILL 1 35000 2> logs/restart-SIGKILL-35000ms-driver.log
RUST_LOG='s11=info,iroh::socket::transports::relay=debug,iroh::_events::relay=debug,warn' \
  $S idle 2> logs/idle-driver.log                 # scenario 4, about 4 min
runbook/poll-metrics.sh 300 19211 19212 > logs/idle-metrics.log
runbook/relay-ctl.sh ...                          # scenarios 2, 3 and 5, see below
```

Each `restart` run: relay on 19101; two relay-only endpoints A and B; 3 s of steady echoes; a 2 MiB burst; kill with signal; wait for exit; sleep `down`; restart the relay on the same port; watch until an echo arrives after the relay is back and all in-flight frames have settled, or 10 s after that first echo. If the QUIC connection closes, the driver re-dials and retries every second.

Two measurement fixes were made before the reported runs. The first version measured "first echo" from the moment the relay died, which caught a straggler echo of 4 ms. It now measures from the moment the relay is listening again. The re-dial retry loop was added after the 35 s run failed, and the survived path is unchanged by it. The 1 s and 3 s results come from the version before that change.

## Results

### 1. Restart under two connected peers

All times are milliseconds from the signal. Medians of 3 runs; the range is in brackets where it matters. "Home back" is when each endpoint's home-relay status returns to connected after it dropped.

| Case | Relay exit | Relay listening | Home A back | Home B back | First echo after relay up | QUIC survived | In flight at kill (median) | Lost |
|---|---|---|---|---|---|---|---|---|
| SIGTERM, 1 s outage | 3.2 (signal 15) | 1008 | 1156 | 1630 | 1909 (1731–2011) | 3/3 | 1177 | 0 |
| SIGKILL, 1 s outage | 3.4 (signal 9) | 1012 | 1646 | 1611 | 1834 (1061–4025) | 3/3 | 1130 | 0 |
| SIGINT (control), 1 s | 4.3 (exit 0) | 1013 | 1615 | 1740 | 1745 (1485–1747) | 3/3 | 1120 | 0 |
| SIGKILL, 3 s outage | 7.4 (signal 9) | 3032 | 3628 | 4020 | 5613 (4022–7475) | 3/3 | 1062 | 0 |
| SIGKILL, 35 s outage, 1 run | 7.1 | 35075 | 60636 | 44726 | 60771 | **0/1: timed out at 35068** | 1062 | **1058** |

Per-run raw values are in `logs/restart-<SIG>-<down>ms-summary.json`.

What the 35 s run shows, from `logs/restart-SIGKILL-35000ms-driver.log`:
- The relay was listening again at +35.1 s.
- The QUIC connection closed with reason "timed out" at +35.07 s, so the connection died at about the time the relay came back.
- The re-dial at +35.1 s failed with "connect timed out" after 20 s. The endpoint's relay actor was still in backoff. The log shows `retry in 27.41s` at +33.2 s, so its next attempt came at +60.6 s.
- B's home returned at +44.7 s, and A's at +60.6 s. The re-dial succeeded at +60.6 s, and the first echo came at +60.8 s.

### 2. Relay key rotation and trust

- `iroh-relay` 1.3.0 has no relay key. `Config` (main.rs:92) has no key field. The server only sends a random challenge; the client signs it (handshake.rs:200–230). The relay therefore has no identity that a client can verify except its TLS certificate. The comment at handshake.rs:224 ("signature … from the server") is a copy-paste error.
- `RelayConfig` holds `url`, `quic`, and `auth_token` only (relay_map.rs). It has no pin field. The client verifies TLS with `CaTlsConfig` (default: embedded Mozilla roots, tls.rs:57), and the server name is the URL host (client/tls.rs:107–110).
- `secret_key` is accepted and silently ignored. `legacy-secret-key.toml` started and answered `/ping` 200 with no warning. `Config` has no `deny_unknown_fields`.

| Step | Result |
|---|---|
| Relay serves leaf A (CA1). Dial with `--ca ca1.der` | ONLINE in 268 ms |
| Rotate to leaf B: new key, same CA1. Restart. Same dial | ONLINE in 268 ms. No client change needed |
| Rotate to leaf C from CA2. Restart. Dial with CA1 only | NOT_ONLINE after 8 s. Status list empty, so no error text in the API |
| Relay log for that case | `TLS[manual] accept … received fatal alert: UnknownCA` (ERROR) |
| Dial with CA1 + CA2 | ONLINE in 263 ms |
| Dial with CA2 only | ONLINE in 212 ms |
| Dial with default WebPKI roots | NOT_ONLINE after 8 s. The private CA is not in those roots |

Verdict: not pinned. Clients trust whatever roots their `CaTlsConfig` names, and the URL must match the certificate's name. A key rotation under the same CA is invisible to clients. A change of CA breaks any client that pins a CA. A ticket holds only the relay URL, so an old ticket keeps working exactly when the URL still passes TLS. I did not test a real ticket.

### 3. TLS certificate rotation

| Mode | Step | Served serial |
|---|---|---|
| Manual | Start with leaf A | …FD (A) |
| Manual | Replace the files on disk, wait 3 s, no restart | …FD (A). Not picked up |
| Manual | Restart | …FE (B) |
| Manual | `rotate-cert` to CA2 leaf C, with backup, validation, and restart | …292F (C) |
| Reloading | Start with leaf A. Log: `Reloaded the certificate` at once | …FD (A) |
| Reloading | Replace the files, wait 5 s | …FD (A). Not picked up |
| Reloading | Restart | …FE (B) |

- Manual loads the certificate once at startup (main.rs:623–638). A restart is required.
- Reloading (`cert_mode = "Reloading"`, undocumented in the kit) reloads on a fixed 24 h interval (`DEFAULT_CERT_RELOAD_INTERVAL = 86400 s`, server/resolver.rs:19). The first tick fires at startup, which is the log line seen. The 24 h wait itself was not measured.
- Source only: server/resolver.rs calls `let _ = reload()` and then logs "Reloaded the certificate" whether or not the reload succeeded. A bad file would be silently ignored, while the log says it was reloaded.

Verdict: a running relay needs a restart to pick up a new certificate. Reloading mode does not react to file changes at all within the first day.

### 4. Idle behaviour (scenario 4, one run, 3 min 40 s)

Relay 19111 was A's non-home relay, and B was homed on it. Relay 19112 was A's home. The timeline is from `logs/idle-driver.log`:

| Time | Event |
|---|---|
| 12:19:40.70 | A homed on 19112, B homed on 19111, so A reaches B through the non-home 19111 |
| 12:19:40.73 | One frame echoed. No application traffic after this point |
| 12:20:50.74 | QUIC still open after 70 s, with 5 s keep-alives. Then `conn.close()` |
| 12:21:50.74 | `Inactive for 60s, exiting (running)` on the 19111 actor. Exactly 60.0 s after the close |
| 12:23:20.74 | The home relay (19112) had one status transition over the whole run: connected |

- Non-home: the connection stayed up for the full 70 s, because keep-alives reset the timer. It closed 60 s after the last datagram.
- Home: the status watcher recorded no disconnect in 3 min 40 s, and no inactivity exit for 19112. Its pings were not sampled (see below).
- Relay side: 19111's `send_packets_sent_total` rose to 79 and stopped at the close. Its log shows `WARN … actor errored Stream terminated` at 12:21:50.74.

Verdict: matches the connectivity design. A non-home connection closes 60 s after the last datagram it sends, but a live QUIC connection keeps it open. The home relay never closes for idleness.

### 5. Operations

- **Metrics**: `metrics_bind_addr` defaults to the `http_bind_addr` IP with port 9090. That is `[::]:9090`, public, if the bind is public (main.rs:344–347). The kit sets loopback. The server answers Prometheus text on any path: `/metrics`, `/`, `/ping`, `/healthz` all returned 200 with the same body. Counter names include `relayserver_accepts_total`, `relayserver_disconnects_total`, `relayserver_got_ping_total`, `relayserver_sent_pong_total`, and `relayserver_send_packets_sent_total`.
- **Log levels** (`RUST_LOG`, 3 s, no clients): unset gives 0 lines. `info` gives 2 lines (metrics start and "serving on"). `debug` gives 23 lines. Each client disconnect logs a WARN. Config parse errors exit 1 with a message even when `RUST_LOG` is unset: `Error: config must be valid toml`.
- **Health**: `/ping` returns 200 with an empty body, in plain and TLS mode. It is not protected by `shared_token`. `/healthz` returns `{"status":"ok","version":"1.3.0","git_hash":"unknown"}` with 200, verified over TLS against CA2. `/generate_204` returns 204 on the plain port. In TLS mode the captive portal is on `http_bind_addr`, which I did not test.
- **shared_token** (`access.shared_token = [...]`, main.rs:176–196): no token gives NOT_ONLINE with `auth_denied = "not authorized"`. A wrong token gives the same. The right token gives ONLINE in 216 ms.
- **Missing config file**: `--config-path /nonexistent` starts silently with defaults, run in a network namespace with `RUST_LOG=info`. Log: `serving on [::]:80` and `metrics on [::]:9090`. Plain HTTP on every interface, with no warning. `Config::load` returns the default when the file is absent (main.rs:541–547).
- **Typo key**: `enable_quic_adr_discovery` is ignored with no warning. The relay starts.

## Runbook (commands run here)

`runbook/relay-ctl.sh` (all subcommands were run):

- Restart: `runbook/relay-ctl.sh restart <config> <pidfile> <log> <base-url>`. It sends SIGINT and waits for the process to exit, then starts it again and waits for `/ping`. SIGINT is the graceful path; SIGTERM is not handled (see surprises).
- Rotate the key and certificate: `CA=<ca.pem> runbook/relay-ctl.sh rotate-cert <new.pem> <new.key> <live.pem> <live.key> <config> <pidfile> <log> <base-url> [ca.pem]`. It checks that the key matches the certificate, checks the chain, backs up the old files, installs the new ones with the right modes, restarts, and prints the served serial. Verified with the CA2 leaf.
- Health: `CA=<ca.pem> runbook/relay-ctl.sh health <base-url>` (TLS verified). Without `CA`, it uses `curl -k` and says so.
- Metrics: `runbook/poll-metrics.sh <seconds> <port>...`.
- No separate "rotate key" exists in 1.3.0. The only key is the TLS private key, so rotating the key means `rotate-cert` with a new key. Remove any `secret_key` line from old configs, since it is ignored.

## What was NOT tested, and why

- **Real networks**: everything ran on loopback, with no public relay, Cloudflare Tunnel, VPS, or systemd. Deploying was out of scope.
- **systemd KillSignal**: not run. The kit's unit uses the default `KillSignal=SIGTERM`, which I observed to hard-kill the relay. `KillSignal=SIGINT` is recommended from the SIGINT result, but that is unverified under systemd.
- **Outage length boundary**: only 1 s, 3 s, and 35 s, and 35 s was one run. The point where the connection dies is somewhere between 3 s and 35 s, and I did not bisect it.
- **Home relay pings**: the counters were not sampled. The 15 s ping interval (`PING_INTERVAL` in actor.rs) is from source.
- **24 h cert reload**: source only. The logging issue on a failed reload is also source only.
- **Real tickets and holepunching**: a ticket is just the relay URL plus the endpoint ID, from source. Direct paths were disabled, so nothing about NAT traversal was measured.
- **`IROH_RELAY_ACCESS_TOKEN`, allowlists, HTTP access, rate limits**: not exercised.
- **Application semantics**: "lost" means the QUIC stream echo, not the owner's protocol messages.

## Verdicts

1. **Restart (SIGTERM and SIGKILL)**: the QUIC connection survives outages up to 3 s (3/3 runs each). When it survives, no in-flight frame is lost (about 1,100 in flight at each kill, 0 lost). With a 1 s outage the peers' home relays are back a median 1.2–1.7 s after the kill (single runs 1.0–2.2 s), and application traffic resumes a median 1.7–1.9 s after the kill. After a 3 s outage application traffic takes a median 5.6 s. SIGTERM and SIGKILL are equivalent here, since the relay does not handle SIGTERM. A 35 s outage kills the connection, and the peers return 9.7 s (B) and 25.6 s (A) after the relay is back, because of the reconnect backoff.
2. **Key**: no relay key in 1.3.0. Not pinned. A same-CA key change is transparent. A CA change breaks CA-pinned clients, and their status API shows no error.
3. **Certificate**: needs a restart in Manual and in Reloading mode, within the first 24 h.
4. **Idle**: non-home closes exactly 60 s after its last datagram, and stays open while QUIC keep-alives run. Home never idles out.
5. **Ops**: metrics default to a public port, logs are silent by default, health endpoints work, and token auth works. Two config mistakes fail silently.

## Surprises

- SIGTERM is not handled. `tokio::signal::ctrl_c()` listens only for SIGINT (main.rs:607), so SIGTERM is a hard kill. The kit's systemd unit would hard-kill on every stop. SIGINT exits 0.
- A missing `--config-path` file, or a misspelled key, starts silently with defaults. The default is plain HTTP on all interfaces with public metrics.
- Reconnect backoff is the long pole after a long outage. The retry delay reached 27.4 s, above the 16 s cap, because backon 1.6.0 adds jitter of up to the current delay on top of it (backoff.rs docs). The application cannot reset this timer.
- A TLS failure is invisible to the client's status API. Only the relay's log shows `UnknownCA`.
- The relay writes a WARN for every client disconnect, so it is noisy at `RUST_LOG=info`.
