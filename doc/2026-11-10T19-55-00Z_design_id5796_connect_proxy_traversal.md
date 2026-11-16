# Design: HTTP CONNECT proxy traversal for ID

- Date: 2026-11-10
- Status: **decision record** — no implementation is included in this change.
- Issue: [#5796](https://github.com/developing-today/code/issues/5796),
  "id: design: HTTP CONNECT proxy traversal (from #5671)".
- Parent design: [#5671](https://github.com/developing-today/code/issues/5671),
  "id: design: ID transport fallback for UDP-denied networks v1", whose
  decisions this record **does not re-decide** (see §1 and §12).
  Programme epic: [#2056](https://github.com/developing-today/code/issues/2056).
- Sibling records consumed, not amended:
  [#5791](https://github.com/developing-today/code/issues/5791) transport
  negotiation & fallback ordering
  (`doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md`) and
  [#5792](https://github.com/developing-today/code/issues/5792) broker /
  serve-proxy wire protocol (decision comment 2026-11-08T08:06:54Z;
  framing slice 1 landed as `pkgs/id/src/serve_proxy.rs`, issue #7569).
  Cloudflare Workers relay record
  (`doc/2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md`)
  explicitly routes networks that allow HTTPS but block WebSocket to
  _this_ tier, so this record is the supported path for them.
- Evidence baseline: `origin/main` at `7dc4aefa23`, worktree
  `~/git/fleet/12791-5796`, 2026-11-10. Verified at that baseline: no
  CONNECT/proxy client exists in the crate — `grep -riE
'https?_proxy|all_proxy|no_proxy|proxy-authorization'` over
  `pkgs/id/src` and `pkgs/id/crates/id-core/src` returns nothing — while
  the _server_ half that a CONNECT tunnel must reach already exists:
  `SSH_TUNNEL_ALPN` + `SshTunnelProtocol` (host-side SSH forwarding,
  `pkgs/id/src/world_ssh.rs:89,97`), `id world ssh-tunnel --listen`
  (local listener for an SSH client, `pkgs/id/src/cli.rs:1962-1977`),
  and the broker frame codec (`pkgs/id/src/serve_proxy.rs`).

### Gate status (was parked on a stale gate)

The packet that preceded this claim (task 12791, 2026-11-08) parked on
"parent #5671 has no decision record". That gate was **wrong on both
halves** and was corrected by triage task 12830 (comment
2026-11-08T12:47Z): #5671 is a decomposition umbrella whose four
decisions (fallback order, broker shape, trust model, E2E boundary) are
fixed **in its own body**, and the children are parallel designs against
that body — #5791, #5792 and #5798 already landed their records with no
parent synthesis record in existence. This record therefore proceeds
without one, and §12 checks the four decisions explicitly.

## 1. What the parent and siblings already fixed (not re-decided here)

Taken as given:

1. **Attempt order (#5671):** direct UDP → relay-assisted UDP → TCP →
   HTTP/HTTPS proxy (CONNECT, and where necessary plain HTTP /
   WebSocket-style framing). CONNECT is the **last tier**, not a
   preferred path.
2. **Broker shape (#5671, #5792):** a serve-proxy mode of the `id`
   binary, both legs outbound, TLS 1.3 over TCP 443 on each leg, one
   binary frame format (`pkgs/id/src/serve_proxy.rs`), dumb pipe holding
   no session or content keys.
3. **Trust model (#5671):** the broker sees connection metadata only,
   never content; authenticated by default, no open relay.
4. **E2E boundary (#5671, #5792 §6):** ID data/sync, queries/results and
   tunneled SSH stay end-to-end encrypted between the true endpoints;
   plaintext exists only at the true endpoints and at a locally
   terminated client-side `id` endpoint.
5. **Negotiation rules (#5791):** the failure **classes** of #5791 §6.1
   are the vocabulary every attempt returns; per-(peer, network) memory,
   the override precedence table (§5), quarantine (§6.3) and the "no
   silent downgrade" rule (§7.3) all apply to CONNECT attempts exactly
   as to UDP/TCP attempts.
6. **Broker leg framing (#5792 §2, §4):** CONNECT is how the client
   _reaches_ the broker when egress demands a proxy; once the tunnel
   exists, the frames and the 25 s PING keepalive are #5792's, unchanged.

This record decides only: **how ID establishes, authenticates, keeps and
tears down an HTTP CONNECT tunnel; where proxy credentials live; what a
corporate proxy that inspects traffic means for ID's assurances; and how
the locally terminated SSH-class endpoint is shaped on top of a CONNECT
tunnel.**

## 2. Decision summary (chosen shape)

1. **One CONNECT client, used by two callers.** A single CONNECT
   implementation inside `id` (a `connect-proxy` transport module, not a
   shell-out to `socat`/`ssh -o ProxyCommand`) establishes a byte tunnel
   to a _single_ named destination. Two callers use it: (a) the
   transport stack dialing a broker/host over TCP 443, and (b) the
   locally terminated endpoint (§7) dialing on behalf of a local SSH
   client. There is no second proxy code path.
2. **Egress proxy configuration is explicit-plus-environment, never
   guessed.** Proxy endpoint and credentials come from, in precedence
   order: per-invocation CLI flag → per-invocation environment
   (`HTTPS_PROXY`/`ALL_PROXY`, `NO_PROXY`) → per-peer config → global
   config → "no proxy" (attempt the direct TCP tier instead). This is
   #5791 §5's override table applied to one more knob; it adds no new
   precedence layer.
3. **Authentication is a credential _reference_, resolved at use, never
   a value in config or logs.** `Basic` is the v1 scheme. `407` is
   #5791's `proxy_auth_required` class: negotiation pauses for a
   credential once, and never falls through to a direct attempt as if
   the network were merely slow.
4. **The tunnel is treated as hostile-path plumbing, not as security.**
   TLS to the endpoint is established **inside** the CONNECT tunnel
   (TLS-in-TLS), end to end. A proxy that terminates and re-originates
   TLS is detectable only as a failed certificate check and is
   **surfaced, never worked around** (§6). ID never disables
   verification to make a corporate proxy happy.
5. **The locally terminated endpoint is `id`'s own listener on
   loopback**, carrying SSH-class bytes to a tunnel established with the
   same CONNECT client. This generalizes the existing `id world
ssh-tunnel --listen` shape rather than inventing a second local proxy.
6. **Keepalive and reuse follow #5792's numbers**, because the tunnel
   carries #5792 frames: PING every 25 s with jitter, dead leg after 3
   unanswered, 120 s broker-side idle close. A CONNECT tunnel that a
   corporate proxy or NAT kills at ~60 s idle must never be the reason a
   world session dies — §4.4.
7. **Diagnostics name the layer.** A CONNECT failure is reported as the
   proxy hop (`proxy 10.0.0.5:3128 — 407 auth required`) distinct from
   the far end (`broker b1.example:443 — timeout`), so support can tell
   "corporate egress" from "peer offline" without a packet capture.

## 3. Egress detection and proxy discovery

### 3.1 Sources, in precedence order

| Source                                   | Scope          | Notes                                                                                                                                                        |
| ---------------------------------------- | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| CLI `--proxy URL` / `--no-proxy`         | per invocation | wins over everything; `--no-proxy` is a forbid                                                                                                               |
| `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY` | per invocation | the same variables the fleet's own sandbox tooling already honours (`docs/2026-11-04_remote-gui-access-runbook.md` uses `$HTTPS_PROXY` with a port override) |
| per-peer config                          | one peer       | pinned proxy for a peer reachable only through one egress                                                                                                    |
| global config                            | all peers      | the machine's usual proxy                                                                                                                                    |
| none                                     | —              | dial the destination directly (the TCP tier), no CONNECT                                                                                                     |

`NO_PROXY` matching is by host suffix and literal IP, as browsers and
`curl` implement it, and it is honoured for both callers. A credential
in a proxy URL (`http://user:pass@host:port`) is **accepted for
compatibility but immediately moved to a reference** (§5.2) and the URL
value is never echoed.

### 3.2 Proxy auto-config (PAC) and WPAD

PAC/WPAD is **rejected for v1** and recorded as a follow-up question
(§13), not silently ignored: it means executing JavaScript to learn an
egress policy, which is a scripting surface inside a network client, and
in the motivating environment (a sandbox with one known CONNECT proxy)
it buys nothing. A user on a PAC-governed corporate network supplies the
resolved proxy explicitly. If PAC is ever adopted it must be a
constrained, non-Turing-complete resolver with the same credential
rules, and it must not become the only way to configure a proxy.

### 3.3 No probing theatre

ID does not "detect" a proxy by trying a direct connection and
interpreting a timeout as proxy evidence. If no proxy is configured and
the direct tier fails, the failure class is #5791's
(`udp_blocked` / `timeout` / `transport_error`) and the error text names
the absence of proxy configuration as a probable cause, once. Guessing
an egress proxy is how clients leak direct attempts to networks that
watch for them.

## 4. Establishing the tunnel

### 4.1 Request shape

```
CONNECT <host>:<port> HTTP/1.1
Host: <host>:<port>
Proxy-Connection: keep-alive        (compatibility only; ignored by modern proxies)
Proxy-Authorization: Basic <b64>    (only when a credential is resolved)
User-Agent: id/<version>
```

- The destination is always `host:port` of the **broker or host
  endpoint** in the current candidate (#5791 §3.2), never a wildcard and
  never a sequence of destinations on one tunnel. One tunnel = one
  destination = one candidate, so failure attribution stays per
  candidate.
- `Proxy-Authorization` is emitted only after a `407` (§5.3) or when the
  configuration pre-authorizes; it never rides a request whose challenge
  has not been seen unless the operator pinned it, in which case that is
  the operator's stated intent and is logged as such (metadata only).
- The response is parsed only as far as needed: status line, the
  `Proxy-Authenticate` challenge on 407, and any `Content-Length` body
  to drain. A `200` (or the historical `200 Connection established`)
  ends the HTTP layer; everything after is the tunnel.
- **No HTTP/1.1 upgrade, no chunked body, no CONNECT-with-body
  extension.** The tunnel starts at the first byte after the response
  headers.

### 4.2 What rides inside

- **To a broker:** TLS 1.3 to the broker, then #5792 frames. This is
  TLS-in-TLS when the proxy itself is reached over TLS (rare;
  `https://` proxy URLs are supported and treated as TLS-to-proxy, then
  CONNECT inside it).
- **To a host endpoint:** TLS to the host, then ID's normal handshake —
  the same handshake a direct TCP tier attempt performs, which is what
  makes #5791 §3.1 ("a probe is a real handshake") true on this tier.
- **SSH-class traffic** (§7) reaches a broker and is forwarded per
  #5792; the SSH authentication happens inside, unchanged.

### 4.3 Deadlines

- The CONNECT exchange has its own deadline, separate from the TLS
  handshake that follows and from the ID handshake after that: **CONNECT
  request→response**, then **TLS**, then **handshake**. A slow corporate
  proxy that accepts TCP and then hangs produces a CONNECT-hop timeout
  attributed to the proxy, not a handshake timeout attributed to the
  peer.
- Numbers are the latency design's (#5671 item 7 / #5797); this record
  fixes only that the three phases are separately timed and separately
  classified, and that a CONNECT attempt is _one_ candidate attempt for
  #5791 §3.3's overall budget.

### 4.4 Keepalive through an idle-killing proxy

Corporate proxies and NATs close idle tunnels aggressively; #5792 assumed
a 60 s floor for the broker leg. On the CONNECT tier the same floor
applies one layer lower, so: **the tunnel carries application keepalive,
not TCP keepalive alone.** #5792's 25 s PING (jittered ±20 %) is the
default; when the configured proxy documents a shorter idle limit, the
PING interval is lowered to stay under it but never below 5 s (a
tunnel-per-5-s ping storm is itself an abuse signal). A tunnel that dies
anyway is a **resumption**, not a session failure (#5792 §5): the ID
layer above never learns about a proxy idle timeout except as a brief
stall.

### 4.5 Reuse and connection count

- Tunnels are per **(proxy, destination, class)**, where class is
  #5792's control+interactive vs bulk split. That is at most two
  tunnels per broker per client — matching #5792's two-connection
  decision, so a proxy never sees a connection fan-out that the broker
  design did not already accept.
- A tunnel is not multiplexed across _different_ destinations, and not
  shared between a broker leg and the local SSH endpoint (§7): a
  compromised or logged proxy must never be able to correlate the two by
  seeing them on one connection.

## 5. Proxy authentication and credential handling

### 5.1 Schemes

| Scheme                            | v1                                             | Reason                                                                                                                                                                                                                                         |
| --------------------------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Basic`                           | **supported**                                  | universal, trivially implemented over the tunnel handshake, and the only scheme our own egress speaks                                                                                                                                          |
| `Digest`                          | follow-up (§13)                                | no better than Basic inside TLS-to-proxy, added complexity for a legacy case                                                                                                                                                                   |
| `NTLM` / `Negotiate` / `Kerberos` | **not supported in v1**; classified, not faked | multi-round challenge protocols with host-level state and platform credential APIs; a faked implementation that "sort of" works is worse than a precise `proxy_auth_required` telling the operator this proxy needs a scheme ID does not speak |

An unsupported challenge is a **first-class, precise outcome**:
`proxy_auth_required` with `scheme=negotiate` in the message, not a
generic failure and not a silent direct attempt.

### 5.2 Where credentials live

- Configuration stores a **reference**, never a secret: an OS credential
  store key (`keyring:`/`secret-service:`/macOS keychain item name), or
  an environment variable **name** whose value is read at use.
- Accepted references: `credential://<store>/<item>`,
  `env://<VAR_NAME>`, and (test harnesses only, behind an explicit
  `--allow-insecure-credential`) `literal:`. `literal:` is refused for
  any non-loopback proxy unless that flag is passed, and its use is
  reported in diagnostics as an insecure configuration.
- A credential embedded in a proxy URL from environment or config is
  moved to an in-memory reference at parse time, redacted from the
  parsed configuration's `Debug` output, and never written back.
- **Never in:** task payloads, queue results, logs at any level, error
  messages, diagnostics output, crash dumps this code controls, or the
  rendezvous advertisement (credentials are per-client egress facts;
  advertising them would be both a leak and meaningless to a peer).

This answers #5671's open question and #5791 §8's deferred half
("storage itself stays with the HTTP-CONNECT traversal design"): the
_storage_ is the platform credential store behind a reference; the
_negotiation rule_ (#5791 §5.4, pause-and-surface on 407) stands
unchanged.

### 5.3 The 407 flow

1. Attempt CONNECT without credentials.
2. `407` with `Proxy-Authenticate: Basic realm="…"` → resolve the
   configured reference **once**.
3. Retry CONNECT **once** with `Proxy-Authorization`.
4. Second `407` → `proxy_auth_required`, surfaced, **no further
   attempts** and no credential rotation/guessing. A wrong password must
   not look like a network outage, and must not produce a login-attempt
   storm against a corporate proxy (which is how accounts get locked).
5. `407` with a non-`Basic` challenge → `proxy_auth_required`, surfaced
   with the scheme named, per §5.1.
6. Credential resolution failing (item missing, store locked) is
   reported as the resolution error plus `proxy_auth_required` — the
   operator needs to know whether to fix the password or the store.

`NO_PROXY` and a `--no-proxy` forbid are checked **before** any
credential is resolved, so a no-proxy destination never triggers a
credential prompt.

### 5.4 One credential per (proxy, destination) at a time

A proxy challenge is scoped to the proxy, not the destination, but the
credential may be per-destination in config. The client keys the
resolved credential cache on `(proxy, credential_ref)` and clears it
when the reference changes or the process exits. No credential is cached
to disk.

## 6. Corporate proxies that inspect traffic

This is the honest section. A TLS-intercepting corporate proxy
(MITM/inspection appliance) presents its own CA to the client and
re-originates TLS to the destination. Consequences for ID, stated rather
than papered over:

1. **ID does not defeat inspection and does not try.** Pinning corporate
   CAs into ID's trust store is an operator action on the host, not a
   behaviour ID adopts because a certificate failed. ID never disables
   verification, never accepts a presented certificate because "a proxy
   is configured", and never downgrades to plaintext.
2. **Inside the tunnel, ID's E2E layer is what survives inspection.**
   The broker forwards ciphertext (#5792 invariant 1); the ID handshake
   is end-to-end. An inspecting proxy that terminates the _outer_ TLS
   therefore sees ID's handshake and frames as opaque bytes. That is the
   same exposure #5671 §4 already assigns to any network path, which is
   the point of keeping payloads E2E.
3. **But ID's outer TLS check fails and that is the correct outcome.**
   If the proxy re-originates TLS to the broker, the certificate ID sees
   is the appliance's, not the broker's; ID must fail `tls_failure`
   (#5791 §6.2: stop and surface, never fall through). Error text names
   the likely appliance and the remedy (install the corporate CA if the
   operator's policy allows, or use a destination the policy exempts).
4. **Do-not-decrypt lists are the operator's lever.** The design
   assumes, and documents, that the correct fix for ID on an inspecting
   network is a policy exemption for the broker endpoint — not a client
   workaround. This is recorded so support conversations start from the
   right fact.
5. **Authentication of the _proxy_ does not authenticate the _peer_.**
   A `200 Connection established` proves the proxy accepted a tunnel;
   it says nothing about the destination. The TLS handshake inside is
   the only peer authentication, on every tier, and no code path may
   treat the proxy's `200` as evidence about the far end.

## 7. Locally terminated endpoint for SSH-class traffic

### 7.1 Chosen shape

The client runs a **local listener on loopback** (`127.0.0.1`, ephemeral
or requested port) that speaks plain SSH. Bytes arriving on it are
carried through a CONNECT tunnel to a broker, which forwards them per
#5792 to the host's world SSH server (`SshTunnelProtocol`, host side
already implemented at `pkgs/id/src/world_ssh.rs:97-147`).

This is the topologies #5671 §4 already names: plaintext exists in the
local `id` process and the local SSH client, and nowhere else. The
existing `id world ssh-tunnel --listen` (`pkgs/id/src/cli.rs:1962-1977`)
is the same shape over the Iroh route; the CONNECT path reuses the
argument surface and the listener/forward code, adding `--proxy` and a
tunnel-class selection rather than a parallel command.

```
ssh -o ProxyCommand="id world ssh-tunnel --stdio --proxy $HTTPS_PROXY NODE" host
ssh -p <local-port> host          # after `id world ssh-tunnel --listen --proxy …`
```

### 7.2 Boundaries

- **Loopback only by default.** A non-loopback `--listen` requires an
  explicit flag and warns: a local endpoint that binds `0.0.0.0` turns a
  UDP-denied-client workaround into an open SSH relay on a laptop's LAN.
- **The local endpoint is not an authentication boundary.** It grants
  nothing: the SSH session authenticates inside the tunnel exactly as it
  would directly, and broker authorization (designed by #5795, still
  open — not this record) governs whether the broker forwards for this
  client at all.
- **No SSH termination and no SSH re-origination** at the local
  endpoint or the broker. The broker is a byte pipe (#5792 §6); the
  local endpoint is a byte pump. Host keys are verified by the SSH
  client, unchanged.
- **`--stdio` remains** for `ProxyCommand`, since some environments
  cannot bind a listener; it is the same pump with stdin/stdout as the
  local socket.
- **Per-connection accounting is metadata-only** — endpoints, byte
  counts, duration — matching #5671 §4's minimization. Command
  contents, key material and session bytes are never logged.

## 8. Failure classification (mapping onto #5791 §6)

No new top-level classes are introduced; the CONNECT tier returns
#5791's classes with the proxy hop named.

| CONNECT-layer outcome                    | Class                                                   | Next action per #5791 §6.2                                                         |
| ---------------------------------------- | ------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| TCP to proxy refused / unreachable       | `timeout` or `transport_error` (with the connect error) | fall through                                                                       |
| Proxy accepts, never answers the CONNECT | `timeout` (CONNECT-hop deadline, §4.3)                  | fall through                                                                       |
| `407` (any scheme)                       | `proxy_auth_required`                                   | stop, ask operator once (§5.3)                                                     |
| `403`, `502`, `503` from the proxy       | `proxy_denied`                                          | fall through (proxy refusing one destination says nothing about direct transports) |
| Malformed/garbage response               | `transport_error` with the raw status line (redacted)   | fall through                                                                       |
| Tunnel up, TLS inside fails validation   | `tls_failure`                                           | **stop and surface** — never falls through, never downgraded                       |
| Tunnel up, ID handshake auth fails       | `auth_failure`                                          | **stop and surface**                                                               |
| Tunnel dies mid-session                  | not a failure class — a **resumption** (#5792 §5)       | resume; surface only if the resume window expires                                  |

Two rules that follow and are easy to get wrong:

- **`proxy_denied` must not quarantine the proxy.** #5791 §6.3
  quarantines `udp_blocked`/`policy_denied`; a proxy refusing _one_
  destination is destination-specific, so the proxy stays a candidate
  for other destinations and `NO_PROXY`-exempt ones.
- **A `407` must not be remembered as a network fact.** It is a
  credential/operator fact; #5791 §4.3's rule that `auth_required` is
  never retried from memory applies unchanged.

## 9. What stays end-to-end / where plaintext exists

| Path                                                      | Encrypted end to end?                        | Where plaintext exists                                                                           |
| --------------------------------------------------------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| ID data/sync through broker over CONNECT                  | yes (#5792, #5671)                           | true endpoints (+ local `id` endpoint if the workload terminates there)                          |
| ID queries/results                                        | yes                                          | same                                                                                             |
| SSH through local endpoint → CONNECT → broker → world SSH | yes, SSH's own E2E                           | local SSH client, local `id` pump, host's SSH server                                             |
| CONNECT request itself (host:port, auth header)           | **no** — visible to the proxy                | proxy (this is inherent, and minimized: no URLs, no payload, no metadata beyond the destination) |
| Hop TLS to broker (outer)                                 | terminates at broker and proxy if inspecting | broker; appliance if MITM                                                                        |
| Control/rendezvous                                        | authenticated + integrity-protected          | as advertised in the rendezvous record                                                           |

**Named non-E2E item:** the CONNECT destination and the fact that a
tunnel exists are visible to the egress proxy by construction. This is
the one non-E2E datum this tier introduces, and it is the minimum the
mechanism can carry. Nothing else — no lengths beyond TCP's own
observable byte counts, no frame types, no stream counts visible to the
proxy as _semantic_ items — is added by ID.

## 10. Invariants implementation must hold

1. **One CONNECT implementation.** No shell-out to system proxy tools, no
   second code path for the SSH endpoint; both callers use the same
   tunnel client.
2. **One tunnel, one destination.** A tunnel is never retargeted; a new
   candidate gets a new tunnel.
3. **TLS inside is mandatory and verified.** No flag, environment
   variable or corporate-proxy detection may disable certificate
   verification or accept a certificate because a proxy is present.
4. **`tls_failure` and `auth_failure` never fall through** and never
   downgrade (#5791 §7.3 inherited, restated because a CONNECT tier makes
   the temptation concrete: "the proxy is right there, just use it
   plainly").
5. **Credentials are references.** No secret in config, logs, error
   text, diagnostics, task payloads or queue results; `literal:` only
   behind the explicit insecure flag and reported when used.
6. **One credential attempt per proxy per destination.** A second `407`
   surfaces; ID never rotates, guesses or brute-forces credentials.
7. **A `200 Connection established` proves nothing about the peer.**
   Peer authentication is the TLS/ID handshake inside, always.
8. **Keepalive rides the tunnel** below the proxy's idle floor (#5792's
   25 s PING, §4.4), and a tunnel died is a resumption, not a session
   failure.
9. **The local endpoint binds loopback by default** and grants no
   authority; non-loopback binding is explicit and warned.
10. **E2E is untouched.** The CONNECT tier carries bytes it cannot read;
    nothing in this layer terminates, re-encrypts or inspects ID payloads
    or SSH.
11. **Failure attribution names the layer** (proxy vs far end) in every
    user-visible and diagnostic message.
12. **Bounded behaviour:** one CONNECT attempt per candidate per
    negotiation pass, inside #5791's overall budget; no reconnect loops
    against a proxy that just refused (#5791 §3.3).

## 11. Alternatives considered and rejected

1. **Shell out to `socat`/`ssh -o ProxyCommand`/`curl -x`.** Rejected:
   an external process per tunnel, credentials on a command line (visible
   in `ps`), no failure classes, no deadline separation, and a runtime
   dependency the `id` binary otherwise does not have. `ProxyCommand` is
   what users do _today_ — the point of this design is to make ID do it
   with ID's error semantics.
2. **Plain HTTP request smuggling instead of CONNECT** (i.e. use the
   broker's WebSocket/HTTP adapter as the primary path). Rejected as
   primary: #5792 already designates WebSocket as the last-resort
   adapter where CONNECT is denied, and CONNECT is strictly better when
   available — raw byte stream, no per-message overhead, no intermediary
   text/compression extensions to disable.
3. **Terminate TLS at the proxy or the broker and re-originate**
   ("make it work with the corporate appliance"). Rejected outright: it
   is exactly the #5671 §4 / #5792 invariant-1 violation — a compromised
   or logged middlebox would gain content access. This is not a silent
   fork of a parent decision; it is the fork the parent explicitly
   forecloses.
4. **Trust any CA when a proxy is configured.** Rejected: it makes
   "there is a proxy" a downgrade signal an attacker or a
   misconfiguration can trigger, and it turns ID's assurance into a
   property of the network rather than of the peer.
5. **PAC/WPAD auto-discovery as the default.** Rejected for v1 (§3.2).
6. **NTLM/Negotiate support in v1.** Rejected as unsupported-but-named
   (§5.1); a partial implementation is worse than a precise error.
7. **One long-lived tunnel multiplexed across destinations and
   classes.** Rejected: it collapses #5792's class split (interactive
   head-of-line isolation), creates a single failure domain for a
   client's whole egress, and hands a logging proxy a cheap correlation
   between a world session and an SSH session.
8. **A separate local proxy daemon for SSH-class traffic** (a
   `ProxyCommand`-shaped sidecar). Rejected: another process to deploy,
   supervise and authenticate; the listener inside `id` (§7) already
   exists in the `world ssh-tunnel` shape.
9. **Detect the proxy by probing.** Rejected (§3.3) — leaking direct
   attempts is the failure mode, not the diagnostic.

## 12. Relation to the parent — explicit check

No decision above changes a #5671 decision:

- **Fallback order** — CONNECT is used only as tier 4; nothing here
  promotes it (§1.1).
- **Broker shape** — both legs outbound, `id` serve-proxy mode, TLS 1.3
  over 443: used, not amended (§1.2).
- **Trust model** — broker sees metadata only; §6 and §11.3 _extend_ the
  same rule to the egress proxy, which is a stricter reading, not a
  change.
- **E2E boundary** — §9 names the one non-E2E datum (CONNECT
  destination) explicitly rather than leaving silence, which is what
  #5671 §6 requires.
- **Out-of-scope per the packet:** broker authorization and the
  capability/allowlist policy (#5795), latency numbers (#5797),
  rendezvous substrate (#5793), Workers component (landed). This record
  only fixes what those designs may assume about the CONNECT tier.

Two parent **open questions** are touched and answered here rather than
silently forked:

- "Proxy-auth credentials: where stored, and how are they kept out of
  logs/config dumps?" — §5.2 (reference in config, platform credential
  store at use, redaction rules).
- "How does fallback interact with #785's SSH-specific design —
  supersede, wrap, or coexist?" — this record's §7 **wraps** it: #785's
  SSH-over-Iroh route is unchanged, `world ssh-tunnel` gains a CONNECT
  transport choice, and nothing in #785 is superseded. The final
  supersede/wrap call for the SSH _fallback requirement_ stays with
  #785's own open scope decision (`doc/2026-11-01_design_id785.md`).

## 13. Splits and follow-ups

No further design split: the tier is one mechanism with four parts
(discovery/config, tunnel establishment, credentials, local endpoint),
and they are decided together above.

Implementation, when scheduled, breaks naturally as:

1. `connect-proxy` client module: request/response, deadlines, failure
   classification (§4, §8) — testable as a pure-ish function against a
   scripted proxy with PBT over response parsing and a chunking oracle
   over the post-header byte stream.
2. Credential references + redaction (§5.2–5.4), with a test that a
   secret never appears in `Debug`/error/log output.
3. Transport-stack wiring: CONNECT as tier 4 in #5791's candidate walk
   (consumes #5791's memory/quarantine, adds no new state).
4. Local SSH endpoint: `--proxy`/transport selection on
   `world ssh-tunnel`, loopback default plus the non-loopback warning.

Recorded as follow-up design questions (not blocking this record):

- PAC/WPAD, if ever: constrained resolver, same credential rules.
- `Digest`, if a real network needs it.
- Whether the credential store abstraction should be shared with other
  fleet secrets (there is an existing `app-cred-helper` pattern on the
  fleet side; unifying them is a fleet-secrets concern, not this tier's).
