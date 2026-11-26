# Roc platform composer — amendment to the composed-platform design

**Date:** 2026-11-24 · **Lane:** ROC-PLATFORM-COMPOSER
**Direction:** Drew, 2026-11-23 ~20:41 CST (the composer), extended ~20:44 CST
(own the five platforms, example apps, one pinned nightly).
**Amends:** "Roc Platforms Deep-Dive + A Composable Platform Family for ID Apps"
(the ROC-PLATFORMS-DESIGN exploration, 2026-11-23). Its repo copy lands as
`doc/2026-11-24T02-09-38Z_research_roc-investigation/platform-design-exploration-2026-11-23.md`
via PR #14012 (docs-migration slice 9 under #13877 — in flight when this
amendment was written, so the amendment lands as its own record rather than
inside that snapshot). The identical text is appended as Part 3 of the working
note in `~/workspace/roc-investigation/`.
**Programme context:** the wider frame is the ID design programme for worlds,
capabilities and permissions (#5955); this amendment belongs to the platform
design record above, not to a separate design.

The amendment follows, verbatim as appended to the design note.

---

# Part 3 — Amendment (2026-11-23, later): the platform composer

**Drew's amendment, in substance:** _"we also would combine multiple platform
capabilities into a single platform, compile it, and then offer that. Some
way to dynamically do that."_

§2.1 fixed composition at platform-construction time and left the mechanism
at "the platform binary is built (or configured) per deployment profile."
This amendment names the mechanism: a **platform composer** — a factory that
takes a manifest and produces a platform. It extends §2.1; it does not
replace it. Where the two differ in emphasis, the difference is stated in
§3.6 rather than smoothed over.

## 3.1 The composer: manifest in, platform out

- **Input:** a composition manifest — target class plus a list of catalogue
  entries by name@version (`cap.sql.d1@1`, `cap.kv@1`, …), each optionally
  carrying composition-time parameters that are part of the platform's
  shape (which entrypoints the host presents, in the §1.8 sense). Runtime
  grants are never manifest content: grants are §2.3's records, a different
  layer, applied later.
- **Output:** one ordinary Roc platform, in the only form Roc knows: a
  platform package — the generated Roc interface (the `pf.*` modules for
  exactly the selected caps, carrying the catalogue's API surfaces and
  error unions) plus a host that implements exactly those hosted
  functions — published as a versioned bundle at a URL, hash-pinned, so an
  app targets it in its header exactly like basic-cli or any other
  platform.
- **The union property:** the composed platform's effect surface is
  _exactly_ the union of the selected caps' surfaces. Nothing else is
  exposed, because nothing else is in the generated interface — §2.1's
  omission gate, applied at construction time. An app on this platform
  cannot name a capability the manifest didn't select.
- **Composability is a catalogue property, not a composer feature.** The
  union property holds only for caps whose host implementations are
  _closed under the contract_ — no ambient authority, no escape hatches.
  The Cloudflare platform's raw `Js` escape program (§1.6) is the
  counterexample: a cap carrying one cannot be certified composable.
  Catalogue entries therefore gain a **sixth contract field** (§2.2):
  _composable: yes/no_, with the reason when no. The composer refuses
  non-composable caps rather than silently weakening the union property.

## 3.2 The dynamic part: composition as a cached function of the manifest

- **Capability-set hashing.** A composition's identity is a hash of its
  canonical manifest: target class, the sorted list of cap name@versions,
  the catalogue version, each selected cap's host-implementation version
  for that target, and the pinned Roc nightly. Two manifests that hash
  equal _are_ the same platform — the composer is a pure function with a
  cache. The first request builds and publishes; every later identical
  request is a cache hit and costs nothing. This is the content-addressing
  discipline the estate already uses elsewhere; here, the hash is the
  platform's name.
- **Profiles vs arbitrary compositions.** Unbounded arbitrary composition
  would make every app its own platform (§3.6 names the cost). The
  registry therefore has two tiers:
  - **Named profiles** — operator-curated manifests published under a name
    - version (`id-server/storefront@2`, `id-android/field-reader@1`), per
      target class / app family. Profiles are the public identities apps are
      written against. A profile is what §2.1's "one id platform per target
      class" becomes in practice: a small, stable set of named points in
      composition space, not one point.
  - **Arbitrary compositions** — any manifest a permitted composer
    submits, published content-addressed (by hash) and unlisted by
    default. An arbitrary composition _graduates_ to a named profile when
    a second app wants it or an operator promotes it; until then it is a
    build artefact, not a product surface.
- **Proliferation control.** Three brakes: (1) the cache makes duplicates
  free, so a variant's cost is paid once; (2) graduation is the only path
  to a name, and names are what humans must support — the nightly-bump
  chore (§2.5) is priced per _profile_, while arbitrary compositions
  re-derive from the catalogue on a bump rather than being maintained;
  (3) the registry records which apps pin which composition, so unused
  compositions are garbage-collectable and heavily used arbitrary ones
  surface as promotion candidates.

## 3.3 Two-level narrowing

Composition and grant are different verbs at different times, and the
amendment keeps them so:

1. **Composition (platform production time)** decides what the platform
   _can_ do — the ceiling of the ceiling. Enforced by omission: unselected
   caps are unnameable.
2. **Declaration (app build time)** — the app declares the subset of the
   platform's surface it uses (RocRay-style, §2.2). Declaration ⊆
   composition, checked against the manifest recorded in the platform
   bundle.
3. **Grant (install/run time)** — the operator's grant record (§2.3)
   narrows further: scopes, budgets, per-app resources. Enforced in the
   host shim exactly as §§2.3–2.4 design it — ports unregistered, bindings
   unmapped, budgets denied.

An app can only use what survives all three narrowings, and neither lower
level can widen an upper one: a grant cannot add a cap the composition
omitted (the functions do not exist), and a composition cannot grant
anything by itself — a composed platform with no grant record provisions
nothing, because the provisioner (§2.3) is what turns a platform into a
deployment.

## 3.4 The static-binding tension, stated plainly

Roc binds an app to its platform when the _app_ compiles. Nothing in this
amendment changes that, and the word "dynamic" must not be allowed to
imply otherwise:

- The dynamism lives entirely in **platform production** — a new
  composition can be produced on demand, in one build, and published
  immediately — which happens _before_ any app builds against it.
- An app that wants a different capability set is re-pointed at a
  different platform artefact and **rebuilt**. There is no runtime
  re-composition of a running app, no hot-swapping a platform under a live
  process, and no app-visible difference between a composed platform and a
  hand-written one. That last point is the design's strength: composition
  is a factory property, not a language feature, so it needs no upstream
  Roc support and leaves upstream's "platforms do not compose" policy
  untouched — the policy governs what an _app_ binds to (one platform),
  and a composed platform is one platform.
- **Adjacent substrate:** Boswell's multi-entrypoint host work (§1.8) is
  what makes one host binary practical here — the interpreter-shim adapter
  already generates per-symbol entrypoints forwarding into one host, which
  is the shape a composed host needs (many narrow hosted surfaces, one
  binary). The November Zulip window also surfaced **roc-platform-patcher**
  ("alter & tailor platforms": middleware injected into a platform to
  observe or alter effects) — prior art pulling the opposite direction,
  tailoring an existing platform after the fact. The composer is
  construction-time and the patcher is interception-time; they compose
  with each other (a composed platform can still be patched for
  observability), but only the construction-time route gives the omission
  property.

## 3.5 Offering: publication, discovery, and who may compose

- **Registry shape:** one record per composition — the canonical manifest,
  its hash, target class, the bundle URL + artefact hash, the cap list
  rendered for humans ("what apps on this platform can do" — RocRay's
  permissions chapter as a registry page), profile name/version if
  graduated, and the pinning-app count. The registry is a catalogue-shaped
  artefact in our own estate; a D1 table plus a static page is sufficient
  at this scale. It is the provisioner's sibling (§2.3), not a new system.
- **Naming and versioning:** a profile is `name@version`, where a version
  bump means the manifest or any cap@version in it changed. The old
  version stays published; pinned apps are unaffected, because platform
  artefacts are immutable and hash-pinned. Arbitrary compositions are
  named by their hash alone until graduated.
- **Who may compose — the trust difference, spelled out.** Composition
  sets a ceiling, not a grant (§3.3), so letting app authors compose is
  safe under the grant model in the narrow sense: an author-composed
  platform cannot give an app anything the operator hasn't granted, and
  the provisioner still creates the resources. Two real differences
  remain. (1) _Surface accountability_ — an operator-composed profile is a
  reviewed artefact; an author-composed platform is unreviewed, so its
  registry record is marked as such, and it can never become a profile
  without operator review. (2) _Resource pressure_ — arbitrary composition
  lets authors mint platform variants at will, a build/registry load
  problem, answered by the cache and GC above plus rate-limiting
  composition requests per author. **Default posture: operators compose
  profiles; app authors may request arbitrary compositions, which run only
  under operator grants and are promoted only by operators.** Self-service
  composition with operator-held grants is the end state; operator-only
  composition is the starting state.

## 3.6 Where this amendment strains the existing design (named, not smoothed)

1. **"Built (or configured)" — §2.1 left it open; the composer forces the
   question.** Mechanically, "compile a platform" can mean (a) a bespoke
   host binary per composition, code-generated and linked — the binary
   literally lacks unselected capability code; or (b) one universal host
   per target class embedding every cap implementation, where composition
   generates only the interface package and a host config. Shape (b) is
   what makes on-demand composition cheap and fast, and omission still
   holds at the Roc interface level — but the unselected code is _present
   in the binary_, so the guarantee rests on the interface boundary and
   host correctness, not on absence. Shape (a) is the stronger artefact
   and the slower factory. **Recommendation:** (b) for arbitrary/on-demand
   compositions; (a) at profile publication time — a profile, once named,
   is compiled bespoke, so the platforms apps actually run in production
   carry the stronger property, and the dynamic path is the
   requesting/prototyping path. Drew's sketch says "compile it, and then
   offer that": under this recommendation the _offered_ (profile) artefact
   is compiled per composition, while the on-demand cache artefact is
   interface-composed. If (a) is wanted everywhere, the cost is a real
   build per composition (minutes, cacheable) rather than a generation
   step (seconds) — a capacity decision, not a design blocker.
2. **§2.1's "one id platform per target class" becomes "a profile family
   per target class."** The portability promise — "only the platform line
   in its header changes" — now holds _within a profile's cap set_: an app
   using `cap.render2d` cannot re-point at a profile that omits it. That
   was already true of per-deployment subsets; profiles make it explicit
   and enumerable instead of deployment-local. The catalogue remains the
   portable artefact; profiles are named, comparable points in its space.
3. **The nightly-bump chore grows a dimension.** §2.5 priced the bump
   chore per platform. Under composition it is priced per _catalogue_:
   caps are re-validated per nightly, profiles recompile, arbitrary
   compositions re-derive or expire. That is cheaper than per-platform
   maintenance _if_ the catalogue is the unit — but only if cap
   implementations are genuinely independent. A cap that reaches into
   another cap's host internals breaks re-derivation, and must be marked
   non-composable until fixed.

## 3.7 What this changes in the build order (§2.6)

Nothing moves earlier. The composer slots into §2.6's order after the shim
prototype proves one hand-built platform:

- **Composer v0 — the named next slice (recorded here, not enqueued):**
  manifest schema + canonical hashing; interface-package generation for
  the storage trio on the Cloudflare shape; profile-only composition
  (operator-submitted manifests; no arbitrary composition yet); registry
  record written beside the provisioner's grant records. Success test: two
  manifests differing by one cap produce two platforms whose exposed
  surfaces differ by exactly that cap's modules, and an app declaring the
  omitted cap fails at build against the second platform. The example apps
  of §3.8 are the composer's first consumers: each profile in v0's scope
  ships with at least one example app built against it, and "the example
  apps build and run against the composed platforms" is part of the
  slice's done test, not a follow-up.

## 3.8 Direction recorded (Drew, 2026-11-23, later): own the five platforms

A further direction from Drew, recorded the same evening, which **extends
the fork/consume analysis of §2.5 and supersedes its default where they
differ**:

1. **Own a variant of each of the five surveyed platforms** — RocRay,
   roc-gui, Joy, roc-native-android, and roc-cloudflare-workers. The plan
   is to use and own our own fork/variant of each, not merely to stay
   API-compatible with upstream. §2.5's recommended default — _re-host_
   (our host, upstream-compatible API) for the storage caps, _consume_ for
   RocRay-shaped and Cloudflare-shaped surfaces — is superseded by this
   direction: for these five, the answer is **own the variant**, with the
   catalogue contract (§2.2) still the family's portable artefact and the
   thing that must not fork. (roc-signals is an engine inside the GUI and
   Android variants, not a sixth platform in this list; it rides with
   them.)
2. **Build multiple example apps directly off our own forks/variants.**
   The example apps are part of the programme, built against our variants
   from the start — at least one per variant, plus the per-profile
   examples of §3.7. They are also the family's regression suite: a
   nightly-pin bump is rehearsed by rebuilding the example apps, and the
   breakage list they produce is the bump's true cost, known before any
   real app pays it.
3. **Pin a Roc nightly as the standard toolchain pin across the family.**
   One pin for all five variants, the catalogue, and every composed
   platform. Bumps are a single family-wide event (rehearsed per point 2),
   never five independent drifts. This interacts with §3.2 directly: the
   pinned nightly is a component of every capability-set hash, so a pin
   bump honestly produces new composition identities — a platform under
   nightly N+1 _is_ a different artefact from the same manifest under N,
   and the registry should say so rather than pretending continuity.

**What this means for the composer:** the catalogue's host implementations
_are_ our variants' capability implementations. A cap's
host-implementation version in the §3.2 hash is our variant's version, and
a composed platform is assembled from our variants' capabilities — the
composer is, in effect, the factory that turns the five owned platforms
into one composable family. Owning the variants is also what makes the
§3.1 union property achievable in practice: caps whose upstream
implementations carry escape hatches or ambient authority can be _fixed in
our variant_ until they are closed under the contract, instead of being
marked non-composable forever.

**The accepted cost, named honestly:** §2.5 flagged the price of forking —
the nightly-bump chore, the host bugs, the divergence, forever — and the
ecosystem's ~5-person bus factor. This direction takes that price on for
_five_ platforms at once, in an RC1-era ecosystem whose APIs are explicitly
unstable. The maintenance the earlier note hoped to rent from upstream is
now ours, concentrated. That is an accepted cost of the direction, not an
oversight in it: ownership is what buys the enforcement points (§§2.3–2.4)
and the composability fixes above, neither of which upstream is building.
The stabilising discipline is the single family pin (point 3) plus the
example apps as bump rehearsal (point 2): the chore becomes one rehearsed,
measured event per bump instead of five silent drifts — but it remains a
standing chore, and a quarter of neglect is as fatal here as §2.5 said it
would be.
