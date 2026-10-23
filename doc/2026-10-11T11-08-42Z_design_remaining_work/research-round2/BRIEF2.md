# Research round 2 brief (2026-10-11)

You are answering questions the owner asked about an identity/P2P system in `pkgs/id` (Rust). The design is nearly final. Your job is research with evidence. You are not implementing product code.

## Read first (read-only)

- Index: `/home/user/work/integrated/doc/2026-10-11T11-08-42Z_design_remaining_work/2026-10-11T11-08-42Z_design_remaining_work.md`
- Task docs in the same folder: `task-NN-*.md`
- Main design: `/home/user/work/integrated/doc/2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md`
- Round 1 spikes: `/home/user/work/spikes-next/<name>/RESULTS.md` (also copied into the repo under `.../spikes/<name>/`).
- Pins: `/home/user/work/spikes/Cargo.toml`. p2panda git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, iroh 1.3.0, iroh-blobs =0.103.1, iroh-tickets 1.0.0, iroh-relay 1.3.0, cbor-core =0.10.1, redb =4.3.0.

## Rules

1. Write only inside your folder: `/home/user/work/research-round2/<your-name>/`. Do not modify `/home/user/code`, `/home/user/work/integrated`, `/home/user/work/spikes`, `/home/user/work/spikes-next`, or any git checkout of p2panda or iroh.
2. Do not push, open issues or PRs, publish packages, create accounts, deploy, or contact services. Public read-only fetches are fine: crates.io API and downloads, static.rust-lang.org manifests, GitHub raw and API reads, public docs.
3. Do not read or copy the large image files in the repo root (`*.iso`, `*.qcow2`). Do not run `nix build`, `nix develop`, `./rebuild`, or anything that changes the host network config. Use `unshare -Urn` for network experiments.
4. Toolchain: `CARGO="$(rustup which --toolchain 1.97.0 cargo)"`. For pinned crates use `export CARGO_TARGET_DIR=/home/user/work/spikes/target`. Otherwise use `target/` inside your folder. Ten agents share this machine: build only what you need, and run only the tests for what you change.
5. Evidence: every claim needs a source (file and line, URL with date, or command and its output). Never state a result you did not see in output. Label each statement as verified, inferred, or not checked.
6. Output: `RESULTS.md` in your folder, with these sections: Questions (verbatim), Short answers (one paragraph each), Evidence (commands and sources), Options (table, if relevant), Recommendation (reasons, and what would change it), Not verified. Keep it under about 600 lines.
7. Final message to the parent: at most 250 words. One paragraph per question, the path to RESULTS.md, and anything that changes the design.
8. Time budget: about 60 to 90 minutes. If you are blocked, stop and write the blocker, with the exact command and error.
