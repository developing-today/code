# Reproducing the C2 netfilter-emulation lab

From the repository root, run:

```sh
nix --extra-experimental-features 'nix-command flakes' run ./doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/c2-holepunch#c2-lab
```

The flake supplies Rust/Cargo (and sccache for Cargo configurations that select it), nftables, iproute2, util-linux, procps, GNU grep/sed, OpenSSL and Python. It builds the C2 peer and the pinned iroh-relay 1.3.0 server, runs the three NAT-behaviour probes, then runs all nine ordered NAT pairs. The default connection window is 8 seconds per pair; set `C2_WINDOW=60` before the command for a longer observation window.

The run uses `unshare -Urn`; it changes networking only inside its temporary user/network namespaces. It writes a timestamped, labelled-emulation report under `results/` and keeps transient detailed logs in the user's cache. The report explicitly distinguishes Linux netfilter emulation from real NAT-device or real-network measurements.

To run one existing scenario manually after building, set `NFT`, `RELAY_BIN` and `C2_BIN` to executable paths and invoke `lab/probe-nat.sh` or `lab/run.sh`. Missing executables fail with a direct error rather than falling back to machine-specific paths.
