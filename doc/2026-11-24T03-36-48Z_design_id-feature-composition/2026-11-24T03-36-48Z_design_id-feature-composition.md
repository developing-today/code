# Feature composition for the ID binary, and the Roc platform composer as one of its features — critical review and Rust implementation plan

- Created: 2026-11-24 (UTC). Status: **plan**. No implementation is
  authorized by this record; §10 is a slice list for later queueing
  after review.
- Direction: Drew, 2026-11-23 ~21:27 CST — prioritize the
  platform-composer design; find its important details and
  shortcomings; build a full implementation plan for it in Rust; and
  carry the same composition principle into the ID app itself as
  opt-in features: a plain ID binary that is "the dumbest client
  with a basic store", a fully-enabled build that runs as a server
  in all modes for backends, and served-environment builds with
  exactly the appropriate Rust features compiled in (platform
  composition and alternate distributed data stores named as
  examples).
- Design basis: the Roc platform-composer design, Part 3
  (§§3.1–3.9) of the platform design exploration
  (`doc/2026-11-24T02-45-34Z_design_roc-platform-composer/` in this
  repo; workspace original in `~/workspace/roc-investigation/`),
  amending its Part 2 platform-family design.
- Predecessor design this plan succeeds: the feature-leveled build
  variants record for #5788
  (`doc/2026-11-09T04-43-05Z_design_id5788_build_variants.md`) —
  four levels (L0 embedded/provider, L1 client-only, L2 headless
  service, L3 full node), variants as composition crates, cargo
  features confined to seam crates, conformance by replay oracles.
  §0.2 states exactly what this plan keeps, what it changes, and
  why.
- Grounding survey: `pkgs/id` at the 2026-11-23 book-survey commit
  line (`1ccb4a19457` family; workspace re-read 2026-11-24). All
  feature definitions quoted in §0.1/§4 are from the tree, not from
  the books.

---

## 0. Premises checked — what is actually true today

### 0.1 pkgs/id already has a feature structure (premise confirmed, and it shapes the whole plan)

This is not a greenfield design. The tree today has:

- A 17-package workspace: root package `id` (public facade **and**
  the user binary, `src/main.rs`; no `[[bin]]`, no other host
  binary except `id-native-shell`) plus 16 crates under `crates/`
  — the #3447 split is landed, not planned (`id-core`, `id-config`,
  `id-caps`, `id-store`, `id-events`, `id-groups`, `id-sync`,
  `id-worlds`, `id-directory`, `id-currency-registry`, `id-web`,
  `id-render`, `id-native-shell`, `id-hw-shell`, `id-cli`,
  `id-import-notion`), plus two vendored p2panda crates held
  **outside** the workspace for feature-graph reasons (below).
- Exactly six root features:

  ```toml
  default = ["sandbox", "world"]
  sandbox = ["world", "id-core/sandbox", "id-caps/sandbox", "id-worlds/sandbox", "id-cli/sandbox"]
  world   = ["id-core/world", "id-cli/world", "dep:num-bigint"]
  ssh     = ["world", "id-worlds/ssh", "id-cli/ssh"]
  web     = ["world", "dep:id-web", "id-core/web", "id-cli/web"]
  dht     = ["id-sync/dht", "id-cli/dht"]
  currency-poc-tb = ["dep:tigerbeetle"]
  ```

  Features gate _implementations_, not CLI surface: every command
  stays in the clap tree in every build and fails at dispatch with
  "this build has no X support" (`src/main.rs`, `id-cli`
  `serve.rs`/`world.rs`). CI's default posture is `--all-features`
  (justfile, `check-all`, `default.nix`), which silently includes
  the test-only features (`test-utils`, `test-provider` — the
  latter documented "never enabled by a production build") and the
  TigerBeetle PoC harness.

- **No store-backend feature exists.** Store choice is runtime or
  concrete per port, under an explicit port discipline
  (`ARCHITECTURE.md` "Store Port Boundaries": record store, blob
  store, event log, world store, object bucket, message stream,
  transport are separate ports; "a single 'store abstraction' does
  not merge them"; an adapter that cannot satisfy a port returns a
  structured unsupported result). Shipped backends: iroh-blobs
  `FsStore`/`MemStore` for blobs (runtime `--ephemeral` switch),
  one redb file (`events.redb`, format `id.events.store.v9`) for
  the event log behind a file-level `Storage` trait, filesystem
  journal for worlds, JSONL for the directory. SQLite is
  deliberately absent from the shipped graph and guarded by a
  `cargo tree` test in `id-store`. The port contract itself
  (`id-core::port`, `PORT_CONTRACT_VERSION = 2`, with redb and
  Turso/libSQL-over-HTTP profiles already declared in
  `id-store::port_adapter`) exists but the Turso record-store
  adapter is **declared, not wired** — its module header says so.
  A read-only foreign libSQL source (`src/foreign_libsql.rs`,
  hand-rolled HTTP pipeline, writes inexpressible) is shipped.
- Build variants already exist in three places that do not agree:
  `scripts/build.sh` (`lib|web|assets` with a `.build-variant`
  marker file — the #6883 wrong-variant bug class), the Nix flake
  (`id-web` = `web,sandbox,ssh,world` — the only shipped artefact
  with SSH serving, and its comment claiming empty Cargo defaults
  is stale; `id-lib` = `sandbox,world` re-added over
  `--no-default-features`), and the justfile (where "lib" means
  plain `cargo build`, defaults on). `scripts/five-minute-path.sh`
  runs the only minimal build: `cargo build --release
--no-default-features -p id` — which today strips worlds
  entirely while keeping the full CLI tree, i.e. the current
  "minimal" build is close to the dumb client Drew wants in
  _linkage_ but wrong in _contract_ (commands parse, then bail).

### 0.2 Relation to #5788 — what this plan keeps, what it changes

#5788's record settled four fixed levels and **rejected**
feature-only variants and per-consumer variants, on two grounds:
features on one package cannot express a _negative_ guarantee (a
client-only build must not **contain** the server stack), and
consumer-local feature sets are unowned drift. Drew's 2026-11-23
direction asks for arbitrary served-environment compositions,
which the letter of #5788 forbids. This plan reconciles the two
rather than quietly overriding a settled record:

- **Kept, unchanged:** variants are compositions, never `cfg`
  thickets; the negative guarantee is a `cargo tree` property
  enforced in CI per published artefact; conformance is the
  determinism replay fixtures run as cross-variant oracles plus
  the shell/receipt contracts; no "lite server" as a _named
  product_; the currency-PoC binding is a harness artefact in no
  composition; upgrade behaviour per #1066 including the
  client-only degrade contract.
- **Changed, by Drew's direct order, using the composer design's
  own machinery:** the unit of composition drops from the level to
  the **catalogued feature**. The four levels become the first
  four **named profiles** in a registry — curated, published,
  supported. Arbitrary compositions are permitted exactly as the
  composer design permits them for platforms: content-addressed by
  a **composition hash**, unlisted by default, operator-built for
  served environments, and graduated to named profiles by the
  amendment rule #5788 already specified (a profile change is a
  record amendment, never a consumer-local act). This answers
  #5788's drift objection with registry mechanics instead of a
  ban: a served environment's build is not a private accident, it
  is a manifest whose hash names it in every `--version` string
  and bug report (§5).
