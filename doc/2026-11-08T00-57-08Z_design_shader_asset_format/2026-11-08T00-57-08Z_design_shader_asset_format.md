# ID shader asset format — v1 design

Design record — 2026-11-08. Deliverable for [#6013](https://github.com/developing-today/code/issues/6013), under the worlds presentation program [#5970](https://github.com/developing-today/code/issues/5970) and parent [#5955](https://github.com/developing-today/code/issues/5955). Design only: this record does not authorize shader compilation, rendering, upload, or runtime implementation.

State reviewed at `main` `8169bbd899` on 2026-11-08. The relevant issues #5955, #5970, and #6012 are open. Repository search found no existing shader asset format record or implementation. The current presentation model keeps guest-produced presentations as data and renders them through trusted adapters; shaders must preserve that boundary.

## 1. Request and problem

Worlds need named, reusable visual assets that can carry effects such as a sparkle filter without embedding a shader in each world's program. The open question in #5970 is whether user shaders join the user-function compile/trust path. This design answers no: a shader is a versioned presentation asset, interpreted only by a trusted renderer through a constrained format adapter. It does not become a world-language function or gain access to authoritative world state.

The format also needs to make compatibility and cost visible. GLSL fragment shaders and WGSL are not interchangeable source languages; a client that cannot run a specific format needs an authored fallback, not an implicit conversion or an improvised substitute.

## 2. Goals and non-goals

Goals:

- Give shader assets a stable name, immutable version, and explicit format identifier.
- Define one narrow v1 unit shape and the data it may read.
- Make licensing provenance and declared inputs part of the asset contract.
- Let #6012's cost ranking and #5970's world/viewer capability policy gate display.
- Preserve authored degradation paths for clients that cannot or may not run the shader.

Non-goals:

- Implement a shader compiler, renderer, upload pipeline, or runtime.
- Add arbitrary user functions, compute shaders, vertex shaders, multipass feedback, or a general plugin system.
- Promise GLSL-to-WGSL conversion or identical output across GPU vendors.
- Set the rank thresholds, publication license policy, or world permission UX owned by adjacent designs.

## 3. Decisions

### 3.1 A closed, code-owned format registry

Each asset names a registered format and format version. The v1 registry entry is `shadertoy.glsl.fragment` version `1`: a Shadertoy-compatible GLSL fragment unit. A registry entry is implemented by a trusted presentation adapter; uploaded metadata cannot register a new executable format, renderer, or extension.

The asset's own revision is separate from its format version. Replacing source creates a new immutable asset revision; it does not silently change what an existing world reference means. A record should carry an asset identifier, revision or content digest, format identifier/version, source payload, declared inputs, license/provenance metadata, and authored fallbacks. This is a logical envelope, not a commitment to a wire encoding or database schema.

Unknown format IDs or unsupported versions are not guessed. The renderer follows the asset's declared fallback policy; if none is usable, it leaves the base presentation unfiltered. It never compiles the payload as a different language.

### 3.2 One fragment unit, with a deliberately small input surface

V1 contains one fragment program using the Shadertoy entry shape:

```glsl
void mainImage(out vec4 fragColor, in vec2 fragCoord)
```

The trusted adapter supplies the wrapper and target. V1 excludes vertex and compute stages, multiple render passes, feedback buffers, audio analysis, and format-specific hidden inputs. The exact GLSL ES profile and compiler target must be frozen by the first renderer implementation and included in the registered format conformance tests; the format name must not claim portability beyond that tested profile.

The v1 declared inputs are:

| Input                      | Meaning                                                  | Boundary                                                                                                                         |
| -------------------------- | -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `iResolution`              | Width and height of the effect's bounded render target   | Supplied by the adapter; not a source for arbitrary screen or device inspection                                                  |
| `iTime`                    | Elapsed time for this effect instance                    | Relative and adapter-controlled; no wall-clock/date access                                                                       |
| `iChannel0`–`iChannel3`    | Optional textures named by the asset's input declaration | Each reference resolves only to an asset the world and viewer may already see; no URL, filesystem, or implicit cross-world fetch |
| `iChannelResolution[0..3]` | Dimensions of the corresponding declared texture         | Supplied only for bound channels                                                                                                 |

The declaration names each channel's semantic purpose and whether it is required. An absent optional channel is bound to transparent black. A missing required or unauthorized reference makes the effect unavailable and selects its authored fallback; it does not widen access. V1 does not expose pointer coordinates, keyboard state, camera, microphone/audio samples, absolute time/date, or ambient device data. A later input needs an explicit capability and consent decision, not merely a new uniform.

### 3.3 A shader is presentation data, not a world function

The source is inert asset content until a registered renderer handles it. Execution belongs to the trusted presentation adapter, never the authoritative world program or server-side user-function runtime. The adapter exposes only the declared target and channels. Shader output may alter pixels in that target; it cannot mutate world data, identity, hitboxes, permissions, chat, or game outcomes. A world may disable an effect even when a viewer's client supports it.

This is a boundary, not a claim that arbitrary shader text is harmless. GPU drivers and compilation remain an attack surface. Any renderer implementation must isolate compilation, bound the target and work, handle compile/runtime failure, and preserve the base presentation. Those mechanisms and numeric limits are implementation gates, not supplied by this design record.

### 3.4 Licensing is explicit metadata; there is no inherited site default

An asset carries provenance and a license assertion/reference, including the source and attribution needed by the licensing policy. The format has no implicit license. In particular, importing from a site must not silently adopt that site's default license (including Shadertoy's user-content default); a missing or unresolved assertion is not treated as public-domain or freely redistributable.

