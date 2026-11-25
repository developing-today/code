# Android live-host view proof

Captured on the `id-relay` Android 35 x86_64 AVD after connecting the APK to a
local `/id-world/1` host over Iroh using the direct `node-id@10.0.2.2:port`
address form. The host served the tic-tac-toe view `------- plays=0 next=X`.

- Cargo tests: [Fleet run 45571511928](https://github.com/developing-today-forks/fleet/actions/runs/45571511928) — two unit tests and one live-session integration test passed.
- Host demo build: [Fleet run 45574035360](https://github.com/dezren39/fleet/actions/runs/45574035360).
- APK build: [Fleet run 45572861024](https://github.com/dezren39/fleet/actions/runs/45572861024).

This demonstrates the live view path. Android tap, drag, and scroll relay are
not part of this slice.

![Connected Android live-host view](live-host-view.png)