- **Changed:** #5788 froze the root package's features as "the
  legacy L3 profile" and put all new capability in seam crates.
  That stands for _capabilities_; this plan additionally admits
  root-level _profile_ features whose only job is to name a
  curated feature set (meta-features, §4.3), because served
  environments need a single `--features` handle per profile.

### 0.3 Premises that turned out false

- _"The plain binary is today's default minus flags."_ False:
  today's default **is** the fat client (`sandbox,world` — the
  wasmtime sandbox included). The plain profile is a new,
  deliberate composition, and reaching it requires splitting
  today's `world` feature (§7.2), because `world` currently gates
  both world _state/replay_ (which the dumb client needs) and the
  command surface around program execution (which it must not
  carry).
- _"Alternate stores are a flag away."_ False: §0.1 — the seam is
  designed (port contract v2) and one alternate profile is
  declared, but no alternate authoritative store is wired. The
  store features in §4 are real implementation slices (§10 S6),
  not manifest entries over existing code.
- _"All flags on" is a well-defined build._ False as stated:
  `--all-features` includes test-only and PoC features that must
  never ship. The full profile in §6 is a **curated** all-on set,
  defined in the catalogue, explicitly excluding `test-*` and
  `currency-poc-tb`.

---

## 1. Critical review of the composer design (Roc platforms)