The format provides the metadata seam; #5955 and its licensing work define classification, verification, and publication policy. Until that policy accepts an asset's assertion, the format must not imply that public sharing is allowed. License metadata is not executable input and cannot grant renderer capabilities.

### 3.5 Ranking and degradation are contracts, not shader claims

The format declares what the ranker needs to measure, but the asset author cannot self-assign a trusted cost rank. Candidate measurements include target pixel area, declared channel count/texture dimensions, pass count (fixed at one in v1), and adapter/compiler work estimates. #6012 owns which measurements count, the rank table, per-world enablement, per-viewer minimum, and any per-person override. This design sets no thresholds. Unknown, unsupported, or over-budget content follows #6012's policy and must not bypass it by changing the declared format.

An effect reference uses #5970's authored fallback chain. The author may supply a static filtered substitute or an unfiltered/base asset; lower presentation tiers may continue through the already-authored sprite/glyph fallback. A missing or unusable fallback is handled as #5970 specifies; the renderer does not invent a converted shader, and it does not make an effect mandatory for worlds or clients.

## 4. Coordination and dependency edges

| Work                                                                                        | Relationship to this record                                                                                                                                                                     |
| ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [#5955](https://github.com/developing-today/code/issues/5955) — worlds design program       | Parent for asset/program semantics and licensing policy. This record provides the shader-format seam; licensing classification and publish eligibility remain there and its licensing children. |
| [#5970](https://github.com/developing-today/code/issues/5970) — presentation assets         | Owns per-world capability enablement, authored fallback chain, presentation tiers, and the boundary between asset effects and world behavior. Shader use must compose with its policy.          |
| [#6012](https://github.com/developing-today/code/issues/6012) — visual-feature cost ranking | Owns measurements-to-rank mapping, gates, viewer minimums, and substitution rules. V1 exposes measurable dimensions but defines no rank thresholds.                                             |
| [#6013](https://github.com/developing-today/code/issues/6013) — this format                 | Owns the named v1 GLSL fragment unit, declared input vocabulary, format-version rule, and the asset metadata seam.                                                                              |

The design record can land independently. Runtime implementation must wait until #5970's policy and #6012's ranking interface are stable enough to enforce, and must use #5955's accepted licensing policy before enabling public sharing. These are implementation dependencies, not reasons to block this format decision.

## 5. Suggested implementation slices (not authorized here)

1. Define the versioned asset envelope and code-owned format registry; add conformance fixtures for the v1 entry shape and input declarations.
2. Add metadata validation and provenance/license hooks, integrating with #5955's licensing policy before public publication.
3. Implement the trusted presentation adapter for the registered fragment profile, with bounded target, channel authorization, failure handling, and the #5970 fallback path.
4. Connect adapter measurements and world/viewer gates to #6012; unknown or unsupported rank must follow its conservative policy.
5. Add cross-client/fallback tests proving that unsupported format, denied capability, missing channel, and renderer failure preserve the authored non-shader presentation.

These slices are sequencing suggestions. The implementation plan should refine ownership and tests after #5970 and #6012 settle their interfaces; no shader runtime work should be inferred from this record alone.

## 6. Open questions for implementation planning

- Which exact GLSL ES profile is supported by the first trusted renderer, and what conformance corpus proves the `shadertoy.glsl.fragment/v1` claim?
- Are four channel slots the right v1 ceiling? This design caps them at four for a small declared surface; measurements and devices may justify less, but not an undeclared expansion.
- What concrete source/license evidence is sufficient for a shader to become publicly shareable? Resolve in #5955's licensing policy, not by defaulting here.
- Which GPU workload metrics can be measured consistently across clients, and how do they feed a common rank? Resolve in #6012; client-specific measurements must not make a high-cost effect appear cheap.
- Which static or unfiltered fallback is required for a shader-backed asset at each #5970 presentation tier?
- What per-effect clock behavior keeps animation bounded and understandable across suspend/resume and replay? V1 exposes only adapter-controlled relative time; details belong in the renderer contract.

No Drew-specific ruling is required to establish the format boundary. The open questions above are implementation/interface questions for the named sibling issues.

## 7. References

- #6013 — [shader asset format](https://github.com/developing-today/code/issues/6013)
- #5955 — [worlds design program](https://github.com/developing-today/code/issues/5955)
- #5970 — [presentation assets and fallback](https://github.com/developing-today/code/issues/5970)
- #6012 — [visual-feature cost ranking](https://github.com/developing-today/code/issues/6012)
- `doc/DOCUMENTATION_PROTOCOL.md` — dated-record placement and immutability
- `doc/2026-10-31T17-47-20Z_design_world_presentation_and_fuel/2026-10-31T17-47-20Z_design_world_presentation_and_fuel.md` — current presentation/data and trusted-adapter boundary
