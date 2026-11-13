# Remote GUI access runbook — which path for what, and what is (not) a blocker

Written 2026-11-04, implementing the GUI-access verdict in
[#5924](https://github.com/developing-today/code/issues/5924). Config:
`nixos/hosts/amd-sunshine.nix` (Sunshine host + Moonlight client + encoder
watchdog) and the Helium CDP wrapper in `nixos/environment/default.nix`.
Host facts verified on the machine the same day: laptop `amd` runs Hyprland
(Wayland/wlroots) under SDDM; GPU is a Radeon 780M iGPU with VAAPI H.264 +
HEVC hardware encode (Mesa radeonsi). Tailnet addresses for the laptop:
`amd-6.raptor-pumpkinseed.ts.net` / `100.99.113.103`.

**Status at writing:** the flake change is merged; activation is a switch +
the checklist in §5, performed when the desktop session is free (a switch
can restart the session — never switch while someone is working through
the desktop).

---

## 1. Decision table — I need to… → use… → how

| I need to…                                             | Use                                     | Address / how                                                                                                                                                                                                                              |
| ------------------------------------------------------ | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Sit at the laptop desktop interactively (human)        | **Sunshine + Moonlight** (primary)      | Moonlight on your device → add host `100.99.113.103` (or the MagicDNS name) → pair once with the PIN from the Sunshine web UI. Tailnet only.                                                                                               |
| Reach the login screen / greeter, or Sunshine is down  | **RustDesk** (fallback)                 | RustDesk against the laptop's desktop session. From the sandbox there is a dedicated SSH forward (`127.0.0.1:21118` → laptop). Never expose 21118 raw — the fleet splice is [#5694](https://github.com/developing-today/code/issues/5694). |
| Everything above is broken and a human must get in NOW | **Chrome Remote Desktop** (break-glass) | One-off, human-driven, then turn it back off. CRD standing/on-by-default is a posture bug — see §4.                                                                                                                                        |
| Do something in an app that has an API/CLI             | **The API/CLI** (agents, first choice)  | Whatever the service's own endpoint is. A GUI is the last resort, not the default.                                                                                                                                                         |
| Drive the browser (agents)                             | **CDP against Helium**                  | `http://100.99.113.103:19222` — route and recipe in §3. On the laptop itself: `http://127.0.0.1:19222`.                                                                                                                                    |
| Drive a native desktop app (agents)                    | **AT-SPI**                              | On the laptop over SSH (`~/tools/laptop-ssh.sh`), against the user session bus (`/run/user/1337/bus`).                                                                                                                                     |
| See what the screen looks like (agents)                | **A still screenshot, on demand**       | `grim` on the laptop over SSH. One frame when needed — never a stream.                                                                                                                                                                     |
| Get a shell on the laptop                              | **SSH**                                 | `~/tools/laptop-ssh.sh` (multiplexed, tailnet). Not the fleet broker.                                                                                                                                                                      |

**Agents never consume a standing video stream.** Not Sunshine, not
RustDesk, not anything. The hierarchy is API → CDP → AT-SPI → stills →
headless (Xvfb / headless sway) as the last resort. If a task seems to
"need" live video for an agent, the task is mis-shaped — escalate, don't
wire up a stream.

## 2. Sunshine / Moonlight (primary desktop path)

- Host side: `services.sunshine` (nixpkgs module) on `amd`, running as a
  **user service** with the graphical session. Capture is `kms` (grabs
  KMS/DRM directly — works under Hyprland/wlroots, independent of the
  compositor), encoder is pinned as a preference to `vaapi`.
- Client side: Moonlight-Qt is installed on the laptop (it is both host
  and client); on phones/other machines use the normal Moonlight app.
- Ports (base 47989): TCP 47984, 47989, 47990, 48010; UDP 47998, 47999,
  48000, 48002, 48010. Web UI: `https://100.99.113.103:47990`.
- Posture: **tailnet-only, by construction.** `openFirewall = false` in
  the config; the ports are reachable because `tailscale0` is a trusted
  firewall interface on this host. They are not open on the LAN, and
  nothing forwards them publicly. Do not "fix" connectivity by opening
  the firewall — fix tailnet membership instead.
- One-time setup: create the web-UI admin credentials (Sunshine stores
  them hashed under `~/.config/sunshine` — never in this repo), then
  pair each Moonlight client with the PIN the web UI shows.

### The encoder rule (the #5924 spike criterion)

A Sunshine stream that "works" proves nothing. The log must show the
**hardware** encoder engaged:

```
Found H.264 encoder: h264_vaapi [vaapi]        ← healthy
Found H.264 encoder: libx264 [software]        ← FAILURE, even if video plays
```

Why this needs a check and not just the config pin: in the pinned
Sunshine source (`src/video.cpp`, `probe_encoders`), a configured
encoder that fails validation is logged
(`Couldn't find any working encoder matching [vaapi]`) and the probe
then **falls through to the remaining encoders, software included**. The
pin is a preference; the alarm is `sunshine-encoder-check`:

- It runs hourly and 10 min after boot as a user timer, and can be run
  by hand any time.
- It FAILS (unit lands in `systemctl --user --failed`) on: the pinned
  encoder being rejected, no working encoder at all, or a software
  winner at probe (`Found … [software]`) or stream time
  (`Creating encoder [software]`).
- It passes only on positive VAAPI evidence in this boot's journal.

## 3. CDP against Helium (agent browser path)

- Helium is wrapped in the flake (`nixos/environment/default.nix`) so
  **every** launch — launcher, desktop file, or shell — carries
  `--remote-debugging-address=127.0.0.1 --remote-debugging-port=19222`.
  Before this change the installed Helium had no debugging port at all.
  A browser process started _before_ a switch keeps its old flags: after
  activation, fully quit Helium and relaunch it once.
- The endpoint binds loopback on the laptop. The tailnet path is a
  `tailscale serve` mapping (created at activation, §5 — serve state is
  imperative tailscaled state in this fleet and is never declared in the
  flake):

  ```
  tailscale serve --bg --tcp 19222 tcp://127.0.0.1:19222
  ```

- **Sandbox → CDP route** (proven 2026-11-04 with a throwaway instance
  on a scratch port: `/json/version`, `/json/list`, and a WebSocket
  `Runtime.evaluate` all succeeded end-to-end): the sandbox reaches the
  tailnet only through the runtime CONNECT proxy — reuse `$HTTPS_PROXY`
  with its port changed to **3130** — and address the laptop by its
  **tailnet IP literal**:

  ```
  http://100.99.113.103:19222/json/version
  http://100.99.113.103:19222/json/list
  ws://100.99.113.103:19222/devtools/page/<id>   (from /json/list)
  ```

  Use the IP, not the MagicDNS name: Chrome's DevTools server rejects
  non-localhost DNS names in the Host header. WebSocket debugger URLs
  returned by `/json/list` already carry the address you connected with.

## 4. Fleet-proxy routing verdicts (study, 2026-11-04)

The fleet proxy is two things: a TCP **CONNECT** egress proxy (the
runtime's, port 3130 for tailnet) and the fleet **broker** protocol
(HTTP request/response JSON pooled per port, for services). Neither
carries UDP, and the broker is request/response-shaped — it has never
carried a raw long-lived byte stream. Per path:

| Path                         | Through the fleet proxy?                                   | Why                                                                                                                                                                                                                                                                                                                                                                    |
| ---------------------------- | ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Sunshine/Moonlight streaming | **No — tailnet-direct, always**                            | The stream is UDP-centric (video/control on UDP 47998–48010). A TCP CONNECT proxy cannot carry it at all, and doesn't need to: tailnet peers already talk direct WireGuard UDP (DERP relay is Tailscale's own fallback and still not the proxy). Putting TCP in the video path would add head-of-line blocking and latency to the one workload that tolerates neither. |
| RustDesk                     | **Not via the broker; stays on its dedicated SSH forward** | RustDesk is TCP-shaped, but the broker's request/response pooling is the wrong layer for an interactive byte stream. The existing isolated SSH forward works; the designed end state is the #5694 splice (fleet tunnel provides confidentiality; 21118 never exposed raw). No change until #5694 builds it.                                                            |
| CDP (Helium)                 | **Yes — this is the sanctioned route**                     | CDP is plain TCP (HTTP + WebSocket). Sandbox → runtime CONNECT proxy (:3130) → tailnet → `tailscale serve` → loopback. Interactive but tiny messages; proxy overhead is noise. Proven live, §3.                                                                                                                                                                        |
| SSH                          | **No — stays on `laptop-ssh.sh`**                          | Multiplexed SSH over the tailnet already works and is the recovery path for everything else; moving it behind the broker would couple the recovery path to the thing it recovers.                                                                                                                                                                                      |

Rule of thumb: **streams stay on WireGuard; control paths may ride the
CONNECT proxy; the broker is for request/response services only.**

## 5. Activation checklist (run in order, desktop free, no switch running)

1. Confirm nobody is working through the laptop desktop and no
   `nixos-rebuild` is already running.
2. Switch from the primary checkout: `nixos-rebuild switch --flake ~/git/code#amd`
   (Drew's standing switch order).
3. `systemctl --user status sunshine` — should be active with the
   graphical session (one logout/login may be needed the first time).
4. Create the CDP serve mapping:
   `tailscale serve --bg --tcp 19222 tcp://127.0.0.1:19222`
5. Fully quit Helium; relaunch from the launcher; verify on the laptop:
   `curl -s http://127.0.0.1:19222/json/version`
6. Verify from the sandbox via the §3 route (`/json/version` + `/json/list`).
7. Open `https://100.99.113.103:47990`, create the Sunshine admin
   credentials, pair Moonlight from the client you will actually use.
8. Start a stream, then run `sunshine-encoder-check` — it must print
   the VAAPI OK line. **This is the #5924 spike criterion; the rollout
   is not done until it passes during a real session.**
9. `systemctl --user --failed` should be empty.

## 6. Blocker taxonomy — what actually blocks you, and what does not

### REAL blockers

| Symptom                                                 | Check                                                                                                                                                                                                                                                                                                                                       | Fix                                                                                                                                                                                                                                        |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Stream works but is laggy / laptop CPU is pegged        | `sunshine-encoder-check`, or `journalctl --user -u sunshine -b \| grep "Found H.264"`                                                                                                                                                                                                                                                       | The encoder fell back to software. Fix VAAPI: `/dev/dri/renderD128` access, user in the `video` group, Mesa/VAAPI driver health (`vainfo` must list EncSlice profiles). Do not accept the stream as healthy until the log shows `[vaapi]`. |
| `sunshine.service` down / Moonlight can't find the host | `systemctl --user status sunshine`; `ss -tlnp \| grep 4798`                                                                                                                                                                                                                                                                                 | Restart the user service; if KMS capture fails in its journal, check the capability wrapper (`capSysAdmin`) and that the session is graphical (the service chains to `graphical-session.target`).                                          |
| CDP unreachable from the sandbox                        | Ladder, in order: (1) on the laptop, `curl 127.0.0.1:19222/json/version` — if dead, Helium predates the switch or isn't running: quit + relaunch. (2) `tailscale serve status` — the 19222 mapping missing means activation step 4 wasn't run. (3) From the sandbox, confirm the route: proxy port **3130** and the **IP literal**, per §3. | Fix the first rung that fails; the others follow from it.                                                                                                                                                                                  |
| Moonlight sees no host / pairing fails                  | Both ends on the tailnet? Web UI reachable at `:47990`?                                                                                                                                                                                                                                                                                     | Tailnet membership first. Pairing state is per-client in the web UI — remove and re-pair. Never open firewall ports as a shortcut (§2).                                                                                                    |

### NOT blockers — do not "fix" these

| Observation                                                                 | Why it's fine                                                                                                                                           |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RustDesk is not running                                                     | It's the fallback, not the primary. Sunshine being up is the healthy state.                                                                             |
| Chrome Remote Desktop is absent / off                                       | Break-glass only. Absence is the correct posture; a standing CRD service is a finding, not a convenience.                                               |
| No video stream exists for agents                                           | By design (§1). Agents use APIs, CDP, AT-SPI, stills. A request to "just give the agent the Moonlight feed" is the failure mode #5924 rejected.         |
| KRdp unit inactive                                                          | KRdp is bound to the Plasma session; this host runs Hyprland. Expected, unrelated to Sunshine.                                                          |
| `sunshine-encoder-check` says "inconclusive / not run yet" right after boot | It only verdicts on log evidence. It becomes meaningful after the first probe/stream — and it turns into a loud FAIL on its own if the evidence is bad. |