Section references are to the design exploration, Part 3 unless
noted. "Must survive" items are the load-bearing decisions an
implementation may not trade away; shortcomings are ordered worst
first, each with what this plan does about it (§8 carries the
fixes into the composer's own implementation plan).

### 1.1 Details that must survive implementation

1. **Composition lives in platform production, never at app
   binding** (§3.4, §2.1). Roc's "platforms do not compose" is
   permanent upstream policy; the composer survives by producing
   _one ordinary platform_ before any app compiles. Any
   implementation that composes inside the app's platform slot
   contradicts the premise.
2. **Omission is the gate** (§2.1, §3.1, §3.3). The composed
   platform's effect surface is _exactly_ the union of selected
   caps because unselected caps are unnameable in the generated
   `pf.*` interface. Runtime wrappers are defence-in-depth for
   scoped grants, never the primary mechanism.
3. **Three-deep narrowing, no widening** (§3.3 — headed
   "Two-level" but enumerating three): composition (production
   time, enforced by omission) ⊇ declaration (app build time,
   checked against the manifest recorded in the platform bundle)
   ⊇ grant (install/run time, enforced in the host shim; scopes,
   budgets, per-app resources). A grant cannot add an omitted
   cap; a composition by itself provisions nothing — the
   provisioner (§2.3) turns a platform into a deployment.
4. **Grants are never manifest content** (§3.1). Runtime grants
   are §2.3 records, a different layer, applied later. Blurring
   this would bake operator decisions into immutable artefacts.
5. **Hash identity is more than the cap list** (§3.2):
   target class, the sorted cap `name@version` list, the
   catalogue version, **each selected cap's host-implementation
   version for that target**, and the pinned Roc nightly. Two
   manifests that hash equal _are_ the same platform; the
   composer is a pure function with a cache. A nightly bump
   honestly produces new identities (§3.8.3).
6. **Immutability + two-tier registry** (§3.2, §3.5): artefacts
   are immutable and hash-pinned; old profile versions stay
   published; named profiles (operator-curated, `name@version`)
   are the public identities, arbitrary compositions are
   content-addressed and unlisted, graduating on second use or
   operator promotion; the registry records which apps pin which
   composition (GC + promotion candidates).
7. **The contract must not fork** (§2.5, §2.2): the portable
   artefact is the catalogue contract — name, Roc API surface,
   declaration/grant syntax, metering schema — identical across
   host implementations in whatever language (Zig/Rust/Kotlin/
   JS). Metering is defined in the catalogue, counted in the
   host shim, stored where the app cannot write (§2.3).
8. **Deny, don't throttle** (§2.3): budget exhaustion is the
   capability's ordinary error value (`Err(QuotaExceeded)`),
   auditable, no partial writes; the one named exception is
   burst shaping on shared-fate resources. Budgets are
   soft/hard pairs (warn 80%, deny 100%) and are grant-record
   fields, not code.
9. **Catalogue independence is a composability precondition**
   (§3.6.3): a cap reaching into another cap's host internals
   breaks re-derivation on a nightly bump and must be marked
   non-composable until fixed.
10. **Stabilising disciplines** (§3.7, §3.8): one pinned nightly
    across the family; example apps as the nightly-bump
    rehearsal suite and as part of Composer v0's done test
    ("two manifests differing by one cap produce two platforms
    whose exposed surfaces differ by exactly that cap's modules,
    and an app declaring the omitted cap fails at build against
    the second platform").

### 1.2 Shortcomings — worst first

1. **Canonicalization is asserted, not specified (§3.2, §3.7).**
   The hash is the design's identity primitive, yet no canonical
   encoding, field ordering (beyond "sorted"), parameter
   encoding, or hash algorithm is given; §3.7 defers "manifest
   schema + canonical hashing" to Composer v0. Two honest
   implementations would hash the same composition differently
   and the pure-function/cache claim collapses. _Plan fix (§8.2):
   the canonical form is specified before any other composer
   code — canonical CBOR (the estate already maintains a
   canonical-CBOR discipline in `id-core`), BLAKE3 (already the
   estate's hash), golden test vectors published with the
   schema._
2. **Composition-time parameters vs the hash (§3.1 vs §3.2).**
   Parameters are "part of the platform's shape" but are absent
   from §3.2's enumerated hash inputs and have no schema, typing,
   or defaults anywhere. Whether two manifests differing only in
   a parameter collide is undecidable from the text. _Plan fix:
   parameters are inside the canonical manifest and therefore
   hash inputs, stated as a rule with the schema (§8.2)._
3. **Diamond and collision problems are silent (§3.1, §3.6.3).**
   The manifest is a flat list; nothing addresses two caps
   requiring different versions of a shared dependency cap,
   colliding `pf.*` module/function names or hosted functions,
   two caps claiming the same entrypoint, or transitive cap
   dependencies at all. The union property is stated as if union
   were always well-defined. _Plan fix (§8.3): catalogue entries
   gain `requires`/`conflicts` declarations; the composer
   resolves or refuses — refusal, never silent weakening, is the
   design's own stance (§3.1) extended to structure._
4. **`composable: yes/no` has no certification process and the
   wrong granularity (§3.1).** Who certifies "closed under the
   contract — no ambient authority, no escape hatches", by what
   test? Host implementations are per-target (§2.2), so a cap
   can be closed on server and leaky on Android, yet the field
   is a single boolean. §3.8's remediation (fix it in our
   variant) has no flip criteria. _Plan fix (§8.3): the field
   becomes a per-target-class map; `yes` requires the cap's
   conformance suite (§1.2.6) green on that target plus a
   recorded reviewer; `no` requires the reason, as today._
5. **Revocation is unaddressed and in tension with
   immutability (§3.2, §3.5).** Artefacts are immutable, old
   versions stay published, "pinned apps are unaffected" — and
   unused compositions are "garbage-collectable" with no
   criterion, and no yank path exists for a composition later
   found vulnerable. _Plan fix (§8.5): revocation lives at the
   grant layer, the one mutable layer the design already has —
   the registry marks a composition yanked; the provisioner
   refuses new grants against it and flags existing ones;
   artefacts stay immutable and honest. GC criterion stated:
   no pinning apps and no grants for a full nightly cycle._
6. **Generated-host security is a label, not a process (§3.5,
   §3.6.1).** Author-composed platforms are "marked unreviewed";
   profiles get "operator review" of unspecified content; under
   the universal-host shape the guarantee "rests on the
   interface boundary and host correctness" with all cap code
   present in the binary, and no verification regime (conformance
   tests, fuzzing, signing, provenance) is named for either
   shape. _Plan fix (§8.4/§8.5): profile publication requires the
   per-cap conformance suites + the §3.7 surface-diff test +
   artefact signing with the estate CA; the universal host is
   never a production artefact (the design's own recommendation,
   made a rule)._
7. **Catalogue versioning policy is absent (§2.2, §3.2).** Both
   `cap@version` and a catalogue version exist and both feed the
   hash; no rule says when each bumps, what a breaking vs
   additive API change does, or whether old cap versions remain
   composable. _Plan fix (§8.3): additive change = catalogue
   patch bump, cap version unchanged; breaking cap API change =
   new `cap@version`, old version remains composable until a
   recorded deprecation; catalogue major bump = the family event,
   rehearsed like a nightly bump._
8. **Cross-target conformance has no test regime (§2.2, §2.5).**
   The family's promise — same app code, same limits, any
   target — rests on per-target host implementations in four
   languages behaving identically, including metering units;
   nothing specifies the suite that proves it. _Plan fix (§8.4):
   each catalogue cap ships a conformance suite (seeded fakes per
   §2.6's shim discipline) that every host implementation must
   pass on its target before its version may appear in a hash._
9. **Profile governance is named, not constituted (§3.2, §3.5).**
   Operators curate and promote — but no graduation criteria
   beyond "a second app wants it", no naming authority or
   collision rules, no deprecation lifecycle, and no process for
   who may add catalogue entries at all. _Plan fix (§8.5): the
   registry record gains curator + review fields; graduation
   requires the §3.7 done test against the composition plus one
   operator review recorded in the registry — deliberately the
   lightest process that makes a name a supported promise._
10. **The manifest format does not exist (§3.1, §3.7).** No
    syntax, schema version, or malformed-manifest behaviour —
    yet the hash, the registry record, and the bundle-embedded
    manifest (§3.3's declaration check) all depend on it.
    _Plan fix: §8.2 designs it (TOML surface, canonical CBOR
    core, schema version inside the hash)._
11. **On-demand latency and capacity are two numbers and no
    policy (§3.5, §3.6.1).** Generation "seconds", bespoke build
    "minutes, cacheable"; no cache-miss latency target, no
    queue/timeout semantics, no definition of a "permitted
    composer" or composition authentication, and rate limits are
    mentioned without values. _Plan fix (§8.6): budgets stated
    against the estate's real build capacity (compile
    coordinator + Actions), with the same discipline as §9 of
    this plan._
12. **Declaration-checking mechanics drift between sections
    (§2.1 vs §3.3).** §2.1 fails ungranted declarations "at
    load"; §3.3 checks declaration at app build time against
    "the manifest recorded in the platform bundle" — whose
    format is undefined, and no checker tool (the `roc-cfw
sync` analogue) is assigned to anyone. _Plan fix (§8.2):
    the bundle embeds the canonical manifest bytes; the checker
    is a named Composer v0 deliverable, run at app build time;
    load-time failure remains as the backstop, per §2.1._
13. **Nightly-bump fate of arbitrary compositions is
    unspecified (§3.2, §3.6.3).** Every bump orphans every
    arbitrary composition's identity by construction; no expiry
    terms, no notice to pinning apps, no behaviour when
    re-derivation fails (cap broke or turned non-composable).
    _Plan fix (§8.5): arbitrary compositions carry a recorded
    "derived-under nightly N" and a re-derivation report per
    bump; failure to re-derive marks the composition stale in
    the registry — visible, never silent._
14. **Internal tensions to respect, not silently fix (§2.1→
    §3.6.2; §2.5→§3.8; §3.3's heading).** "One platform per
    target class" became "a profile family per target class"
    (portability now holds within a profile's cap set); §2.5's
    re-host default is superseded by §3.8's own-the-five where
    they differ, with the contract rule standing; §3.3's heading
    says two levels, its body three. An implementer reading any
    single section alone builds the wrong thing — this plan
    treats Part 3 as amended by §§3.8–3.9 throughout.
15. **The concave decision is open (§3.9.3).** Build our own
    implementation of the concave shape on the ID platform
    family (a), or adopt the days-old upstream as a variant base
    (b) — "costs differ by an order of magnitude", recorded as
    Drew's to decide. §8 plans the composer so either answer
    slots in (concave is a cap family + profile either way), and
    does not presume it.
16. **Composition-level metering is not revisited in Part 3
    (§2.3 vs Part 3).** Two caps over one underlying resource
    (two storage caps, one database) — double-counted, shared
    budget, or per-cap only? The text is silent. _Plan fix
    (§8.3): metering stays per-cap in the catalogue; the grant
    record (§2.3) gains an optional shared-pool field for caps
    the operator knows share fate — operator knowledge, not
    composer inference._

---

## 2. Critical review of the feature-composed binary model itself

The same composition idea applied to a Cargo workspace has its
own failure modes. Several are not hypothetical — they are
observable in today's tree (§0.1, §2.2).

### 2.1 Cargo feature unification traps

- Features are **additive and unified per build graph**: any
  crate in the graph enabling a feature enables it for every
  consumer in that build. The tree already demonstrates the
  hazard class twice over. (a) The vendored p2panda crates are
  excluded from the workspace precisely because membership would
  join their feature graph to the lock resolve
  (`p2panda-sync/test_utils → p2panda-store/test_utils →
sqlite → sqlx`), guarded today by a `cargo tree` test. (b)
  Dev-dependencies leak features into test builds: root dev-deps
  enable `id-core/test-utils` + `test-provider`,
  `id-caps/test-provider`, `id-sync/test-utils`; `id-store`'s
  dev-dep pulls the facade **with default features on** — so
  test builds never prove a minimal production graph.
- **A live forwarding gap exists today:** root `web` does not
  forward `id-web/sandbox`, yet `id-web/src/world_ws.rs` keys
  code on `id-web`'s own `sandbox` feature. Under root `web`
  alone those blocks compile out; under `--all-features` (CI's
  default) unification turns them on. The shipped Nix `id-web`
  build and a CI build can therefore differ in world-WS
  behaviour without anyone choosing that. This is the canonical
  failure the catalogue's per-profile verification (§9) exists
  to catch.
- No `resolver` is pinned in any manifest. Edition 2024 implies
  the v3 resolver, but a catalogue that reasons about feature
  graphs should say so in the manifest, not by implication.
- Features cannot subtract. A negative guarantee ("this binary
  contains no serve stack") is never a feature property; it is
  a **composition** property — which crates are linked at all.
  This is #5788 §4's rule and it survives intact: features gate
  optional dependencies _inside_ seam crates; profiles are
  defined by composition crates (§7.1).

### 2.2 Mutually exclusive alternatives — Drew's named case

Cargo has no exclusion mechanism: nothing stops `--features
store-a,store-b`. Stores are the instance Drew named, and the
port discipline (§0.1) sharpens it: two _authoritative_ backends
for the same port in one binary is a correctness hazard (which
one is the truth?), while multiple _read-only foreign_ sources
already coexist safely (`foreign_libsql` precedent — writes
inexpressible). The plan's rule (§4.4):

- Each port's authoritative backend is an **exclusivity group**
  with exactly one member per composition. Enforcement is
  three-layered: the build tool refuses a manifest naming two
  (§5); the composition crate carries a `compile_error!` guard
  for direct cargo invocations; CI's per-profile `cargo tree`
  assertions verify the shipped graph contains exactly one.
- Read-only/foreign adapters are ordinary non-exclusive
  features — coexistence is their point.
- The same pattern covers future either/or pairs (e.g. two
  group-encryption schemes, §4.2 note on BeeKEM/keyhive) —
  exclusivity is a catalogue field, declared per group, not a
  case-by-case argument.

### 2.3 Combinatorial test surface

Six root features today is 64 subsets; the §4 catalogue is ~25
features. The powerset is not testable and the plan does not
pretend otherwise. The testable units are: (a) each **named
profile**, fully (build, clippy, test suite, conformance); (b)
each **seam crate's features in isolation** (cargo-hack-style
`--each-feature` sweeps on seam crates only, on a slow cadence —
§9 budgets this); (c) per-profile `cargo tree` negative-guarantee
assertions (cheap, run always); (d) #5788 §6's powerset _spot_
checks on composed seam crates. Arbitrary compositions get (c)
plus the replay-oracle conformance run (§9) — the same journal,
every shipped composition, the same state hash — which is the
estate's existing cross-variant oracle, reused rather than
reinvented.

### 2.4 Boot-cost and size claims — what is actually known

Drew's expectation — the giant binary "might take a little
longer to boot" — is **unmeasured**, and this plan asserts no
number. What is known structurally: binary size alone is a weak
predictor of cold start; the real costs are eager initialization
(iroh endpoint setup, wasmtime engine compilation, web asset
embedding/route setup) and which of them a given command path
triggers. Today's tree already lazy-gates most of this by
command dispatch, but no per-variant startup measurement exists.
§9 therefore includes a measurement slice (cold start, idle
memory, binary size per profile, added to the #5685
per-interface matrix per #5788 §5) whose results set the budgets.
Until it runs, every boot/size claim in either direction is a
hypothesis. The one claim the evidence does support: the plain
profile's _linkage_ budget is enforceable today via §2.3(c)
assertions, whatever the timings turn out to be.

### 2.5 Support burden of many build variants

Every published artefact is a support surface: its own defect
reports, upgrade path, and conformance obligations. The estate
already pays a small version of this cost — three disagreeing
variant definitions (§0.1) produced the #6883 wrong-variant bug
and a stale Nix comment that misdescribes the default package's
features. The plan's controls, mirroring the composer registry:
only **named profiles** are published artefacts (§6); arbitrary
compositions are operator-built for a named served environment,
carry their composition hash in `--version` output and
diagnostics (§5.3), and are recorded in the composition registry
(§5.4) with their environment and owner — a defect report names
its composition the way a platform app names its platform hash.
Profiles graduate and retire by record amendment (§0.2), which
caps the supported set at what operators deliberately chose.

---

## 3. The minimal core — the plain ID binary

The plain profile is Drew's "dumbest client with a basic store
that could get a user up and running quickly". Working name:
profile **`plain`** (≈ #5788 L1 with the corrections of §0.3).

**In the plain binary:**

- Identity: key/keyset creation and recovery (the `Id`, `Key`,
  `Keyset` command families) — a client that cannot hold an
  identity is not a client.
- The basic store: the event log on redb (`events.redb`) and the
  blob store on iroh-blobs `FsStore` — today's shipped backends,
  unmodified. `--ephemeral` (MemStore) stays.
- Sync: iroh transport with gossip + mDNS + relay (today's
  defaults). No DHT (opt-in feature, §4).
- Client commands: `put/get/get-hash/put-hash/cat/show/peek/
find/search/list/tag/doc/claim/peers/petname/group` client
  halves, the REPL — i.e. everything except hosting. Per the
  F13.8 boundary (#3846), `serve` is the only hosting command;
  plain's `serve` is absent as a _capability_: following today's
  established pattern the subcommand may parse, but the build
  carries no serve implementation and dispatch says so.
- World **state**: journal replay and read/render of world state
  (the `world-state` half of today's `world`, §7.2), including
  the #1066 degrade contract — a plain client meeting a newer
  program renders read-only and issues the unsupported-version
  receipt; it never partially executes.

**Explicitly NOT in the plain binary:**

- wasmtime / program execution (no `sandbox-exec`) — the single
  largest dependency and the plain profile's defining omission;
- world _hosting_ and serving of any kind (no `serve`, no SSH
  world serving, no web UI, no gallery hosting);
- the directory's server roles, currency anything, importers;
- DHT, transports beyond the iroh defaults, MCP anything;
- platform composition (it is a served-environment capability,
  §8 — a plain client _consumes_ composed platforms as an app
  target, it does not run the composer).

The done-test for plain is #5788's L1 test, kept: a `cargo tree`
assertion that no serve, provider-host, or wasmtime code is
linked — a property of the artefact, checked in CI, not a
code-review promise.

---

## 4. The feature catalogue for pkgs/id

Conventions: a catalogue entry is a root-level feature name (the
handle served environments use), the seam crates it forwards to,
what it adds, its requirements, and its exclusivity group if
any. Status marks what exists today: **shipped** (feature exists
and works), **seam** (the code/seam exists; the feature wiring is
the work), **designed** (a design record exists; implementation
is a slice), **placeholder** (catalogued so compositions can
name it; no design yet beyond the programme reference). Profiles
are §6.

### 4.1 Core (present in every profile; not features)

Identity + keysets; event log (redb); blob store (iroh-blobs);
graph index; sync base (iroh + gossip + mDNS + relay); client
command set; REPL; world state/replay (`world-state`, §7.2 —
core because the degrade contract requires every client to
render worlds it cannot execute).

### 4.2 Opt-in features

| Feature                                                                                      | Adds                                                                                                                             | Requires                         | Status                                          |
| -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- | ----------------------------------------------- |
| `sandbox-exec`                                                                               | wasmtime program execution (today's `sandbox`, renamed in effect: forwards `id-worlds/sandbox` + markers)                        | `world-state`                    | shipped (as `sandbox`)                          |
| `serve`                                                                                      | hosting: file/world serving, the `serve` command's host halves (today unconditional code with sub-gates; becomes a real feature) | —                                | seam                                            |
| `web`                                                                                        | Axum web UI (`id-web`)                                                                                                           | `serve`, `world-state`           | shipped                                         |
| `ssh-serve`                                                                                  | world serving over SSH (today's `ssh`, russh in `id-worlds`)                                                                     | `serve`, `sandbox-exec`          | shipped (as `ssh`)                              |
| `dht`                                                                                        | Mainline DHT address lookup (today's `dht`)                                                                                      | —                                | shipped                                         |
| `directory`                                                                                  | home-server directory: views, auth, mail, invites (`id-directory`)                                                               | `serve`                          | seam (crate ships; root feature does not exist) |
| `currency`                                                                                   | currency registry + gateways (`id-currency-registry`, facade currency modules)                                                   | `serve`                          | seam                                            |
| `roc-composer`                                                                               | the Roc platform composer (§8): manifest → composed platform → registry                                                          | `serve`, `sandbox-exec`          | designed (§3.7 v0 scope)                        |
| `mcp-server`                                                                                 | standalone MCP server binary surface over the `id-caps` MCP bridge (book: library-only today, no server binary exists)           | `serve`                          | designed                                        |
| `import-notion`                                                                              | Notion importer CLI wiring (`id-import-notion`, orphan library today)                                                            | —                                | seam                                            |
| `import-obsidian` / `import-evernote` / `import-logseq` / `import-joplin` / `import-onenote` | the #2048-model importer family                                                                                                  | —                                | designed (#5845–#5848 family)                   |
| `transport-mcp`                                                                              | MCP-door transport adapter (#13929)                                                                                              | —                                | designed                                        |
| `transport-email`                                                                            | email transport adapter (asymmetric levels per #13929)                                                                           | —                                | designed                                        |
| `transport-mqtt`                                                                             | MQTT adapter + device bridge (#2343)                                                                                             | —                                | designed                                        |
| `transport-irc`                                                                              | IRC relay adapter                                                                                                                | —                                | designed                                        |
| `transport-overlay`                                                                          | Tor/Veilid-class overlay adapters (TODO-list candidates; licence-screened per item)                                              | —                                | placeholder                                     |
| `transport-sneakernet`                                                                       | bundle-carried offline transport (ID-graph D4)                                                                                   | —                                | designed (#13889)                               |
| `group-encryption-keyhive`                                                                   | BeeKEM/keyhive group encryption as the alternative scheme                                                                        | exclusivity group `group-crypto` | placeholder (TODO long-term)                    |

### 4.3 Profile meta-features

For each named profile (§6) a root meta-feature of the same name
(`plain` is the absence of meta-features plus core; `full`,
`basement`, …) that expands to the profile's feature set.
Meta-features contain no code gates anywhere — they exist so a
served environment has one handle, and so `cargo tree`
assertions can key on them. This is the one deliberate deviation
from #5788 §4's "features are never the variant definition": the
_definition_ still lives in the profile's composition crate
(§7.1); the meta-feature is a generated mirror of it, and §9's
CI asserts the two agree — drift between them fails the build
instead of shipping silently (the §0.1 three-sources-of-truth
failure, closed mechanically).

### 4.4 Exclusivity groups

| Group          | Members                                                                                                                                                         | Rule                                                                                                                   |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `store-record` | `store-redb` (core default), `store-turso` (libSQL port adapter — §0.1's declared profile, wired by §10 S6), `store-postgres-http` (#5805), `store-d1` (hosted) | exactly one authoritative record-store backend per composition                                                         |
| `store-events` | redb file (core), Turso (designed, hosted tenancy #9083)                                                                                                        | exactly one                                                                                                            |
| `group-crypto` | p2panda-encryption scheme v1 (core), `group-encryption-keyhive`                                                                                                 | exactly one per composition; worlds name their scheme in their manifest, per the vendored scheme-negotiation precedent |

Store features select the backend **behind the existing port
traits** — they never introduce a second store abstraction. The
browser target's IndexedDB adapter is a target-specific
composition (the TS adapter in `browser/`), catalogued as
`store-indexeddb` with target `wasm32` only; it is not a member
of the host-binary groups.

---

## 5. From a composition manifest to a built binary

### 5.1 The build manifest

A served environment's build is named by a TOML manifest in the
repo (or the operator's deployment repo), e.g.:

```toml
# composition manifest — schema v1
name = "basement-store-node"        # absent for arbitrary compositions
target = "x86_64-unknown-linux-musl"
profile = "basement"                # optional base profile
features = ["store-turso", "dht"]   # additions / group selections
```

Rules: a manifest names a base profile _or_ an explicit feature
list (not both ambiguously — additions to a profile are stated
as additions, and the resolved set is what's hashed); every
feature must exist in the catalogue (§4); exclusivity groups
resolve to exactly one member; `test-*` and harness features
are unrepresentable in a manifest.

### 5.2 The composition hash (the composer's §3.2, applied to ourselves)

Identity = BLAKE3 over the canonical CBOR of: schema version;
target triple; the **sorted resolved feature list with each
feature's owning-crate versions**; the catalogue version; the
workspace lockfile hash; the pinned Rust toolchain. Two
manifests that hash equal are the same build — the build is a
pure function with a cache (§9). Grants, config, and data are
never manifest content (the §3.1 rule, kept): a composition
decides what a binary _can_ do; what it _does_ is runtime
configuration, a different layer.

### 5.3 The build tool

A small workspace tool (working name `id-compose`, §7.3) that:
validates a manifest against the catalogue (unknown features,
group violations, unrepresentable features — refusing, per the
composer's stance); computes the composition hash; invokes
cargo with the resolved `--no-default-features --features …`
set against the profile's composition crate; stamps the
artefact (build-info env → the binary's long `--version`, which
already carries provenance today, gains `composition <hash>`
and the profile name); and writes the registry record (§5.4).
Bespoke builds run through the fleet compile coordinator /
Actions (§9), never as an unrecorded local act for a served
environment.

### 5.4 The composition registry

Starts as a versioned JSON/TOML registry in the repo (the
composer registry's D1-table equivalent is a later step, per
§3.5's "sufficient at this scale" judgement applied to
ourselves): one record per composition — canonical manifest,
hash, target, artefact hash + URL, resolved feature list
rendered for humans, profile name if graduated, served
environment + owner, pinning deployments count. Yank semantics
per §1.2.5: a yanked composition gets no new deployments; the
artefact stays immutable and the record says why.

---

## 6. Named build profiles

The four #5788 levels, retained as the registry's founding
profiles, plus the served-environment examples Drew named.
Profile definitions live in composition crates (§7.1); the
table is their human rendering.

| Profile                           | ≈ #5788            | Shape                                                                                                                                                                                                         | Published artefact                                                                                                                     |
| --------------------------------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `plain`                           | L1 (corrected, §3) | core only: dumb client + basic store                                                                                                                                                                          | yes — the user download                                                                                                                |
| `full`                            | L3                 | **curated all-on**: core + `sandbox-exec`, `serve`, `web`, `ssh-serve`, `dht`, `directory`, `currency`, `roc-composer`, `mcp-server`, importers, all non-exclusive transports, default store group selections | yes — the backend/server build; slower boot accepted, measured per §9                                                                  |
| `basement`                        | L2                 | headless service: core + `serve`, `sandbox-exec`, `directory`, `dht`; no presenters (no `web`, no `ssh-serve`)                                                                                                | built by operators                                                                                                                     |
| `embedded`                        | L0                 | library composition, no binary: core minus CLI/REPL, provider link only                                                                                                                                       | library only                                                                                                                           |
| `composer-host` (served example)  | —                  | `basement` + `roc-composer` + `store-turso`: the platform-composition environment                                                                                                                             | operator build, hash-registered                                                                                                        |
| `alt-store-node` (served example) | —                  | `full` with `store-record` switched to `store-turso`: the alternate-distributed-store environment                                                                                                             | operator build, hash-registered                                                                                                        |
| `hosted-edge` (served example)    | —                  | hosted edition shape: `web` + `directory` + `store-d1` against the hosted-worker surface                                                                                                                      | aspirational — gated on the hosted-edition slices (#2285/#3403 line); catalogued now so the composition has a name when the seams land |

"Full" is a profile, not `--all-features` (§0.3): if a future
feature is not in `full`, that is a catalogue decision recorded
in the registry, visible — never an accident of a flag list.

---

## 7. Crate layout changes vs today's structure

Deliberately small: the #3447 split already produced the seams.
The changes are wiring and two splits, not a re-architecture.

### 7.1 Composition crates (per #5788 §4)

One thin crate per published profile — `crates/id-client`
(plain), `crates/id-node` (full), `crates/id-headless`
(basement), `crates/id-embedded` (embedded, library) as working
names. Each holds only: its manifest (seam-crate dependencies
with `default-features = false` + the profile's explicit feature
set), its binary/library entry point, and its conformance test
target. The root `id` package becomes the legacy facade it
already half-is: it keeps serving the `full` profile during
migration (§10 S5 re-expresses it as the `id-node` composition),
and shrinks thereafter. No capability code moves into a
composition crate, ever.

### 7.2 The `world` split

Today's `world` feature conflates state and execution-adjacent
surface (§0.3). Split into `world-state` (journal, replay,
rendering, receipts — core, §4.1) and the execution surface,
which folds into `sandbox-exec` (root + `id-core` + `id-cli` +
`id-worlds` feature names aligned). This is the plan's most
invasive seam work and its critical path: `serve.rs` alone
carries ~41 `world` cfg sites. The split is behaviour-preserving
for the default build (default = `world-state` + `sandbox-exec`
during migration; the five-minute path then becomes the plain
profile's first build).

### 7.3 New crates (only where a feature needs a home)

- `crates/id-composer` — the Roc platform composer (§8): library
  - a `roc-compose` bin target. Depends on `id-core`
    (canonical CBOR, hashing) and the catalogue; deliberately not
    on `id-worlds` — the composer produces platforms; it is not
    itself a world program.
- `tools/id-compose` (or an xtask) — the §5.3 build tool. Kept
  out of the shipped workspace's default members (the §2
  survey's default-members lesson: anything non-default needs
  compensating CI wiring, which §9 provides explicitly).
- No new store crates: store backends land inside `id-store`
  behind the port traits, per §4.4.

### 7.4 Feature hygiene (small, immediate, independent of the rest)

Pin `resolver = "3"` explicitly in the workspace manifest;
forward `id-web/sandbox` from root `web` (or remove the dead
feature — the §8/S2 slice decides from the world-WS evidence);
align member-crate defaults with facade usage or document the
divergence per crate; fix the flake's stale default-features
comment and generate the Nix/just/build.sh variant definitions
from the profile manifests (§4.3) so there is one source of
truth.

---

## 8. The Roc platform composer in Rust (the `roc-composer` feature's substance)

The composer is planned as a subsystem of `pkgs/id`
(`crates/id-composer`, §7.3) because its catalogue, registry, and
provisioner are siblings of the ID estate's own (§3.5: the
registry "is the provisioner's sibling, not a new system"), and
because the ID binary's served profiles are its first operator.
Build order honours §2.6/§3.7: nothing here moves before the
shim prototype proves one hand-built platform; Composer v0's
scope is §3.7's, verbatim.

### 8.1 Components

1. **Catalogue** — the six-field entries (§2.2 + §3.1) as
   versioned data in the repo: name@version, Roc API surface
   (the module source the generator emits from), declaration +
   grant syntax definitions, per-target host implementations
   with their versions, metering schema, composability map
   (§8.3). The catalogue has its own version, bumped per §1.2.7's
   rules.
2. **Manifest** (§8.2) — parse, validate, canonicalize, hash.
3. **Generator** — from a resolved manifest, emit the platform
   package: one generated `pf.*` Roc module per selected cap
   carrying the catalogue's API surface and error unions
   verbatim (text generation from catalogue data — no compiler
   API is needed or used; the undocumented Zig embedding path
   and its adapter are a separate, later concern for
   introspection tooling, not for generation), plus the bundle's
   embedded canonical manifest bytes (the §3.3 declaration
   check's input).
4. **Host assembly** (§8.4) — universal host + generated config
   for arbitrary compositions; bespoke compile at profile
   publication.
5. **Registry + publication** (§8.5) — bundle publication at a
   hash-pinned URL (estate R2), registry records beside the
   provisioner's grant records.
6. **Checker** — the `roc-cfw sync` analogue: verifies an app's
   declarations against the bundle's embedded manifest at app
   build time (§1.2.12).

### 8.2 Manifest schema and canonical hashing (fixes §1.2.1/2/10/12)

- Surface syntax: TOML (operator-written, diffable). Core form:
  canonical CBOR via `id-core`'s existing canonical-CBOR
  discipline; hash: BLAKE3 over the core form.
- Canonical manifest contents — **all** of: schema version;
  target class; the sorted list of `cap@version`; per-cap
  composition-time parameters (inside the hash — §1.2.2's rule);
  catalogue version; per-target host-implementation versions
  for each selected cap; the pinned Roc nightly. Golden vectors
  ship with the schema; two implementations hashing differently
  is a test failure, not a discovery.
- Malformed/unknown-cap manifests are refused with the
  offending names listed (the §2.1 load-failure style, at
  composition time).

### 8.3 Catalogue rules the design left open (fixes §1.2.3/4/7/16)

- Entries gain `requires` (cap@version ranges) and `conflicts`
  declarations. Resolution: the composer computes the closure;
  a diamond that cannot resolve to one version per cap is
  refused with the conflict named. Module/function name
  collisions across selected caps are a catalogue defect,
  caught by a catalogue lint (the generator emits into
  per-cap namespaces by construction, so collisions surface as
  duplicate hosted functions at generation time and refuse the
  composition).
- Composability is a **per-target-class map**, not a boolean.
  `yes` on a target requires: the cap's conformance suite
  (§8.4) green on that target, no ambient authority by
  construction review, and a recorded reviewer in the catalogue
  entry's history. `no` requires the reason (today's rule).
- Versioning: additive API change = catalogue patch bump;
  breaking cap change = new `cap@version` (old stays
  composable until a recorded deprecation with a nightly-cycle
  notice); catalogue major = family event, rehearsed like a
  nightly bump across the example apps.
- Metering stays per-cap; the grant record gains an optional
  shared-pool field for caps an operator knows share an
  underlying resource (§1.2.16).

### 8.4 Hosts and conformance (fixes §1.2.6/8)

- **Universal host** per target class embedding every
  composable cap implementation; a composition generates the
  interface package + host config. Used for arbitrary/on-demand
  compositions only — the requesting/prototyping path.
- **Bespoke compile** at profile publication: code-generated,
  linked per composition, unselected code absent. Profiles —
  the artefacts production runs — carry the stronger property
  (§3.6.1's recommendation, adopted as a rule).
- **Per-cap conformance suites**: each cap ships a suite
  (seeded-fake discipline from §2.6's shim prototype) that every
  host implementation of that cap must pass on its target
  before its version may appear in a composition hash. The
  suites are also the cross-target oracle: same suite, every
  target, identical metering outcomes.
- Profile publication additionally requires the §3.7
  surface-diff test and artefact signing under the estate CA
  (the fleet CA / IdP-interim-CA line already in service).

### 8.5 Registry, graduation, revocation (fixes §1.2.5/9/13)

- Registry record per composition, §3.5's fields plus: curator,
  review record (for profiles), yank state, derived-under
  nightly, re-derivation status.
- Graduation (arbitrary → profile): second app wants it **or**
  operator promotion (the design's disjunction), **plus** the
  §3.7 done test run against the composition and one recorded
  operator review. Names are minted only here.
- Revocation: yank at the registry; the provisioner refuses new
  grants against a yanked composition and flags existing
  grants to their operators; artefacts remain immutable
  (history stays honest), apps already granted keep running
  until their operator acts — stated plainly because it is the
  honest limit of revocation under immutability.
- Nightly bumps: arbitrary compositions are re-derived and the
  result recorded per composition (re-derived / stale with
  reason); pinned apps are notified through the registry
  record, never surprised.

### 8.6 Capacity and latency (fixes §1.2.11)

Composition requests are build jobs on the estate's real
capacity: the compile coordinator (placement + budgets) and
Actions, with Blacksmith's free minutes as the burst pool —
the same budgets §9 states for binary compositions. Targets,
stated as targets to be measured against, not promises:
universal-host composition (generation only) in seconds;
bespoke profile publication in minutes, cacheable; a cache hit
(composition hash already published) costs nothing. Composer
access is operator-authenticated at the start (the design's
"operator-only composition is the starting state", §3.5);
per-author rate limits arrive with self-service composition,
with values set from measured capacity, not before.

### 8.7 Composer v0 scope (per §3.7, restated as the slice boundary)

Manifest schema + canonical hashing (§8.2); interface-package
generation for the storage trio on the Cloudflare shape
(`cap.sql.d1`, `cap.objects.r2`, `cap.kv`); profile-only
composition (operator-submitted manifests; no arbitrary
composition yet); registry record written beside the
provisioner's grant records. Done test: §3.7's — two manifests
differing by one cap yield platforms whose surfaces differ by
exactly that cap's modules; an app declaring the omitted cap
fails at build; the v0 profiles' example apps build and run.

---

## 9. Build and CI strategy for the matrix

- **What CI runs, per change:** the profiles touched by the
  change's crates (path-filtered), each with build + clippy +
  its test suite + `cargo tree` negative-guarantee assertions +
  the generalized forbidden-dependency guards (the no-SQLite
  test is the template: per-profile forbidden lists, e.g. plain
  forbids wasmtime/russh/axum in its graph).
- **Slow cadence (nightly/weekly, coordinator-scheduled):**
  seam-crate `--each-feature` isolation sweeps; the replay
  oracle across **all** profiles (same journal → same state
  hash); powerset spot checks per #5788 §6; boot/size
  measurement per profile into the #5685 matrix (cold start,
  idle memory, binary size, replay throughput), with budgets
  set from the first measurements and regressions treated as
  defects.
- **Capacity honesty:** a full workspace build with wasmtime is
  minutes-scale; the matrix above is sized to the estate's real
  budgets — the compile coordinator's org caps
  (developing-today 20, developing-today-forks 20, dezren39 40
  concurrent placements) and Blacksmith's 3,000 free
  minutes/month — by keeping per-change runs profile-scoped and
  pushing powerset-class work to the slow cadence. If measured
  build times break that budget, the cadence narrows before
  coverage silently does.
- **Caching:** builds are keyed by composition hash (§5.2);
  identical hash = artefact reuse, for binaries exactly as for
  composed platforms. sccache/cargo caching is an accelerator,
  never the identity mechanism.
- **The `--all-features` habit is retired as a CI default** and
  replaced by the `full` profile for maximal-union testing,
  with test-only features enabled only in the test harness
  configurations that name them. (Today's posture tests a graph
  no user runs, and can mask per-profile failures — §2.1's
  `id-web/sandbox` gap is the standing example.)

---

## 10. Phased slice list (queue-packet sized; plan output, not yet queued)

Each slice is sized to one queue packet (one session, one PR,
evidence on the tracking issue). Dependencies stated; S1–S2 are
independent of the rest and unblock everything.

- **S1 — Catalogue + manifest + hash, as a design-of-record
  amendment.** The §4 catalogue, §5 manifest schema + canonical
  hashing spec, and the §0.2 reconciliation written into the
  variants record line (amendment to #5788's record, per its own
  §8 rule). Done test: the catalogue validates against today's
  tree (every shipped feature appears, every §4 row names its
  crates).
- **S2 — Feature hygiene.** §7.4: resolver pin; the
  `id-web/sandbox` forwarding question decided from evidence and
  fixed; member-default divergences documented; Nix/just/
  build.sh variant definitions generated from one profile
  source. Done test: the Nix `id-web` build and a cargo `web`
  build produce identical feature graphs for `id-web`.
- **S3 — The `world` split.** §7.2: `world-state` separated
  from the execution surface across root/`id-core`/`id-cli`/
  `id-worlds`. Done test: default build behaviour-identical
  (full existing suite), and `--no-default-features --features
world-state` builds with the degrade contract intact.
- **S4 — The plain profile.** `crates/id-client` composition
  crate (§7.1) + negative-guarantee CI (§3). Done test: plain
  builds, its `cargo tree` assertions pass, the five-minute
  path script points at it, a fresh-data-dir walkthrough
  (init identity → put → sync → get against a full node)
  passes.
- **S5 — The full profile + composition stamping.** Root
  re-expressed as the `id-node` composition (§7.1); long
  `--version` gains the composition hash + profile name (§5.3).
  Done test: the shipped default artefact is the composition
  crate's output; its hash is reproducible from the manifest.
- **S6 — Store exclusivity, first alternate.** The port
  adapter wired: `store-turso` as a real record-store backend
  behind `id-core::port` (§4.4), the three-layer exclusivity
  enforcement (§2.2), conformance via the existing
  store-adapter profile suites. Done test: the `alt-store-node`
  profile builds and passes the store conformance suite; a
  two-member manifest is refused by tool, guard, and CI alike.
- **S7 — `id-compose` build tool + registry.** §5.3/§5.4:
  validate → hash → build (via coordinator) → stamp →
  registry record; the two §6 served examples produced through
  it. Done test: both example compositions build from clean
  manifests and their registry records verify against the
  artefacts.
- **S8 — CI matrix + cross-profile conformance.** §9's
  per-change and slow-cadence runs wired (flake checks +
  coordinator schedule), including the replay oracle across
  all profiles. Done test: a deliberately introduced seam
  divergence is caught by the oracle in CI.
- **S9 — Measurement + budgets.** §2.4/§9: cold start, idle
  memory, size per profile into the #5685 matrix; budgets set
  from measurements and recorded. Done test: the matrix rows
  exist for every profile and a regression past budget fails
  the slow-cadence run.
- **S10 — Roc Composer v0.** §8.7's scope, against the
  platform programme's own build order (§2.6 shim prototype
  precedes it — that prototype is the platform programme's
  slice, tracked there; S10 starts when it lands). Done test:
  §3.7's, verbatim.
- **S11 — Roc Composer v1.** Arbitrary compositions via the
  universal host; bespoke compile at graduation; yank +
  re-derivation reporting (§8.5); per-target composability in
  force (§8.3). Done test: an arbitrary composition is
  requested, served by the universal host, graduated, and
  re-published bespoke — with registry records at each step.
- **S12 — Transport adapter features.** First adapter from
  #13929's order (bridge/HTTP) as a §4 catalogue feature with
  its declared level; subsequent adapters as their own slices.
  Done test per adapter: its #13929 level contract exercised
  against a plain and a full node.
- **S13 — Hosted/alt-store served profiles.** `store-d1` +
  `hosted-edge` (§6) when the hosted-edition seams land.
  Gated, not scheduled: catalogued so the composition has a
  name; slices are filed when the seams exist (the #5788 §9
  discipline).

## 11. Open decisions (recorded, not taken here)

- **Concave (a) vs (b)** — §1.2.15: Drew's, already open on the
  design record. The composer plan is neutral to the answer.
- **Arbitrary compositions in production:** this plan's
  recommendation — permitted for served environments when
  hash-registered and operator-granted (the composer's own
  posture), with named profiles the default and the only
  _published_ artefacts. Needs Drew's ratification at S1,
  since it is the point where this plan amends #5788.
- **Profile graduation authority:** the registry curator role
  (§1.2.9) — in practice Drew/operator discretion today;
  formalizing further is deferred until a second operator
  exists.

---

_Record ends. Companion issue: filed with this record as the
tracking issue for §10's slices; the prioritization is also
noted on the composer design's record (PR #14029) and on #5788._
