# Design: build-time defaults file for id network defaults

- Issue: #5916 (Drew's requirement, peer-discovery plan review, 2026-03-23).
  Relates: #4756 (topic/secret pairing), #3447 (id-core extraction, which
  carried the current mechanism in), #2145 (web_bindings), epic #2056.
- Status: design record for queue task 16881 (design pass). This document
  settles the six questions the #5916 packet names. It carries no
  implementation; §8 is the implementation follow-on's brief.
- Method note: the issue's 2026-11-04 snapshot ("no defaults-file mechanism
  exists") is stale. A mechanism shipped with the id-core extraction and
  grew in #2145. This design is therefore a _migration and hardening_ of a
  live mechanism, not a greenfield build — and it starts from an audit of
  what the live mechanism actually does (§1), including two defects.

## 0. Drew's requirement, restated

From the 2026-03-23 review (verbatim in #5916): the network defaults
should be populatable from an optional JSON file at build time, such that
the built binary is equivalent to the hardcoded-const build; a build flag
chooses replace-instead-of-append; serve-launch flags can add values on
top; a flag skips defaults entirely. Drew's follow-up asks the embedding
question directly: "compile time can't parse this into a const on its own
it has to be a string?" — answered in §4.

## 1. Audit: what exists today (verified 2026-11-14)

`pkgs/id/crates/id-core/src/defaults.conf`, embedded via
`include_str!` in `discovery.rs`, hand-parsed once into a `Defaults`
struct behind a `LazyLock`:

- Contents: `[bootstrap]` node IDs, `[topic]`, `[topic_secret]`,
  `[web_bindings]` (#2145). The checked-in file carries the public topic
  and secret, duplicating the `DEFAULT_TOPIC` / `DEFAULT_TOPIC_SECRET`
  consts.
- `resolve_config` cascade: hardcoded consts → conf values → runtime
  `--replace-defaults` → CLI `--bootstrap` appends, `--topic` /
  `--topic-secret` overrides → `--no-default-bootstrap` /
  `--no-default-topic` skips.
- The parser is silent: unknown sections, misspelled section names, and
  extra values in singleton sections are ignored without any signal.
- Relay policy is iroh's N0 default (`connectivity.rs` `RelayPolicy`);
  DNS/discovery endpoints are iroh's; neither is in the file. Gossip
  on/off is a runtime flag only (`--no-gossip`). Timing constants
  (`ANNOUNCE_INTERVAL`, `STALE_THRESHOLD`, …) are protocol constants.

Defects found by this audit:

- **D1 — pair atomicity fails in the default path.** #4756's rule (topic
  and secret resolve from one source, never mixed) is enforced only
  under `--no-default-topic`. In the normal path the two halves resolve
  independently, so a conf supplying a topic but no secret pairs a
  private topic with the _public_ default secret. A private deployment
  that edits only the topic line silently joins a network whose DHT
  records anyone can read.
- **D2 — `--replace-defaults` leaks.** It is documented as "use ONLY
  conf", but when the conf omits the topic or the secret the code falls
  back to the hardcoded consts. Replace mode cannot actually produce a
  const-free binary, and nothing reports that it didn't.

## 2. Decision 1 — format and contents: JSON, typed, fail-loud

The file becomes **`defaults.json`** next to `defaults.conf`'s current
home in id-core, parsed with serde into a typed struct. The conf format
and its hand parser are retired in the same change.

Why JSON over keeping the conf: (a) it is what Drew asked for; (b) the
conf's silent-ignore parser is a correctness hazard for exactly the
private deployments the file exists for — a `[topc]` typo currently
produces a binary on the public defaults with no signal; serde with
`deny_unknown_fields` turns every such mistake into a startup failure
naming the field; (c) the typed struct is reusable if a runtime config
file ever shares the shape (§7).

v1 contents — the deployment-owned network defaults, no more:

```json
{
  "mode": "append",
  "bootstrap": ["<node-id>", "…"],
  "topic": "…",
  "topic_secret": "…",
  "web_bindings": ["127.0.0.1:3000"],
  "relay": "n0",
  "gossip": true
}
```

- `bootstrap`, `web_bindings`: carried over from the conf unchanged in
  meaning.
- `topic` + `topic_secret`: a pair — both present or both absent (§5).
- `relay`: new in v1 — the `RelayPolicy` selector (`n0` default;
  extensible to explicit relay URLs as a list variant). This is the one
  audited network default outside the conf that deployments genuinely
  need to pin.
- `gossip`: new in v1 — the build-time default that `--no-gossip` /
  `--gossip` then override at launch.
- Explicitly out: timing/protocol constants (they are protocol, not
  deployment), DNS endpoints (no id-owned endpoint constants exist;
  iroh's defaults ride with the relay choice — inventing an endpoint
  list before there is a consumer would be a second semantics).

**Equivalence property.** Building with an empty file (`{}`) must
produce effective defaults identical to the const-only build, for every
combination of serve flags. Building with a file whose values equal the
consts must also be identical. This is a test, not a hope (§8): a
flag-matrix test resolves both builds' configs and asserts equality.

**Optionality.** The file is optional at build in the only sense that
matters: `{}` (or a file with only `"mode"`) is a complete, valid build
input meaning "pure consts". A deployment build substitutes its own
file in the source tree (or via the Nix overlay pattern in §4) — the
binary never reads a defaults path at runtime.

## 3. Decision 2 — replace vs append: one build-time mode, in the file

`"mode": "append" | "replace"` lives **in the file itself**, not in a
separate build flag, so the choice travels with the values it governs
and is visible in provenance (§6). Per-list granularity is rejected for
v1: Drew's sketch names one build-level choice, and per-key modes make
the equivalence property and the pair rule (§5) combinatorial. A future
per-key override can be added as data without changing the cascade.

Semantics, applied per value class:

- Lists (`bootstrap`, `web_bindings`): append = const/conf list then
  file list, deduplicated, order preserved. Replace = file list only.
- Scalars (`relay`, `gossip`): file value wins when present, in both
  modes.
- The pair (`topic`, `topic_secret`): append = file pair replaces the
  const pair _as a unit_ when present. Replace = the file pair is the
  only pair; a replace-mode file missing either half is an error at
  first parse (§5), never a silent const fallback. **This closes D2.**

The runtime `--replace-defaults` flag is **removed** in the migration:
it was a runtime spelling of a build-time decision, and its leak (D2)
came from exactly that confusion. `--no-default-bootstrap` and
`--no-default-topic` stay — they are the "skip defaults entirely"
launch flags Drew's sketch names, and they are genuinely runtime
decisions.

## 4. Decision 3 — embedding: string embed + one typed parse (Drew's question)

Drew's intuition is right: compile time cannot turn JSON into typed
consts on its own. The options:

| Option                                              | Verdict                                                                                                                                                                                                                                                                                                |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `include_str!` + serde parse once behind `LazyLock` | **Chosen.** Zero build machinery, the file stays diff-able data, parse cost is once per process and trivially small. This is also what the current mechanism already proves in production.                                                                                                             |
| `build.rs` codegen of typed consts                  | Rejected for v1. Buys compile-time validation at the cost of a build script in id-core (today it has none — the root package's `build.rs` is frontend-assets only, #7152), generated-code churn, and worse Nix ergonomics. The validation it buys is recovered by the checked-in-file parse test (§8). |
| Const-eval JSON parsing                             | Rejected. No serde in const contexts; a hand-rolled const parser reintroduces the silent-parser problem in a harder medium.                                                                                                                                                                            |
| `include!` of a `.rs` data file                     | Rejected. The input stops being JSON — it fails Drew's format requirement and makes the file unusable by non-Rust tooling.                                                                                                                                                                             |

Placement: the file and its parsing stay in **id-core** beside
`discovery.rs` — the root package's `build.rs` is not involved, and no
new build script is added anywhere. **Nix supply:** the file is in-tree
source; a Nix build that wants custom defaults substitutes the file via
the package's existing source override (the same pattern as any patched
source file) — no env-var plumbing, no `ID_DEFAULTS_FILE` build-time
path reading, because a path outside the source tree would make builds
non-reproducible and provenance (§6) unauditable.

## 5. Decision 4 — runtime composition: one cascade, pair-atomic

One cascade, documented in exactly one place (`discovery.rs`), replacing
the two divergent resolution-order docs the current code carries:

```text
const layer        hardcoded DEFAULT_TOPIC/SECRET pair, empty lists, relay=n0, gossip=on
  ↓ file layer     defaults.json, applied per its mode (§3)
  ↓ launch adds    --bootstrap appends; --web-listen replaces web_bindings (existing #2145 rule)
  ↓ launch pair    --topic + --topic-secret override as a pair
  ↓ launch skips   --no-default-bootstrap drops default bootstrap;
                   --no-default-topic requires the launch pair or falls back to the const pair
  ↓
effective config
```

**Pair rule (closes D1):** at every layer, topic and secret are taken
from exactly one layer as a unit. A layer supplying exactly one half is
an error naming the layer and the missing half — the rule #4756 already
enforces on the `--no-default-topic` path, extended to the file layer
and the default path. `resolve_config` keeps its signature shape; the
pair resolution collapses to "highest layer with a complete pair wins".

## 6. Decision 5 — provenance: every binary can name its defaults' origin

Parsing the embedded string also computes its SHA-256. `defaults()`
gains a provenance sibling:

```text
DefaultsProvenance { source: ConstsOnly | File, sha256: Option<String>, mode: Option<Mode> }
```

`source` is `ConstsOnly` when the file is `{}`-equivalent (no effective
values). The provenance is printed in the serve startup log line and in
the version/about surface, e.g.
`defaults: file sha256=9f2c… mode=append` or `defaults: consts`.
No build.rs is needed: the hash is of the embedded string itself,
computed at first parse — it identifies exactly the bytes the binary
was built with.

## 7. Decision 6 — one JSON shape for build and runtime config? No (for v1)

The build-defaults JSON is a closed, deployment-owned, build-lifecycle
shape. Runtime configuration (serve flags today) has a different owner,
lifecycle, and trust profile; merging the shapes now would couple a
build artifact to a runtime surface that does not yet exist as a file.
The serde types are defined so a future runtime config file _can_
embed the same struct — reuse is preserved, coupling is not created.

## 8. Implementation follow-on (brief)

1. Add `defaults.json` (values = today's conf values) + typed serde
   parse with `deny_unknown_fields`; delete `defaults.conf` and
   `parse_defaults`; keep `Defaults` field-compatible plus `relay`,
   `gossip`, `mode`.
2. Rewrite the pair resolution in `resolve_config` per §5; remove
   `--replace-defaults` (CLI + plumbing + tests); enforce the pair rule
   in the file layer (error names layer + missing half).
3. Wire `relay` and `gossip` defaults into connectivity/serve, with the
   existing flags overriding.
4. Provenance (§6): hash at parse, startup log + version surface.
5. Tests: equivalence flag-matrix (empty file ≡ consts; const-valued
   file ≡ consts); replace-mode missing-half errors; file-layer
   single-half pair error (D1 regression); unknown-field rejection;
   the checked-in `defaults.json` parses (the build.rs-validation
   substitute).
6. Docs: the one cascade in `discovery.rs`; remove the stale
   resolution-order copy.

Acceptance mirrors this issue's: a binary built with a custom file
reports its provenance, resolves the pair atomically from one layer,
and a `{}` build is indistinguishable from today's build.
