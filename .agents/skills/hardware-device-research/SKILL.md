---
name: hardware-device-research
description: Build or audit a source-traceable hardware knowledge base. Use when asked to research or identify a board, module, appliance, component, teardown, firmware platform, datasheet, pinout, internal hardware, compatibility, source code, firmware, or existing device documentation.
---

# Hardware Device Research

Research a hardware device recursively: document the complete product, identify its functional components, create a reusable record for each component or interface, preserve relevant artifacts, and verify the resulting knowledge base.

## What makes this different from summarising a datasheet

Five habits do most of the work. If you internalise nothing else, internalise these.

**Prefer primary evidence, and say which kind you used.** Vendor prose is the weakest source in the stack and is frequently wrong. Published EDA files, parsed firmware images and vendor board-support headers are authoritative and usually available. Label every consequential claim: `executed-success` · `reported-working` · `inferred` · `not-tested`, and never present an untested command as though it were verified.

**File by what a document *describes*, not by what you were researching.** A chip datasheet fetched while studying a board is a *component* artifact. See [Where a file belongs](#where-a-file-belongs--decide-by-what-it-describes-not-by-what-you-were-researching).

**Record conflicts; do not resolve them by preference.** When sources disagree, state the disagreement, cite both, and say what evidence would settle it. Vendors contradict themselves often, and the contradiction is usually the finding.

**Preserve failures and negative results.** A URL that 404s, an HTML error page saved with a `.pdf` extension, a search that returned nothing, a translation that added nothing — each saves the next agent from repeating the work. They belong in the record.

**Nothing useful may exist in only one place.** Every artifact is either in the repository, or archived with a placeholder carrying its hash and reacquisition instructions. Scratch is not storage.

## Where research is written

⚠ **Hardware research does not live in this repository.** It lives in a **separate repo**,
**[`developing-today/hardware-doc`](https://github.com/developing-today/hardware-doc)**, checked
out beside this one and symlinked in at `doc/hardware`:

```
<repo-parent>/
├── code/                       ← this repo
│   └── doc/hardware  ────────┐   symlink (gitignored)
├── hardware-doc/        ←────┘   the knowledge base — WRITE HERE
└── hardware-doc-archive/         bulk artifacts (separate repo, usually unpublished)
```

Set it up — clones if missing, fast-forwards if clean, never clobbers local work:

```bash
./scripts/hardware-doc-init.sh      # also runs automatically in the devshell
```

**Write records in `hardware-doc`, and commit them there.** It is deliberately not a submodule
and not vendored here: at ~440 MB it would make every clone of this repo roughly 6.5× larger.

Its layout, conventions and the full method are documented in that repo:

- `../hardware-doc/README.md` — entry points and research passes
- `../hardware-doc/AGENTS.md` — working conventions
- `../hardware-doc/.agents/skills/hardware-device-research/SKILL.md` — **the authoritative copy of this skill**
- `../hardware-doc/ai-crawler-site-access-table.md` — per-site retrieval findings

```text
hardware-doc/
├── devices/<manufacturer>/<normalized-product-name>/
├── components/<manufacturer-or-generic>/<normalized-part-or-interface>/
├── vendors/<manufacturer>/          # documentation-sourcing knowledge
├── guides/<domain>/                 # device-independent, cross-cutting
└── tools/                           # netlist parsers, archiver, image tools
```

Bulky derived artifacts move to **`../hardware-doc-archive`** — a sibling of the *real*
repository root, resolved worktree-safely:

```bash
ARCHIVE="$(dirname "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")")/hardware-doc-archive"
```

Use `--git-common-dir`, not `--show-toplevel`: in a linked worktree the toplevel is the
worktree, whose parent is the wrong directory.

## Required Inputs

Infer these when reliable; ask only when ambiguity would materially change the target:

- Manufacturer and exact product name
- Product/SKU/catalog identifier
- Region or sales variant
- Known PCB/hardware revision
- User's intended firmware framework, if relevant
- Repository artifact-size policy and whether omitted artifacts should be locally cached

Do not confuse enclosure colors, battery bundles, language suffixes, or sales bundles with PCB revisions.

## Workflow

### 1. Inspect the repository

- Check documentation conventions, symlinks, ignore rules, existing device/component records, and unrelated working-tree changes.
- Preserve unrelated changes.
- Do not commit unless explicitly requested.
- Staging is allowed as part of organizing the research, but keep large or licensing-uncertain artifacts unstaged until the user reviews the categorized artifact report. Prefer staging authored research, manifests, acquisition tooling, and artifacts with clear terms.
- Add or update umbrella, device, and component indexes.

### 2. Identify the exact device

Establish the product identity from primary evidence:

- Official product page and product identifier
- Official wiki/manual/support page
- Hardware or PCB revision, if published
- Sales variants and regional differences
- Lifecycle status and publication/update dates

If identity remains ambiguous, record the alternatives and evidence rather than guessing.

### 3. Research in source-priority order

Prefer sources in this order:

1. Manufacturer product pages, support pages, manuals, schematics, repositories, downloads, and APIs
2. Component-manufacturer product pages, datasheets, technical reference manuals, errata, and application notes
3. Standards bodies and certification/regulatory databases
4. Official framework, SDK, package registry, and library documentation
5. Authorized distributors and credible mirrors
6. Community repositories, teardowns, and forum reports, clearly labeled non-authoritative

For every relevant source capture:

- Title
- Publisher
- Canonical URL
- Retrieval date in `YYYY-MM-DD`
- Document/version/revision/publication date, if available
- What the source establishes
- Whether it is primary, mirror, or community evidence
- Local artifact path, if retained

Use an immutable page revision or repository commit URL where possible.

Use this minimum source-table schema:

```markdown
| ID | Title | Publisher/author | Class | Medium | URL | Retrieved | Published/updated | Establishes | Scope/limitations | Local path |
|---|---|---|---|---|---|---|---|---|---|---|
```

`Class` describes authority and must be one of `primary`, `authorized mirror`, `credible mirror`, `standard`, or `community`. `Medium` describes format, such as `official page`, `manual`, `datasheet`, `store listing`, `forum`, `blog`, `review`, `social post`, `video`, `repository`, `benchmark`, `teardown`, `regulatory record`, or `archive`. Give important factual claims a source ID in the same paragraph/table row or in an explicit evidence column.

Forums, blogs, reviews, tweets and other social posts, videos, issue discussions, project logs, software projects using the device, owner reports and cultural/community material all count as useful evidence. Record whether each contribution is a firsthand measurement, firsthand ownership/use report, reproduced documentation, demonstrable project, informed interpretation, opinion, hearsay, or unsupported assertion. Popularity or production quality does not increase authority by itself.

#### Breadth and search ledger

Search every applicable source class rather than stopping after finding one adequate official page. Use the exact product name, SKU, aliases, PCB markings, chip IDs, artifact filenames, firmware identifiers, major features, framework names, and common misspellings. When useful, search alternate languages, archived pages, repository code/history/releases/issues/discussions/forks, package registries, distributor mirrors, regulatory records, forums, blogs, videos with technical material, and dead-link archives.

Maintain `research-log.md` containing:

- Service/site searched, exact query, filters, result depth/pages, and date
- Useful, duplicate, rejected, inaccessible, dead, or archived result status
- Original, canonical, redirected, mirror, archive, release, issue, discussion, video, and direct-download URLs
- Attempts to recover dead or moved sources
- Search cutoff and known exclusions

Maintain a raw link ledger even when `sources.md` is curated. Do not discard a potentially useful link merely because a better source exists. Prefer two independent sources for consequential claims when available while giving primary evidence precedence.

Maintain `commands.md` as a command ledger for every useful acquisition, extraction, build, dependency-installation, configuration, flash, recovery, debug, conversion, inspection and verification command discovered or executed. Preserve:

- Command verbatim, source and purpose
- Working directory, shell/OS, tool and exact version
- Prerequisites, environment variables and required hardware state
- Expected output/result and generated files
- Status: `executed-success`, `executed-failed`, `reported-working`, `inferred`, or `not-tested`
- Execution/retrieval date, applicable device revision and framework version
- Failure output, diagnosis and corrected command when applicable

Redact secret values but retain variable names and setup instructions. Prefer checked-in reusable scripts for reliable multi-step procedures, with `commands.md` linking to them. Do not silently discard failed commands because they often document important pitfalls.

### 4. Gather device artifacts

Use an acquisition-first, repository-first approach. The agent can and generally should download every plausibly useful public file regardless of size or initial license clarity so its contents, metadata, embedded notices, versions, hashes, and development value can be inspected. Place useful artifacts in the device/component artifact tree by default. A download location does not itself decide whether a file will be staged or committed.

#### Where a file belongs — decide by what it *describes*, not by what you were researching

This is the single most common filing error, and it is expensive to correct later. A file fetched while researching a board is not necessarily a *board* file.

| The file describes… | It belongs under | Example |
|---|---|---|
| A specific board/product | `devices/<manufacturer>/<product>/artifacts/` | Board schematic, factory firmware, enclosure CAD |
| A chip, module or part | `components/<manufacturer>/<part>/artifacts/` | SoC datasheet, sensor datasheet, connector spec |
| A vendor's portal, process or tooling | `vendors/<manufacturer>/artifacts/` | Driver-porting guide, flashing utility, vendor-wide app note |
| Something cross-cutting | `guides/<domain>/` | Framework programming guides, protocol references |

**A chip datasheet obtained from a board vendor's download page is still a component artifact.** File it under the component and note the mirror in the source table.

> **Observed failure.** Three agents each fetched an ESP32-S3 datasheet while researching three different boards — from Espressif, from Waveshare's mirror, and from Seeed's copy. The result was three files with **three different SHA-256 values and three different sizes** (1,098,115 / 1,186,331 / 1,186,462 bytes). They were not redundant copies; they were different revisions from different portals, and nobody could tell without hashing them.

**Vendor mirrors are worth keeping — but only when labelled.** If a vendor ships a different revision than the chip maker, that is a finding (vendors routinely serve years-old revisions). Retain the mirror beside the original with a name that states the source (`esp32-s3-datasheet-v1.6-waveshare-mirror.pdf`), record both in the source table, and say plainly which one the board was designed against. If the mirror is byte-identical to the original, keep one and record the duplicate URL rather than storing it twice.

#### Size decides *where*; classification decides *whose*

When a datasheet is pulled while researching a device, two independent questions apply. Answer them in order:

1. **Whose is it?** → the table above. A chip datasheet is a **component** artifact even though a device task fetched it.
2. **Small or large?** → small stays in the repository; large moves to the archive with a placeholder left in its place.

"Large" is a judgement against the repository's tolerance, not a fixed number — but a multi-megabyte framework guide or offline documentation bundle is large, and a few-hundred-kilobyte part datasheet is not.

**When a component artifact is archived, three links must exist.** Any one of them missing breaks the trail for the next reader:

| # | Link | Lives in |
|---|---|---|
| 1 | **Placeholder** — hash, byte size, retrieval date, archive path, reacquisition URLs | The component's own `artifacts/` directory, at the file's former path |
| 2 | **Component → placeholder** | The component `README.md` and its source table, marking the row `archived <date>` |
| 3 | **Device → component, and device → placeholder** | The device record that caused the fetch |

Link 3 is the one that gets forgotten. A reader working from a board record must be able to reach an archived chip document without already knowing which component owns it. Cite it where the artifact is listed, not only in a components index.

A worked example of the full chain, using strikethrough to show the file is no longer in place:

```markdown
| ~~esp-dev-kits-en-master-esp32p4.pdf~~ **archived 2026-08-24** | 30 482 003 |
  `04d75d2a…` | Offline user guides — archive record (`devices/espressif/shared-artifacts/ARCHIVED-FRAMEWORK-GUIDES.md` in `hardware-doc`) |
```

Where several devices share one component, the placeholder is written **once** in the component directory and cited by **every** device that uses it. Do not copy the artifact per device.

#### The same rule applies to everything that crosses folder boundaries

Components are the most common case, not a special one. The identical pattern — **classify by what it describes, place by size, leave a placeholder, cite it from every consumer** — applies to:

- **General and cross-cutting information** → `guides/<domain>/`
- **Examples and demo code used by more than one device** → the component or guide that owns it, cited by each device
- **Vendor-wide material** (portal guides, flashing tools, driver bundles) → `vendors/<manufacturer>/`
- **Anything else consumed from two or more places**

`doc/hardware/` is not a closed taxonomy. If material genuinely fits none of `devices/`, `components/`, `vendors/` or `guides/`, add a category rather than forcing a bad fit — but only when the misfit is real, and say in the index why the category exists.

#### Record identity, not just location

Every retained or archived artifact records whatever identity information exists. Missing fields are written as unknown rather than omitted silently.

| Artifact kind | Record |
|---|---|
| Document | **Version/revision**, publication or revision date, retrieval date |
| Repository or file from one | **Commit hash** (full), **tag or release name**, release URL, and the permalink at that commit |
| Release asset | Release tag, asset name, upload date |
| Archive/binary | SHA-256, byte size, and any embedded build identifier |

For repository content, a branch name is not identity — branches move. Cite the **commit hash**, and prefer a permalink (`…/blob/<sha>/…`, `…/tree/<sha>/…`) or a release-pinned URL over a `HEAD`/`main` link. Where a release exists, record both the tag and the commit it points at.

#### Transient workspace

The archive has exactly **two** states. Keep it that way — a third invites stranding.

```
<archive-root>/
  doc/<repo-relative-path>     # DECIDED: mirrors the repository, placeholder in repo
  scratch/<subject-slug>/      # UNDECIDED: raw fetches, intermediates, working files
```

**The moment a file's home is known, move it to `doc/` at its repo-relative path** and write the placeholder. Mirroring the repository's own path inside the archive means the mapping needs no lookup table and survives reorganisation. Do not add a staging tier inside `scratch/` that mirrors repo paths — it would only give work somewhere to stall. *(Measured on an ~11,000-file archive: every file with a decided path was already in `doc/`; none of the ~5,800 scratch files had one.)*

**Download into `scratch/` directly; do not stage in `/tmp` and copy later.** The files must reach the archive regardless, so `/tmp` adds a mandatory step that can fail. `/tmp` is also not dependable — it is cleared between sessions on many hosts, and has been observed losing an agent's files mid-task. Use it only for work you will discard: peeking inside an archive, probing a URL, checking magic bytes. If you do put something there you may want, `mv` it immediately — on a shared filesystem that is a rename, not a copy. *(In one session a 230-file wiki corpus, FCC exhibits and vendor headers vanished from `/tmp` mid-task. On that host `/tmp` was a bind mount of the same filesystem as `$HOME`, so it was neither faster nor cheaper than writing straight to the archive.)*

**Inside `scratch/<subject-slug>/`, organise however the subject needs.** No layout is prescribed. Reusable material needs no special place: a chip datasheet is a *component* artifact, so once recognised it belongs in `doc/hardware/components/…`, which is where a parallel agent will look for it.

**Namespace by subject, never by agent** — agents are concurrent and ephemeral; the subject outlives them. **List the parent before creating any directory**; a convention probably exists. *(Three agents in one session independently created `workspace/`, `tmp-workspace/` and `_tmp-sweep-*/` for the same job — 363 MB of duplication, reconciled by hand.)*

**Give each subject a `README.md`** recording what it holds, when it was fetched, whether a canonical copy now exists in the repository, and any failed or misleading fetches — an HTML error page saved as `.pdf`, a URL that now 404s.

**Do not tidy other agents' scratch.** It is working space; its mess is not a defect.

When a fetched file resembles one you already hold — same name, same version, a vendored copy of a known library, or the same document in another language — see [step 17](#17-analyze-vendored-dependencies-near-duplicates-and-large-artifacts) before discarding either.

#### What to collect, and where it lands

Search for and preserve, when available:

- Datasheets and manuals
- Schematics, BOMs, Gerbers, PCB/EDA files, CAD/STEP/DXF and dimension drawings
- Pinout diagrams and connector drawings
- Official SDKs, examples, libraries, source archives, repositories and release snapshots
- Factory firmware, bootloaders, partition tables and recovery tools
- Release notes, errata, FAQs and revision histories
- Product images useful for identification or mechanical work

Suggested device layout:

```text
devices/<manufacturer>/<product>/
├── README.md
├── pinouts-and-buses.md
├── research-log.md
├── commands.md
├── coverage.md
├── resources-and-conflicts.md
├── compatibility-and-status.md
├── development.md
├── factory-firmware.md
├── gaps-and-conflicts.md
├── sources.md
├── community.md
├── product-history-and-family.md
├── market-and-pricing.md
├── comparisons-and-recommendations.md
├── performance.md
├── projects-and-community.md
├── media.md
├── media/
│   └── manifest.json
├── features/
│   ├── README.md
│   └── <feature>.md
├── examples/
│   ├── catalog.json
│   ├── search-log.md
│   ├── best.md
│   └── selected/
├── acquisition/
│   ├── README.md
│   └── manifest.json
└── artifacts/
    ├── originals/
    ├── schematic/
    ├── demo/
    ├── firmware/
    ├── datasheets/
    └── source-snapshots/
```

Err toward downloading, validating, inspecting, and preserving potentially useful artifacts rather than omitting them because their size or license is initially unclear. Vendor useful artifacts by default unless they are excessively large for the repository after measured analysis or explicit terms very clearly forbid repository redistribution. Retain original archives and extract useful contents for direct access. Preserve upstream directory structure, licenses, copyright notices, and filenames unless a filename is unsafe or nonportable. Record any rename.

Extract useful facts from downloaded material into authored research documents so future development does not depend on reopening the artifact. This includes pin mappings, register details, build settings, toolchain versions, firmware metadata, example structure, dependency versions, revision identifiers, and recovery procedures.

Record the retrieval date and all available immutable identity information: document version/revision, release/tag, repository commit hash, archive checksum, file build identifier, and upstream publication/update date.

At completion, no useful or uniquely acquired information may exist only in `/tmp`. Promote the artifact into the repository tree, move it to an ignored persistent cache under `doc/hardware/`, or create complete reacquisition metadata and derived documentation before removing temporary files.

#### Placeholders must stand alone

When an artifact is moved out of the repository, the placeholder left in its place is the **only** thing most readers will ever see. The archive is machine-local: someone cloning the repository has no `~/…-archive/` and never will. A placeholder that merely says "moved to the archive" is useless to them.

**A placeholder must be sufficient to reacquire the file without the archive.** It is the same provenance the [acquisition manifest](#artifact-and-acquisition-manifests) already records for every artifact — SHA-256, byte size, canonical URL, retrieval date, upstream version/date, and commit hash or release tag where applicable — written for a human instead of a parser. Keep the two consistent; if they disagree, the manifest is authoritative and the placeholder is stale.

Beyond those shared fields, a placeholder also states:

- **What it was** — original filename and repository path
- **Why it was moved** — size, licence, redundancy
- **Reacquisition URLs**, ideally more than one, ordered by reliability, preferring a permalink or release-pinned URL over a `HEAD`/branch link
- **The archive path**, as a convenience for whoever *does* hold the archive

Record failed or blocked reacquisition honestly (`automatic`, `manual`, `blocked`, `lost`) rather than omitting the instructions.

**The archive path is a convenience, not the contract.** It should be kept accurate — if you reorganise the archive, grep the repository and repair the references in the same change, exactly as you would a breaking rename — but a reader without the archive must still be able to recover the file from the placeholder alone. Verification therefore tests the placeholder's completeness, not whether the path resolves on this machine.

**Placeholder conventions vary** (`*.ARCHIVED.md`, `*.REMOVED.md`, `*.DUPLICATE.md`, `ARCHIVED-*.md`, directory `README.md`). When auditing coverage, walk **all ancestor directories** — a directory-level placeholder shadows everything beneath it, and checking only a file's immediate parent reports large numbers of false gaps.

**Use one archive root.** Do not create a near-identical sibling (`hardware-doc-archive` vs `hardware-docs-archive`); check for existing roots before creating one, and prefer a symlink over a second directory.

Never save an error page or HTML response with a `.pdf`, `.zip`, or firmware extension. Validate file type from content, not just URL or suffix.

### 5. Build the visual, product, community and market dossier

Create custom prose and structured evidence describing the device as a physical product, technical platform and market option—not only as a collection of specifications.

#### Visual identification

Find useful front, back, side, port/label, enclosure-open, teardown-stage, PCB front/back, revision-marking, component close-up and representative-use images. Retain and embed useful images when practical; if rights are unclear, they may remain unstaged pending user review. Every image must record creator/rightsholder when known, source page and direct URL, publication/retrieval dates, applicable device/revision, license/permission evidence, redistribution status, modifications, local path, caption and alt text. Captions explain what the image demonstrates and any crop, annotation, rotation or processing. Do not remove watermarks or present an image-search result as provenance.

#### Identity, history, family and culture

Document exact marketed identity, manufacturer, aliases, SKUs, variants, revisions, regions and bundles; announcement, preorder, launch, revision, discontinuation and support milestones; intended audience and original positioning; manufacturer/project background; community and product culture; predecessors, successors, siblings and related platforms; shared SoCs/modules/enclosures/PCBs/firmware/ecosystems; and material family differences. Use a sourced dated timeline. Do not infer equivalence from similar names or appearance.

#### Community and projects

Catalog applicable manufacturer/specialist forums, independent blogs, editorial and owner reviews, tweets/social posts, discussion threads, videos/streams/talks, teardowns, repositories, integrations, deployments, project logs and common or unusual use cases. For videos record relevant timestamps; for threads link specific posts/comments. Separate demonstrated projects, proposals, abandoned attempts, copied showcases and marketing claims.

Anecdotes establish that an experience occurred, not how common it is. Use prevalence wording only with defensible evidence; where possible report the inspected sample, for example “three reports among 42 reviewed threads.”

#### Dated pricing and availability

Record launch pricing and current pricing separately across official stores, authorized/mainstream retailers, specialist sellers, marketplaces such as AliExpress, clone/compatible listings, and used/refurbished/liquidation markets. Each observation includes date, region, seller, condition, exact configuration/bundle, genuine/clone/uncertain status, item price, shipping, known tax/duty treatment, currency and conversion date, stock evidence and source.

Distinguish MSRP, crowdfunding, preorder, introductory and observed street prices. Validate marketplace “from” prices, coupons, minimum quantities and selected configurations. Do not merge bare boards, kits, memory/storage variants, used units or clones into one range without explicit normalization. Report ranges with sample count and date.

#### Competitors, equivalents and clones

Compare direct competitors, practical substitutes, predecessors/successors, clones and related components by explicit use case and price/performance tier. Cover processor, memory/storage, radios, I/O, power, thermal design, enclosure, certifications, firmware, drivers, documentation, warranty, availability, community support and lifecycle. Include mainstream, specialist, marketplace and used-market alternatives where appropriate.

Call something “equivalent” only for the stated workload when interfaces, electrical behavior, performance and software requirements actually align; otherwise call it a partial substitute. Break down meaningful differences between the main device and common clones rather than assuming none.

#### Uses, uniqueness, strengths and shortcomings

Explain common evidenced projects/use cases, unusual possibilities, distinguishing or unique capabilities, concrete reasons to choose the device, whole-device hardware/software/support/lifecycle shortcomings, and workarounds—including whether workarounds erase its price, complexity, performance or reliability advantage. Separate manufacturer-intended uses, demonstrated projects, repeated owner practices and speculation.

#### Market fit then and now

Evaluate launch-era fit using contemporaneous prices and alternatives, then current fit using evidence dated to the research snapshot. Provide scenario-specific “use this when,” “do not use this when,” and alternatives by budget, mainstream, premium, low-power, compact, supported, experimentation, performance and professional/enterprise tiers as applicable. State workload, constraints, date and region for each recommendation.

Technical tables must be followed by prose explaining practical implications, context, uncertainty and exceptions. Prose must connect specifications and measurements to real decisions without generalizing beyond the sourced configurations. Create charts/plots when pricing, benchmark, latency, power, thermal, feature or competitor data becomes materially easier to understand visually. Every chart must identify data sources, retrieval/test dates, axes, units, configurations, sample sizes, transformations, uncertainty/error representation and generated source data/script; never use decorative or incomparable charts as evidence.

### 6. Inventory the complete hardware

Derive the component list from schematics, BOMs, chip markings, official descriptions, and source code. Include all firmware-relevant or externally accessible functions:

- MCUs, SoCs, coprocessors and programmable logic
- Flash, RAM and removable storage
- Displays and display controllers
- Touch, buttons, encoders, switches and sensors
- Audio input/output, codecs, DACs, amplifiers and microphones
- Wireless radios, antennas and RF switches
- Motor, haptic and LED drivers and actuators
- Power conversion, charging, protection and battery interfaces
- USB bridges, muxes, level shifters and bus switches
- Expansion headers, debug ports and significant external connectors

Ordinary passives do not need individual records unless they affect firmware, signal integrity, calibration, safety, or a published interface.

Do not assign a guessed part number. Create an unidentified/generic record and explain what evidence would resolve it.

### 7. Research every component recursively

Give every listed functional component or interface its own folder and `README.md`. For fitted modules or subassemblies, repeat the inventory process for their internal firmware-relevant parts when documentation or teardown evidence makes that useful. Continue until reaching commodity passives, inseparable/unpublished internals, or parts whose deeper decomposition does not help development; record the stopping reason.

Each record must include:

- Manufacturer and exact part/family, or unidentified status
- Function and important capabilities
- Supply, limits, protocols, addresses, register/API references and relevant package/revision
- Device-independent interfaces, pins, buses, addresses and limits
- Recommended official SDK APIs, Arduino libraries, ESP-IDF components, Linux drivers, or other applicable software
- Datasheet/manual/product/library URLs with retrieval dates and versions
- Local artifacts and their provenance
- Caveats, errata, variant conflicts and unsafe assumptions
- A **Used By** section with one subsection/row per device, linking back and describing only that device's role and integration

Suggested component layout:

```text
components/<manufacturer>/<part>/
├── README.md
└── artifacts/
```

Keep the complete wiring table in the device record. Component records may summarize integration under the correctly keyed **Used By** entry, but must not present one device's wiring as universal. The device README must crosslink every component it mentions. The component index must expose every record.

When evidence exists, component records should also provide a smaller equivalent dossier: release/lifecycle, package/grade variants, visual/package markings, approximate pricing and availability, common applications, notable projects, competing/substitute parts, drop-in versus partial compatibility, reasons to choose, shortcomings, ecosystem/support and migration caveats. Keep board-level behavior separate from component-level capability.

### 8. Document pinouts and architecture

Create tables for each processor and bus showing:

- GPIO or connector pin
- Signal/function
- Direction
- Bus and address/chip-select
- Shared or strap-sensitive behavior
- Source of the mapping

Explain ownership and communication between processors, shared buses, mux selection, boot/reset behavior, USB routing, power domains, and peripheral arbitration.

Prefer schematic and source-code evidence over marketing prose. Record conflicts instead of silently choosing one.

### 9. Document development and recovery

When the target is programmable or firmware-bearing, record:

- Supported frameworks and exact minimum/recommended versions
- Board target/profile and relevant build configuration
- Required libraries and versions
- Build, flash and monitor procedure
- USB/UART/debug-port selection
- Factory restore procedure and flash offsets
- Firmware target MCU, version, build date, SDK version and partition details
- Known bootstrapping, power, connector-polarity and bricking risks

For non-programmable targets, replace this section with applicable setup, calibration, driver, host-software, maintenance, or recovery information and mark firmware fields `Not applicable`. Never present an untested inferred flash command as authoritative. Label it inferred and cite the evidence.

### 10. Audit vendor firmware and driver source against the primary documents

Vendor libraries are evidence, not authority. Reading a vendor's driver **side by side with the component's datasheet** is one of the highest-yield activities in this workflow: it reveals undocumented board behaviour, and it routinely exposes real defects that every downstream user inherits. Do this for each programmable component whose vendor code is available.

For each driver or firmware component:

- Decode every constant, register write and initialisation sequence against the datasheet's register map, and state what each one actually does.
- Compare the vendor's power-up/power-down ordering, delays and sequencing against the datasheet's required ordering.
- Identify registers and features the driver never touches, especially fault, status and interrupt reporting.
- Check that burst/sequential register access matches the device's actual auto-increment behaviour.
- Verify that status flags the datasheet marks as critical are surfaced rather than masked away.
- Confirm the driver reads values from the device rather than from host storage.
- Check whether the configured part matches the fitted part, including actuator type, waveform library, voltage range and package variant.
- Trace suspicious code to its origin; ported drivers frequently carry assumptions from the original chip.

Record each finding with the source file and line, the datasheet section that contradicts it, the practical consequence, and whether the defect is active or inert. Distinguish a genuine defect from a deliberate vendor adaptation, and preserve adaptations as documented patches. Report severity honestly: an incorrect write that is later overwritten is a latent trap, not a live bug.

When the vendor ships only a binary, say so plainly, and extract what firmware strings, partition tables and image headers legitimately establish.

### 11. Survey and select examples

Search broadly for official and community examples in repositories, forks, issues, package examples, forums, blogs, videos with code or technical detail, archived projects, and vendor/community integrations. Download promising candidates for inspection, including candidates that may ultimately be rejected. Deduplicate forks and copied projects while preserving lineage.

Create `examples/catalog.json`, `examples/search-log.md`, and `examples/best.md`. Catalog every meaningful discovered example, including broken, stale, duplicate, unlicensed, rejected, and inaccessible candidates, with:

- Stable ID, title, author and source class
- Canonical URL and immutable commit/release/archive URL
- Retrieval date, last activity date, release/tag and Git commit hash
- Device/SKU/hardware revision and framework/toolchain versions
- Features, components and hardware resources exercised
- Dependencies and exact versions
- License and evidence
- Build/run/report status and test environment
- Strengths, pitfalls, limitations, lineage and selection decision

When examples are numerous, inspect a broad candidate set and select a compact, high-value portfolio that collectively covers every major device feature. Include, where available:

- Best minimal/diagnostic example per feature
- Best integrated application covering many features
- Best maintained and reproducible starting point
- Examples testing performance, concurrency, resource limits or unusual capabilities
- Examples demonstrating distinct credible implementation approaches
- Useful negative examples exposing failures, incompatibilities or pitfalls

Vendor selected examples when reasonably sized and not subject to explicit no-redistribution terms. For every example not vendored, preserve its links, immutable revision, retrieval commands, dependency/setup commands, findings, and reason for omission. Explain why selected examples are preferable and why notable alternatives were not selected. Build or statically validate selected examples when feasible and retain exact failures rather than hiding them.

### 12. Write feature-oriented development guides

Inventory every meaningful user-facing and developer-facing capability, then create a directly discoverable guide at `features/<feature>.md`. Examples include Bluetooth, Wi-Fi, display, touch, buttons/encoders, audio input/output, cameras, infrared, accelerometers, GPS, LoRa, storage, USB, battery/power, haptics, LEDs, GPIO, ADC, PWM, pulse counting, timers and debugging. Only create applicable guides, but do not omit an advertised or fitted capability.

Each feature guide must answer a natural question such as “How do I set up Bluetooth on this device?” and contain:

- Capability summary and exact hardware path through linked components
- Prerequisites, board configuration, framework/library choices and exact versions
- APIs, classes, functions, headers, components and installation commands
- Pins, buses, addresses, channels, interrupts, DMA, timers, pulse counters, memory, partitions, bandwidth, power and other resources
- Minimal procedure or code and links to small known-good examples
- Advanced/integrated projects using each major approach
- Alternatives and a decision table explaining when and why to choose each
- Initialization, teardown, concurrency and recovery behavior
- Pin/resource conflicts and realistic simultaneous-use scenarios
- Silicon limits, board limits, framework limits and observed/reported limits, clearly distinguished
- Known-good combinations, known failures, reports of compatible/incompatible peripherals, symptoms and workarounds
- Debugging, diagnostics, pitfalls, safety constraints and unresolved questions
- Evidence status: official, built, hardware-tested, community-reported, inferred or untested
- Applicable hardware revision, framework/version range, confidence and last verification date

The device README must include a **Common tasks / How do I...?** index linking every major capability directly to its guide. A user should not need to infer which architectural document contains an implementation procedure.

### 13. Reuse common development knowledge

Put device-independent procedures in `doc/hardware/guides/<domain>/<guide>/README.md`, such as generic ESP-IDF Bluetooth setup, Arduino Wi-Fi provisioning, I2C diagnosis, pulse-counter use, display benchmarking, SDMMC mounting or safe power testing. Common guides contain reusable concepts, APIs, alternatives, validation and generic pitfalls without unqualified device pin assignments.

Device feature guides link to common guides and clearly state one of:

- The standard approach applies unchanged.
- The standard approach applies with listed device-specific setup/deltas.
- The standard approach is incompatible; use the documented device-specific method and explain why.

Keep bidirectional links between common and device-specific guides. Do not duplicate generic procedures unless local variation requires an explicit adapted copy.

Also place recurring buying, market and workload evaluation knowledge in `doc/hardware/guides/markets/<genre>/README.md`, for example routers, SBCs, development boards, mini PCs, NAS systems or handhelds. Genre guides define buyer/workload taxonomy, meaningful specifications and benchmarks, dated price tiers, common architectural tradeoffs, comparison traps, minimum evidence requirements and alternatives by tier. Device dossiers link the applicable guide and state device-specific deltas; never inherit a generic recommendation without checking current price, revision, software and workload evidence.

### 14. Record vendor documentation-sourcing knowledge

Manufacturer documentation portals have durable, reusable patterns and traps that apply to every future device or chip from that vendor. Capture them in `doc/hardware/vendors/<manufacturer>/README.md` rather than burying them in one device or component record. Create or extend this guide whenever research required non-obvious effort to locate, enumerate or validate official material.

Each vendor guide should record, with live-verified examples and dates:

- Which hosts serve which document classes, and their relative stability
- Exact URL templates for documents, downloads, images, wikis, APIs and immutable revisions
- Version and per-target path structure, and which paths are unstable such as `latest`
- How to enumerate everything published for one product, including sources not present in rendered HTML such as wikitext, APIs, sitemaps, release pages or archives
- Document-class checklists so predictable items are not missed, for example datasheet, reference manual, errata, design guidelines, programming guide, product change notices, application notes, certifications and module variants
- Known migrations where documents moved between hosts or projects, and how to detect and recover from them
- Access traps such as SPA/JS shells, soft 404s, WAF-blocked parameters, redirects, login requirements, region variants and language variants
- Validation requirements, since portals commonly return HTML with document-like URLs
- Rights and licensing observations that recur across the vendor's material
- Working command examples and a per-product checklist

Link vendor guides from the relevant device and component records, and link back to the products that produced the findings. When a documented pattern later fails, update the vendor guide with the corrected observation and its date instead of silently working around it.

### 15. Map coverage, resources and compatibility

Create:

- `coverage.md`: every feature mapped to hardware, official docs/examples, community examples, selected examples, feature guide, build/hardware/limit test status and gaps.
- `resources-and-conflicts.md`: GPIOs, buses, addresses, DMA, timers, PWM, pulse counters, interrupts, memory, flash partitions, USB paths, clocks, power rails, boot straps, muxes, ownership and arbitration.
- `compatibility-and-status.md`: working, partial, failing, untested and conflicting reports keyed by hardware revision and software/library version.

Evaluate combinations such as display + SD + audio or Bluetooth + Wi-Fi rather than considering each feature only in isolation. Preserve negative and version-specific reports. Distinguish firsthand tests, official claims, credible community reports and inference.

### 16. Characterize performance by workload

Create `performance.md` for applicable compute, accelerator, memory, storage, boot, application latency, network, wireless, VPN, routing, firewall/IDS, container, virtualization, thermal and power workloads. Every result records device/configuration/revision, software/firmware/kernel/drivers, test method, environment/topology, sample count, result/units, variability or percentiles, clocks/governor/cooling, power/thermal state, source and evidence status.

For network/VPN/container results include interfaces/link rates, peers, protocol/cipher/MTU, payload, concurrency, topology, CPU utilization and native baseline. For power include measurement point/instrument; for thermal include ambient, warm-up, duration and throttling. Do not merge unlike benchmark versions, configurations or methods into a ranking, infer application performance from component specifications alone, or hide bottlenecks/confounders.

Explain what results mean for real workloads and cumulative feature combinations. Distinguish official claims, silicon limits, independent measurements, isolated owner reports, locally reproduced results and estimates. If evidence cannot support a comparison, mark it unresolved rather than inventing precision.

### 17. Handle licensing and storage honestly

- Public availability does not imply redistribution permission, and a user request cannot grant third-party redistribution rights.
- Identify applicable terms from authoritative license text, download terms, publisher policy, and license/notice files found inside downloaded artifacts. Record the exact license expression/name, evidence URL or retained license path, scope, and obligations. Do not infer that one license covers unrelated neighboring files.
- Preserve license files, copyright notices, source offers, and required attribution.
- Treat an archive as mixed-license when its members have different or incomplete terms. A bundled dependency's license does not cover the surrounding archive.
- Distinguish official source, bundled dependencies, opaque binaries, proprietary tools, and community code.
- Avoid treating installers or unrelated toolchains as normal repository content, but download them to temporary inspection space when they are relevant to understanding versions, bundled files, recovery procedures, or license terms.
- Record intentional duplication caused by retaining archives beside extracted contents.
- Repository size policy and Git LFS do not replace a licensing decision.

Record two independent fields for every artifact:

**Redistribution status** describes known legal permission, not where the file is stored:

- `allowed`: explicit evidence permits repository redistribution with no unmet artifact-specific obligation.
- `conditional`: redistribution is permitted only after recorded obligations are satisfied, such as preserving notices, attribution, source availability, or no-modification requirements.
- `prohibited`: applicable terms prohibit the intended repository redistribution.
- `unknown`: applicable terms were not found, conflict, or have unclear scope.

**Disposition** describes the current or proposed storage behavior and may change after user review:

- `repository`: retained in the repository tree; separately record whether it is `unstaged`, `staged`, or already tracked.
- `local-cache`: downloaded under an ignored, untracked path for local use and reproducible through the acquisition manifest.
- `reference-only`: normally retain metadata and a URL rather than the artifact in the repository; it may still be downloaded temporarily or into a local inspection cache to validate content, hashes, metadata, and embedded license terms.

Treat these fields as research findings and planning aids, not automatic legal judgments. Absence of a discovered license is not a reason to skip acquisition or prevent the artifact from being placed in the repository tree. Unless explicit terms prominently forbid copying or sharing, a useful datasheet, PDF, archive, image, binary, or source tree may be downloaded into the repository but should remain unstaged when its terms are strict or unclear. Explain the evidence, uncertainty, obligations, size, and practical options, then ask the user whether to stage it, move it to local cache, compress it, or keep only acquisition metadata. Record the user's decision.

When the choice materially affects clone size, offline use, access requirements, or uncertain/strict terms, present the user with a grouped list containing license evidence, uncertainty or obligations, original and compressed size where useful, development value, recommendation, and all practical alternatives. Ask what to stage or retain. Categorize at least: clear/common licenses; strict/conditional terms; unknown terms; unusually large files/directories; opaque binaries; vendored source/dependencies; and explicit copying/sharing restrictions. Do not repeatedly ask about routine small artifacts covered by an explicit repository policy.

### 18. Analyze vendored dependencies, near-duplicates and large artifacts

**Default: identical SHA-256 is the only reason to keep a single copy. Anything else, keep both.**

1. **Byte-identical** → keep one; record the other URL as an additional source. Do not store the same bytes twice.
2. **Same name and version, different hash** → **keep both and investigate.** Differing bytes under one version is itself a finding: vendors re-issue documents silently, mirrors lag, and PDF exports differ. Name each copy for its source and state which one the hardware was designed against. *(One chip datasheet was held in three copies — the chip maker's and two board-vendor mirrors — with three different SHA-256 values and sizes. Not redundant copies: different revisions from different portals, indistinguishable without hashing.)*
3. **The same document in two languages** → keep both. Retain the **English** copy in the repository when it is small enough, and archive the rest with a placeholder. State whether the translation is equivalent in substance — that finding saves the next reader from mining it. **If a translation carries material the English lacks, or if no English version exists at all, the non-English document is the primary source** and belongs in the repository on its own merits; say so, and note which language it is.

For bundled libraries, SDKs, source trees, fonts, media, firmware payloads, and generated assets:

4. Identify whether each tree is an unmodified commonly available release, a pinned upstream commit, a patched fork, generated output, or unknown.
5. Compare release/version strings, Git hashes when present, file hashes, directory structure, and representative diffs against upstream sources.
6. **If it differs from upstream, determine why before discarding anything.** Extract the delta as a patch, keep it beside the vendored copy, note what each hunk does and whether it is a local modification or merely an older upstream revision, and **retain the pinned upstream source the patch applies to** so the patch stays meaningful and reproducible. A non-empty diff is *not* proof of a fork — read the hunks; most vendored copies are simply stale.
7. Record the exact version, commit hash, release/tag, dates, vendor source archive/member path, upstream URL, license, local modifications, and confidence.
8. If reproducible from upstream, offer to omit or ignore the bulky copy while keeping detailed reconstruction instructions. Include both the original vendor source and canonical upstream source because the vendor copy may contain patches.
9. If modified or not reproducible, summarize the differences and explain why retaining the vendor copy may matter.
10. Put a README beside an omitted/ignored large tree explaining what was removed or ignored, why, its original size/hash, and exact commands or acquisition-manifest entry needed to restore it.

For large binaries and archives, test lossless compression with appropriate broadly available formats (for example `zstd`, `xz`, or ZIP) in temporary space. Record original/compressed size, ratio, checksum, decompression command, and whether the format preserves required metadata. Do not replace the original silently. Ask whether the user wants the original, compressed form, both, local cache, or reference-only metadata.

## Required Device README Content

```markdown
# <Manufacturer> <Product>

> Product ID, revision/region status, release status and research retrieval date.

## Identity and variants
## What it is and visual identification
## Product history, family and culture
## Key specifications
## Architecture and components
## Common uses and representative projects
## Distinctive strengths
## Shortcomings and constraints
## Performance summary
## Pricing and availability
## Competitors, equivalents and clones
## Launch-era versus current market fit
## When to use / when not to use
## Alternatives by tier
## Images and teardown/PCB views
## Community, editorial and project coverage
## Common tasks / How do I...?
## Documentation map
## Artifact layout
## Known conflicts and unresolved identities
```

The key-specification table should link each functional part to its component record.

## Artifact and Acquisition Manifests

Maintain:

1. An auditable artifact manifest covering files intended for the repository.
2. A committed machine-readable acquisition manifest covering every useful artifact not tracked in Git, including ignored repository files, local-cache files, reference-only records, temporarily inspected files, inaccessible artifacts and failed acquisitions.
3. A committed downloader driven only by the acquisition manifest.
4. A concise license-and-decisions report grouping known permissive licenses, conditional/strict licenses and obligations, prohibited terms, and unknown terms requiring user input.
5. Reproduction instructions for large omitted/ignored vendored trees and binaries.

Each source-artifact record must include:

- Stable ID and artifact kind
- Canonical URL
- Expected SHA-256 and byte size
- Destination relative to the repository root
- Retrieval date and upstream version/date
- License expression/name, evidence, scope, and obligations
- Redistribution status and disposition
- Repository state: `untracked`, `unstaged`, `staged`, `tracked`, `ignored`, `temporary`, or `not-downloaded`
- Extraction format, destination, member-selection rules, and rename/strip instructions
- Target hardware and notes
- Upstream repository URL, release/tag and commit hash where applicable
- Compression alternatives and measured sizes where applicable
- Omission reason, access/authentication requirements, fallback URLs, reacquisition method/status, and last successful verification when not tracked

Use `null` when a value cannot yet be known and explain why. Downloading into a temporary inspection area is encouraged when needed to determine hashes, size, contents, versions, or embedded licensing. Promote validated files into their chosen destination only after inspection.

Extracted members may inherit provenance from a parent source-artifact record, but must record their archive member path and destination. Do not duplicate every extracted member as an independent acquisition when it has no separate source URL. Record failed downloads with URL and reason.

Every firmware record must prominently include, when discoverable:

- Target processor/MCU and compatible hardware revision
- Image role, such as merged factory image, bootloader, partition table, application, filesystem, or coprocessor firmware
- Expected flash offset and required companion images
- Image/version identifier and build date
- SDK/framework/toolchain version and partition details
- Byte size and SHA-256
- Authoritative or inferred flashing/recovery instructions, clearly distinguished

The downloader must:

- Fetch `local-cache` entries by default. Permit explicit `reference-only` inspection/download with a flag, without changing their recorded disposition automatically.
- Refuse redirects to unsupported schemes.
- Download to a temporary file; validate HTTP status, content type/magic, byte size and SHA-256; then install atomically.
- Extract only after validation and reject archive paths escaping the destination.
- Be idempotent and verify existing files before skipping them.
- Preserve required license and notice files.
- Print acquired, cached, skipped and failed paths. It must not commit. Staging should be an explicit mode or separate reviewed step so strict, unclear, and large files remain unstaged until the user decides.
- Support a verify-only mode suitable for CI and post-clone checks.

`acquisition/README.md` must provide clean-clone commands in execution order. For every useful non-tracked artifact, provide exact or best-available download commands, destination, expected size/hash, authentication or manual steps, extraction/member selection, transformations, tool versions, preserved notices, fallback mirrors, verification, and expected failure modes. Mark reacquisition as `automatic`, `manual`, `blocked`, or `lost`; never silently omit instructions because acquisition currently fails.

## Research stopping criteria

Do not claim literal completeness across the changing internet. Continue broad discovery until:

- Every applicable source class and feature-specific query family is searched and logged to a declared depth.
- Product names, IDs, aliases, chip IDs, filenames, firmware IDs and framework terms have been used.
- Every meaningful result has a catalog/ledger disposition.
- Every advertised or fitted feature has official/community evidence or an explicit gap.
- Two consecutive broad search passes produce only duplicates, noise or no new qualifying sources; or remaining sources require private access, unavailable hardware, destructive testing or disproportionate effort.
- High-value examples are sufficiently inspected to select representative minimal, integrated, alternative and limit-testing examples.
- Every useful temporary artifact has been promoted, persistently cached or fully manifested for reacquisition.

Describe the result as a reproducible broad-coverage snapshot with its date, searched sources, query depth and exclusions—not as proof that no other resource exists.

## Conflict Protocol

When credible sources disagree:

1. State the conflict prominently.
2. Quote or summarize each claim with its source.
3. Compare against schematic, physical-function and source-code evidence.
4. Recommend the least risky implementation path.
5. Keep the issue unresolved unless evidence actually proves one interpretation.

Common traps include family versus exact part numbers, inherited driver filenames, stale product-page specifications, bundle names mistaken for revisions, and connector-series names used loosely.

## Evidence discipline and known failure modes

Each of the following has produced a real error in practice. Guard against them explicitly.

**Never promote a candidate to a finding.** Comparison shortlists, "families to check the top marking against", suspected part numbers and search hypotheses must be visually distinct from readings, and must never be copied forward as fact. A part number that differs by one character from a real part on a *different* board is a particularly common trap. Before creating any component record, confirm the designator and part actually appear in the evidence.

**Prove absence, not just presence.** When testing whether a part or net exists, enumerate the full set — a complete reference-designator census across every sheet — rather than searching for the one you expect. Gaps in a designator sequence are themselves evidence, usually that a sheet was never published.

**Record negative results with the same rigour as positive ones.** A refuted hypothesis must be written down with the evidence that refuted it and the date, or it will be re-investigated indefinitely. State plainly when the existing documentation was already correct.

**Distinguish an explicit no-connect from an omission.** A drawn no-connect marker is a positive assertion by the designer; a blank cell is merely unknown. They justify different confidence.

**Verify the text layer before trusting extracted text.** PDFs may use glyph subsetting or a uniform code-point offset that yields *human-legible but wrong* text — the most dangerous failure, because it looks fine. Cross-check extracted strings against a known heading, and fall back to the publisher's HTML build or a rendered page image. Never transcribe values from a text layer you have not validated.

**Read a document's own metadata.** Title fields, producer strings, footers and draft markings frequently betray that a datasheet was derived by editing another part's document, which explains inherited conventions and tells you which source to trust when they disagree.

**Pin the locale when hashing trees.** Directory listings sort differently under different locales, so the same directory can yield two different digests. Always record the exact recipe alongside any tree digest.

**Separate software manuals from hardware documents.** SDK and framework guides often render hardware constants symbolically and explicitly defer to the reference manual. Do not treat them as a source for peripheral counts, pin assignments or electrical limits.

**Deduplicate after parallel work.** Independent workers converge on the same artifacts under different filenames. Hash all artifacts at the end, keep the most explicitly versioned name, and handle the rest under the normal removal policy.

## Working alongside other sessions

Multiple agents may write to the same knowledge base concurrently.

- Establish ownership before editing. Check recent modification times; treat a tree modified within the last hour as actively owned.
- Never edit another session's in-progress tree. Link to it instead.
- Re-read any shared index or file immediately before editing it, and append rather than rewrite.
- Expect shared indexes to change under you; verify your edits survived.
- Do not stage, revert or "fix" another session's files, and do not adopt their untracked work.
- Report suspected external interference rather than repairing it unilaterally.

## Verification

**Verify as you go.** These are the properties the work must hold, not a batch job to run once at the end. Check each one *at the moment you create the thing it applies to* — hash an artifact when you download it, write the placeholder when you move the file, cite the component when you add the device row. A final pass then **confirms** rather than **discovers**, and takes minutes instead of unpicking a week of accumulated drift.

Re-running every check across every file on every pass is wasteful and, worse, encourages skipping them. Run the full sweep when finishing a device, when reorganising anything shared, or when auditing someone else's work.

Before completion:

1. Recompute inventory counts and byte totals from actual files.
2. Compute SHA-256 for retained artifacts.
3. Verify archive integrity.
4. Validate PDF and binary magic/type.
5. Confirm filenames are portable ASCII or document necessary exceptions.
6. Check authored relative Markdown links resolve. **Archive paths need not resolve** — the archive is machine-local and absent from a fresh clone — but every citation of an archived artifact must point at a placeholder that satisfies the [placeholder contract](#placeholders-must-stand-alone). Keep archive paths accurate anyway; if you reorganise the archive, repair the references in the same change.
7. State that bundled upstream Markdown is excluded if it was not link-checked.
8. Check bidirectional device/component links, and for each archived artifact confirm the **three-link chain**: the placeholder exists at the file's former path, the owning component/vendor record cites it, and **every device that caused or uses the fetch cites it too**. The third link is the one that gets forgotten — a reader starting from a board record must reach the archived document without already knowing which component owns it.
9. Ensure every URL has a retrieval date and version/date where available.
10. Compare known official hashes and firmware hashes.
11. Detect duplicate hashes, quantify redundant bytes, and explain intentional duplication.
12. Confirm failed downloads were not retained under misleading extensions.
13. Run repository status and ensure only intended paths changed.
14. Validate every source row has class, retrieval date and version/date or an explicit `unknown`.
15. Validate every artifact has complete direct or inherited provenance, license evidence/scope, redistribution status, and disposition.
16. Flag tracked or staged artifacts whose disposition, redistribution evidence, or conditional obligations conflict with the recorded user decision; report them for review rather than silently changing or deleting them.
17. Spot-check claim-to-source coverage for identity, specifications, pinouts, fitted parts and firmware/recovery claims.
18. Confirm every `local-cache` path is ignored and absent from Git's index.
19. Validate the acquisition-manifest schema and run the downloader in verify-only mode.
20. Confirm license reporting separately lists known licenses, conditional/strict obligations, prohibited terms, and unknown terms requiring user decisions.
21. Verify vendored dependency identity, version/tag/commit/date, modification status, and reconstruction instructions where applicable. Where a vendored tree differs from upstream, confirm the **delta is retained as a patch**, its hunks are characterised (local modification versus merely stale upstream), and **the pinned upstream revision the patch applies to is retained** so the patch stays reproducible.
22. Measure major directory/file contributors and test useful compression options for large binaries or archives.
23. Confirm the user-facing decision report lists all unstaged uncertain/strict artifacts, unusually large paths, binaries, and explicit restrictions with recommendations.
24. Validate the research log covers every applicable source class, feature query family, declared result depth, dead-link recovery and stopping criterion.
25. Confirm every discovered meaningful example has a catalog disposition and immutable revision when available.
26. Confirm selected examples collectively cover all major features, distinct approaches, integration, diagnostics and relevant resource-limit scenarios; document uncovered features.
27. Build or statically validate selected examples when feasible with pinned dependencies, retaining exact failure output and status.
28. Confirm every advertised/fitted capability appears in `coverage.md`, has a natural-language task link in the device README and links to a feature guide or explicit gap.
29. Validate each feature guide covers software options, commands, resources/conflicts, examples, alternatives, limits, known-good/failing reports, debugging and applicability metadata.
30. Validate common guides and device-specific deltas are bidirectionally linked and do not hide device exceptions.
31. Check resource conflicts for realistic concurrent feature combinations and distinguish silicon, board, framework and observed limits.
32. Confirm every meaningful component/feature has a compatibility/status record including negative or conflicting evidence.
33. Verify every useful artifact not tracked in Git satisfies the [placeholder contract](#placeholders-must-stand-alone) — hash, byte size, version or commit, retrieval date, and at least one reacquisition URL — so it can be recovered **without** access to the archive. Record blocked or lost reacquisition honestly rather than omitting instructions.
34. Confirm no useful or uniquely acquired artifact/information remains only in temporary storage.
35a. Confirm available vendor driver/firmware source was audited against the component datasheets, with findings citing file, line and the contradicting datasheet section, and active defects distinguished from inert ones.
35b. Confirm no component record, part number or designator was created from a candidate list, suspicion or near-miss part number rather than a direct reading.
35c. Confirm refuted hypotheses and confirmed-correct existing documentation are recorded with evidence and date.
35d. Confirm extracted PDF text was validated against a known heading or rendered page before any value was transcribed.
35e. Confirm tree digests record their exact recipe including locale, and that artifacts were hash-deduplicated after parallel work.
35. Validate `commands.md` preserves consequential commands with context, versions, expected results and success/failure status, and that reusable multi-step workflows have scripts where practical.
36. Confirm forums, blogs, reviews, social posts, videos, teardowns and public projects were searched where applicable and cataloged by medium/evidence type.
37. Verify community claims distinguish measurement, firsthand report, demonstrated project, interpretation, opinion, hearsay and unsupported assertion; reject prevalence claims without defensible sampling.
38. Confirm useful front/back/port/teardown/PCB/use images were sought and every retained/embedded image has source-page provenance, creator when known, dates, revision applicability, rights evidence, modification history, caption and alt text.
39. Verify launch and current pricing are separate, dated and tied to exact region/configuration/condition/seller; validate marketplace variants and expose shipping, tax/duty, currency date, stock, authenticity and sample-count caveats.
40. Confirm used, refurbished, clone and genuine-device observations are not silently merged and each comparison defines cohort, workload, configuration and material differences.
41. Reject claims such as `equivalent`, `faster` or `better value` when relevant requirements, configurations or methods are materially incomparable.
42. Verify workload results include revision, software versions, method, environment/topology, samples, units, variability/percentiles and power/thermal state where applicable.
43. Confirm official specifications, theoretical limits, estimates, anecdotes, independent measurements and locally reproduced results remain distinct.
44. Confirm launch-era market fit uses contemporaneous evidence while current fit uses dated current prices, software status and alternatives.
45. Verify use/not-use recommendations name workload, constraints, date/region and alternatives by appropriate tier, including marketplace and used options where relevant.
46. Confirm applicable market/genre guides are linked with explicit device-specific deltas rather than inherited stale conclusions.
47. Confirm every technical table is accompanied by prose explaining practical meaning, applicability, limitations and uncertainty.
48. Confirm a vendor documentation-sourcing guide exists or was updated for each manufacturer whose material required non-obvious discovery, including URL templates, enumeration method, document-class checklist, known migrations, access traps and validation notes.
49. Confirm vendor guides are cross-linked with the device/component records that produced the findings, and that superseded patterns were corrected with dates rather than silently bypassed.
50. Confirm useful comparative/performance datasets have reproducible charts where visualization adds value, with sourced data, units, configurations, dates, uncertainty and generation scripts.
51. Confirm multi-language material follows policy: English retained in the repository where size allows, other languages archived with a placeholder, and an explicit statement of whether translations are substantively equivalent — or, where no English version exists, that the non-English document is the primary source.

Store a concise verification report under `doc/hardware/` and link it from the umbrella README.

## Completion Standard

The research is complete only when:

- The exact device and variants are clearly identified.
- Official specifications and applicable artifacts are represented; firmware/examples are included or marked `Not applicable`.
- Every useful artifact found is vendored by default or has a recorded excessive-size/explicit-restriction/access/redundancy reason plus complete reacquisition instructions.
- Every firmware-relevant component/interface has a reusable record and crosslinks.
- Pinouts, buses, addresses, architecture and development workflow are documented.
- Every meaningful feature has a task-oriented development guide, evidence, examples, alternatives, pitfalls, conflicts, limits and status.
- Broad official/community example discovery is logged, meaningful candidates are cataloged and representative best examples are selected.
- Reusable common guides are linked with explicit device-specific applicability and deltas.
- Feature coverage, resources/conflicts, compatibility and negative results are mapped.
- Product identity, appearance, history, family/ecosystem, release/lifecycle and community culture are documented with visual/source provenance.
- Forums, blogs, reviews, social posts, videos, teardowns and real projects are cataloged with evidentiary limitations.
- Launch and current pricing, availability, competitors, substitutes and clones are compared using dated normalized observations.
- Workload-specific performance and shortcomings are contextualized in both technical tables and explanatory prose.
- Launch-era and current market fit, use/not-use guidance and alternatives by tier are evidence-backed and scoped.
- Applicable reusable market/genre guides are linked with explicit device-specific deviations.
- Vendor documentation-sourcing patterns, migrations and traps are recorded in reusable manufacturer guides and cross-linked.
- Sources include retrieval dates, versions and provenance.
- Unknowns, failed downloads and source conflicts are explicit.
- Local files have checksums and validated types.
- Indexes and authored relative links pass verification.

Finish with a concise summary containing the device/component paths; searched source classes and depth; discovered/selected/vendored example counts; feature-guide coverage; working/partial/failing/untested counts; vendored, ignored, cached and omitted artifact counts/bytes; largest paths and compression options; reacquisition status; verification result; unresolved gaps; user decisions still needed; and whether anything was staged or committed.
