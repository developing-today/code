# Actions exec — build and tiered-extension proof material

Raw proof scripts and run logs from the two lanes that built fleet Actions exec, the alternate exec mode that runs agent sessions directly in GitHub Actions runners. The design and the authoritative completion records live on the tracking issue, [developing-today/code#13774](https://github.com/developing-today/code/issues/13774). This folder preserves the lanes' working material, which existed only in /tmp on the laptop and the sandbox. Landed 2026-11-24 as a backfill under the work-product landing rule.

## ACTIONS-EXEC-2 — the build (2026-11-23)

Built and deployed the broker verbs, the OIDC RS256 gate at exec.security.cab, and the `fleet actions` command family.

- Code PR [#13800](https://github.com/developing-today/code/pull/13800) merged `5e4107076821`; dezren39/fleet PR [#113](https://github.com/dezren39/fleet/pull/113) merged `42a102884c3c`; bootstrap pin PR [#13817](https://github.com/developing-today/code/pull/13817) merged `84aab4fcc77a`.
- `fleet-static-5e41070` published; queue broker flipped to generation 7.
- Handoff proven end to end twice: session `fx-788a7a160d3bb9e52664` on dezren39/fleet run 45282914944, and session `fx-92377966ddb3778af0a9` on run 45283728217. The second run failed at the composite action's agent-install step; diagnosing it needs that run's log, which is held behind an execution-risk flag hold (stop record: [#13774 comment 7116388536](https://github.com/developing-today/code/issues/13774#issuecomment-7116388536)).
- Supplement: `ae-proof.sh`, the lane's in-vivo proof script (ran on the laptop).

## ACTIONS-EXEC-3 — tiered lease extensions (2026-11-23 to 2026-11-24)

Implemented the tiered lease-extension policy: first extension auto-approved with a notification-only push, second extension under a deny window where silence approves, manual approval beyond that and past the size and cumulative caps.

- Design amendment recorded first: [#13774 comment 7116645799](https://github.com/developing-today/code/issues/13774#issuecomment-7116645799). Completion record: [#13774 comment 7118298977](https://github.com/developing-today/code/issues/13774#issuecomment-7118298977).
- PR [#13854](https://github.com/developing-today/code/pull/13854) merged `f56dc7f713efacb3ff5159786fa50a32d0cb5cd8`; pin PR [#13869](https://github.com/developing-today/code/pull/13869) merged `df6da50247d7d0722bd2170b283747b2a93d2d82`.
- `fleet-static-f56dc7f` published; queue broker flipped to generation 8, generation 7 kept as rollback.
- Live verification on the production broker: session `fx-e64e5bdf0b5a97668450` — first extension auto-approved at tier 1, second pended at tier 2, a deny landed inside the window, session closed. Tier-2 silence approval is unit-proven only (it needs a real 10-minute window).
- Known gap found by the live proof: broker-raised pushes are currently rejected by the PM aggregator's alerts intake (HTTP 401, token mismatch), filed as [#13902](https://github.com/developing-today/code/issues/13902). The tier flows work end to end; delivery to the phone is what is blocked.
- Supplements: `ae3-live-verify.sh` (the live verification script) and the lane's run logs — `ae3-test-fq.log` and `ae3-test-bin.log` (test suites: fleet-queue 616 passed, fleet bin and registry 141 passed), `ae3-clippy.log`, `ae3-clippy2.log`, `ae3-clippy3.log` (clippy could not complete on the laptop's version-split toolchain; main-branch CI is the backstop), `ae3-publish.log`, `ae3-publish2.log`, `ae3-deploy-build.log`, `ae3-deploy.log`, `ae3-dl.log`, `ae3-live-verify.log`.
