# Design: licensing policy — server ToS uploader grant, account-creation defaults, admin license policies

- **Date:** 2026-11-11
- **Status:** Design (queue task 14805). The ToS text in §2 is a design artifact, to be lawyer-checked before it ships as terms. One question in this design is **not** decided here: the uploads-regime GPL question is an open Drew decision, presented with options and a recommendation in §7. The adoption regime (§1.3) is closed and is not revisited.
- **Issue:** [#5984](https://github.com/developing-today/code/issues/5984), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S90, S94–S102, policy parts, of the 2026-11-04 brief).
- **Inputs:** the license taxonomy & classifier [#5977](https://github.com/developing-today/code/issues/5977) (`doc/2026-11-10_design_id5977_license_taxonomy.md` — the enumerated license set, the five-level warning ladder, and the SPDX-expression tag format this policy consumes verbatim); the C1 licensing ruling on this issue (comment 6650953359, Drew 2026-11-04 — the two regimes of §1.3); the server configuration design [#5753](https://github.com/developing-today/code/issues/5753) (the policy language of §4 is expressible as KDL in that config); the combination checker [#5985](https://github.com/developing-today/code/issues/5985) (consumes license facts at combination events; this policy decides admission, not combination).

## Decision summary

| Question | Decision |
| --- | --- |
| How unlicensed / not-open content may exist on a server | Only under the §2 ToS grant: the uploader attests they are the rights holder (or authorized) and grants the server the rights it needs to operate. Otherwise the artifact must carry an accepted open license (§1.1: the two honest routes) |
| ToS grant scope | §2.1: store, reproduce, distribute, publicly display/perform, compile/execute (code), cache and format-shift, for operating and promoting the service. **Federation sublicense:** granted, to federation peer servers only, on the same terms, revocable by de-federation (§2.2) |
| Revocability | The grant is irrevocable for copies already distributed to third parties in reliance on it; the uploader's prospective remedy is deletion (§2.3). Deletion stops new distribution and removes the object from the deleting server on the deletion SLA; it does not claw back lawful prior distributions or peers' cached copies past their cache TTL (§2.4) |
| Account defaults | §3: one agreement at account creation. Code defaults to **MIT or Apache-2.0** (uploader picks one as their account default); non-code defaults to **CC0-1.0** (stored normalized per #5977 §4) or a no-attribution CC variant. Per-upload override always wins. Pre-agreement uploads are quarantined from public distribution until the account completes the agreement (§3.3) |
| Admin policy language | §4: allowlist / denylist / ladder-ceiling primitives over taxonomy entry ids, families, and traits, scoped per content kind and per principal group, expressed in KDL in the #5753 server config. All four of the issue's example policies are expressed in §4.3 |
| Defaults we ship | §5: ladder ceiling at **MPL** (levels 0–2 of #5977 supported), **NC refused on our servers**, unlicensed accepted **only** under the §2 grant + ownership attestation |
| CC-NC sharp case | §6: on a commercial server the server **refuses NC uploads for public distribution** (identification is the classifier's job — NC artifacts are classified, flagged, and may exist privately / on servers whose policy allows them). Where an NC artifact slips in under a false attestation, the **uploader bears the risk** (attestation + indemnity in the ToS); the server's duty is notice-and-takedown on a rights-holder complaint (§6.2) |
| Uploads-regime GPL | **OPEN — Drew decides (§7).** Recommendation: provisional warn-and-allow under admin control, exactly the C1 ruling's provisional framing, until Drew rules |
| Adoption regime | **CLOSED** (§1.3): our own dependencies are NOT AGPL / NOT GPL. Nothing in this design touches it |

## 1. Principles

### 1.1 The two honest routes

An artifact may be publicly hosted on a server by exactly two routes:

1. **Licensed:** it carries a license from the #5977 enumerated set that the server's policy (§4) admits for its content kind; or
2. **Owned:** the uploader attests they are the copyright holder or are authorized by the holder to grant the §2 rights, and the artifact is hosted under the §2 ToS grant instead of a license.

There is no third route. "I found it somewhere" is neither a license nor a grant, and the upload flow must not pretend otherwise: an artifact whose declared facts fit neither route is refused for public distribution (it may remain in the uploader's private storage, §6.1, where the server's own terms for private storage apply).

### 1.2 Our own standard

Content *we* publish under our own accounts is **MIT or Apache-2.0 for code** and **CC0-1.0 for non-code**, matching the account defaults of §3 — the defaults are our standard, offered to everyone, not a rule we waive for ourselves.

### 1.3 The two regimes (C1 ruling — read before citing this doc on GPL)

The word "GPL" names two different questions in this program, and the C1 ruling (issue #5984, comment 6650953359) gives them different statuses:

- **Adoption regime — CLOSED, firm.** What our own code depends on: NOT AGPL, NOT GPL (the 2026-10-24 hard filter). Settled; this design does not touch it, and no reading of §7 reopens it.
- **Uploads regime — OPEN, undecided.** What users may host is not decided. There is no ban and no support commitment for GPL in user content. §7 carries the decision to Drew.

### 1.4 License UX is a server-side conversation, not an upload tax

Licensing must not be obtrusive to ordinary users. The flow is designed so that:

- The **account agreement (§3) happens once**, at account creation, in plain language;
- The **per-upload surface** is a default already filled in from the account agreement, changeable in one action; a user who never touches it is always honestly licensed, because the default *is* their standing grant/declaration;
- The **policy conversation happens between world authors and server admins** (what a world's server admits), not as per-file interrogation of every contributor. Users submitting code to run inside worlds, and users uploading other content kinds, go through the same account-default machinery — one flow for every kind, kind-specific defaults.

## 2. The ToS grant (design artifact — lawyer check required before ship)

The clauses below are the design's substance, written to be lifted into the terms of service and reviewed by a lawyer before they bind anyone. They are deliberately narrow: the grant gives the server the rights operating a hosting service actually requires, and nothing beyond operating it.

### 2.1 Grant and attestation

By uploading content to a server, the uploader represents and warrants that they are the copyright holder of the content or are authorized by the holder to make this grant, and grants the server operator a **non-exclusive, worldwide, royalty-free license** to:

- **store and reproduce** the content, including backups and redundancy copies;
- **distribute, publicly display, and publicly perform** the content to the audience the uploader selected (public, world members, or named principals);
- **compile, execute, and run** the content where it is code submitted to run on the server or inside a world;
- **cache, transcode, resize, and format-shift** the content as needed to serve it (thumbnails, re-encodings, compiled artifacts), i.e. make the technical derivative copies delivery requires;
- **sublicense the above to federation peer servers** under §2.2, and to infrastructure subprocessors (hosting, CDN, backup) solely to operate the service.

The grant does **not** transfer ownership, does not cover uses unrelated to operating the service (no merchandising, no training-data sale, no endorsement), and does not survive for new content after account termination.

### 2.2 The federation sublicense — decided: yes, bounded

A federated server that cannot pass content to peers cannot federate. The grant therefore includes a sublicense to **federation peer servers**, bounded three ways: (a) only to servers the home server actually federates with, under the same terms — a peer takes the content subject to this grant and the home server's policy outcomes for it; (b) the peer gains no rights the home server lacks (in particular no right to widen the audience the uploader selected); (c) de-federation ends the peer's right to make *new* distributions (existing state follows §2.4's deletion composition). Servers that do not federate never exercise this clause.

### 2.3 Revocability

The grant is **revocable prospectively, irrevocable retrospectively**:

- The uploader may at any time delete the content or narrow its audience; from the deletion SLA onward the server must not make new distributions.
- Distributions already lawfully made in reliance on the grant (a peer's federated copy already served, a world already published containing the artifact) are **not** retroactively infringements. A revocable-forever grant would make every downstream world a latent violation and federation impossible; a never-revocable grant would make deletion meaningless. This split is the design's settlement of the issue's revocability question.

### 2.4 Composition with deletion

When the uploader deletes an object:

1. The deleting server removes it from public distribution within the deletion SLA and stops new distributions immediately on deletion request processing.
2. Federation peers are sent a deletion notice; peers must honor it on the same SLA for their hosted copies. Cached/derived delivery copies (CDN, thumbnails, compiled caches) expire within their normal TTLs, which the server config must keep short (days, not months).
3. Backups age out on the backup rotation; they are never restored into distribution except to restore the uploader's own account state at the uploader's request.
4. §2.3 governs what deletion cannot do: it cannot reach copies third parties lawfully obtained under the grant or under the artifact's declared license before deletion.

### 2.5 Interaction with declared licenses

The grant and a declared license coexist, they are not alternatives on the same axis: the ToS grant runs to the **server** (so it can operate); the declared license runs to **everyone** (so users can reuse). An uploader may not use the ToS grant to launder third-party licensed content into server distribution the license forbids — the attestation in §2.1 is exactly the statement that would be false, and §6.2 places the risk for that falsehood on the uploader.

## 3. The account-default agreement

### 3.1 The one-time agreement

At account creation, before the first upload, the user agrees (plain-language summary shown at signup; full text is §2 plus this section):

> Everything I upload is mine to upload. For **code**, my default license is the one I pick here — **MIT** or **Apache-2.0**. For **everything else**, my default is **CC0** (a public-domain-style dedication — recorded as CC0-1.0 per #5977 §4, with the one sentence of UI copy that spec requires) or, where the picker offers it, a no-attribution CC variant. I can change the license on any single upload. If I upload something under no license, I am granting the server the rights in the terms to store and share it as I direct.

The code-default pick is a real choice the user makes once (MIT vs Apache-2.0 differ — the Apache patent grant — and the design does not hide that difference); the picker shows one line on it. The standing alternative — express no preference and fall back to route 2 of §1.1 (ToS grant, no open license) — is offered explicitly, not as a dark pattern default.

### 3.2 Per-upload override

Every upload carries the account default pre-filled and may override it per artifact: any #5977 enumerated entry, or "my own work under the ToS grant" (route 2). The override is stored as the artifact's declared license expression in the #5977 §6 tag format; the registry never records free-text licenses. An override that the target server's policy (§4) refuses for that kind is refused at upload with the policy reason named — never silently re-licensed.

### 3.3 Uploads predating any agreement

Content uploaded before this agreement existed (or before an account completed it) is **not** retroactively granted anything: no term in this design reaches back. Such artifacts are:

- kept in place but **withheld from new public distribution** (existing links resolve to a "licensing pending" notice, not the content) until the account completes the §3.1 agreement and confirms, per artifact or in bulk with per-artifact override, that the default applies or a specific license/grant does;
- fully usable again the moment that confirmation lands; if it never lands, the content ages with the account and is removed on the ordinary retention schedule, with notice.

Retroactive attestation demands ("click to claim everything you ever uploaded was always licensed") are rejected as a design: the confirmation is expressly prospective from its timestamp, and the registry records it that way.

## 4. The admin policy language

### 4.1 Primitives

A server policy is a set of rules over the #5977 license facts. Three primitives, composable:

- **Allowlist** — admit only these entries/families/traits;
- **Denylist** — refuse these entries/families/traits;
- **Ladder ceiling** — admit nothing whose #5977 ladder level exceeds N.

Each rule is scoped by **content kind** (`code` / `non-code` / `all`) and by **principal group** (`all` / named groups / `admins`). Matching values are taxonomy ids, families, or obligation traits from #5977 §2 — never free text, never license names typed by an admin (the "GPL in the name" example of §4.3 is a *family/trait* match, not a substring match — §4.3, policy 3, says why).

### 4.2 KDL shape (in the #5753 server config)

```kdl
licensing {
    // default policy for everyone, both kinds
    policy kind="all" principals="all" {
        ceiling 2              // ladder ceiling: nothing above MPL's level
        deny trait="noncommercial"
    }
    policy kind="code" principals="curators" {
        allow "GPL-3.0-only" "GPL-3.0-or-later"   // group-scoped exception
    }
}
```

### 4.3 Evaluation semantics

0. **Hard denies first.** A top-level `deny` (outside any `policy` block) is a server-wide constraint: it refuses before any scoped policy is consulted, and no `allow` in any policy overrides it. The shipped NC deny (§5) lives at this tier. Scoped denies, by contrast, are rules of their policy only (step 2).
1. **The unknown gate.** An artifact classified `LicenseRef-Unknown` (#5977 §2.6) is refused for public distribution **before** any ceiling test — explicitly, not by ladder arithmetic: the taxonomy assigns `LicenseRef-Unknown` level 2 (`warning`), so the shipped ceiling of 2 would otherwise *admit* unidentified licenses, which is never intended. The only way past the gate is an explicit `allow "LicenseRef-Unknown"` in a matching policy (an auditable admin decision, e.g. for a named principal migrating legacy content), and passing the gate subjects the artifact to the rest of the test normally.
2. **Scope precedence: the narrowest matching policy decides.** Among policies whose kind scope matches the artifact's kind and whose principal scope contains the uploader, the most specific one decides alone: a principal-named scope beats `principals="all"`, and a kind-named scope beats `kind="all"` (a policy narrow on both axes beats one narrow on either). Broader matching policies do not also apply — this is what lets a group policy *relax* a global allowlist for its members (§4.4, policy 4) instead of merely adding to it. Within the deciding policy: a matching **deny** refuses; else a matching **allow** admits (allows are exceptions *within* a ceiling — an allow names entries the ceiling would otherwise refuse); else the **ceiling** decides (level ≤ ceiling admits); a deciding policy with an allowlist admits only its entries (the allowlist replaces the ceiling test for that policy).
3. **Principal-group exceptions are first-class**, per the issue's requirement — they are the same primitives with a narrower `principals=` scope, not an afterthought bolted onto a global policy.
4. **Refusals are explained**: the upload response names the rule that refused (policy scope, primitive, and the artifact's classified level/family) — an unexplained license refusal is a support ticket, an explained one is the policy working.

### 4.4 The four example policies, expressed

The issue's acceptance test for this language: Drew's four example admin policies, each in the §4.2 shape.

**Policy 1 — "I only want GPL 2 or 3."**

```kdl
licensing {
    policy kind="all" principals="all" {
        allow "GPL-2.0-only" "GPL-2.0-or-later" "GPL-3.0-only" "GPL-3.0-or-later"
    }
}
```

An allowlist policy: only the named entries are admitted, for every kind (GPL entries apply to non-code only via artifacts actually carrying them; the classifier never puts a GPL id on a CC-shaped work).

**Policy 2 — "Only GPL 3."**

```kdl
licensing {
    policy kind="all" principals="all" {
        allow "GPL-3.0-only" "GPL-3.0-or-later"
    }
}
```

**Policy 3 — "Nothing that has GPL in the name."**

```kdl
licensing {
    policy kind="all" principals="all" {
        ceiling 4                        // everything else is admissible
        deny family="strong-copyleft"    // GPL-2, GPL-3
        deny family="network-copyleft"   // AGPL, SSPL
        deny "LGPL-2.1-only" "LGPL-2.1-or-later" "LGPL-3.0-only" "LGPL-3.0-or-later"
    }
}
```

The LGPL line is explicit because the taxonomy files LGPL under `weak-copyleft` (§2.3 of the taxonomy) alongside MPL and EPL — a family deny there would refuse licenses this admin never named. The four entry ids are the whole LGPL population of the enumerated set, so the enumeration is complete, not a sample.

This is the case that proves names are the wrong substrate: "GPL in the name" as a substring rule would miss `SSPL-1.0`'s network-copyleft facts, while family/entry matching expresses the admin's actual intent — no copyleft-by-name licenses — completely, and stays correct as the taxonomy appends entries (#5977 §6: entries are append-only, and new copyleft entries arrive *with* their family, so the family denies cover licenses that do not exist yet; a future LGPL-shaped entry would arrive as a named entry to add to the explicit line, visible in exactly the way a new license entering the set should be).

**Policy 4 — "I only accept MIT or CC0 unless it's code written by these groups or admins."**

```kdl
licensing {
    policy kind="all" principals="all" {
        allow "MIT" "CC0-1.0"
    }
    policy kind="code" principals="worldsmiths" {
        ceiling 2
    }
    policy kind="code" principals="admins" {
        ceiling 2
    }
}
```

The default scope is the two-entry allowlist; the exception is two group-scoped policies (one per group, or one policy naming both — the scopes are lists) that admit code from those principals up to the MPL ceiling. By §4.3's scope precedence, a worldsmith uploading Apache-2.0 code is decided by the group policy alone — the global allowlist does not also apply — so the exception genuinely relaxes the default for code. Non-code from the same groups stays under the default allowlist (their group policies are scoped `kind="code"`), because the issue's exception is about *code written by* those principals.

## 5. Defaults we ship

Out of the box, a server running our software ships:

| Setting | Shipped default |
| --- | --- |
| Ladder ceiling | **2 (MPL)** — #5977 levels 0–2 supported; 3–4 flagged and not admitted for public distribution unless the admin's policy allows them |
| Noncommercial | **Refused on our servers** — top-level `deny trait="noncommercial"` (a §4.3 hard deny, so no group policy can allow it back in on our servers; §6 for the full treatment; self-hosted servers may choose otherwise) |
| Unlicensed | Admitted **only** via §1.1 route 2: the §2 ToS grant **plus** the ownership attestation made at upload (the attestation checkbox is a separate act from having agreed to the ToS at signup — it is per-artifact, because it is per-artifact facts that make it true or false) |
| Account defaults offered | §3 exactly: code MIT/Apache-2.0 pick; non-code CC0-1.0 |

## 6. Creative Commons NC — the risk treatment

### 6.1 Identify and allow to exist

NC work is not contraband. The classifier identifies it (#5977 §2.5: `noncommercial` trait, flagged *differently* from copyleft because the restriction is on field of use, not derivatives), and NC artifacts may exist in private storage and on any server whose policy admits them — self-hosting stance: an admin running a genuinely non-commercial community server may allow NC with a one-line removal of the shipped deny. What *our* servers do is a default, not a capability removal.

### 6.2 The sharp case, settled as far as design can settle it

The case from the issue: a **CC-BY-NC-SA** file uploaded by someone who **does not hold the copyright**, to a server that is **commercial** (it sells Steam keys), granting — falsely — store-and-share rights.

- **What the server does, in order:** (1) *Before the fact:* on our servers the classified NC trait meets `deny trait="noncommercial"` (§5) and the upload is refused for public distribution regardless of the attestation — the commercial server does not take NC distribution risk on purpose. (2) *If classification missed it* (NC terms in an unparseable custom text, facts_source disagreeing): the artifact is held from public distribution when detection later classifies it NC, pending admin review. (3) *On a rights-holder complaint:* notice-and-takedown — the server removes or disables distribution on a facially valid complaint without adjudicating the underlying rights, and notifies the uploader, who may counter-notice. The server never "clears" an NC artifact for commercial distribution itself.
- **Who bears the risk:** **the uploader.** The §2.1 attestation is a warranty; a false attestation is the uploader's breach, and the ToS carries the corresponding indemnity for claims arising from it. The server's exposure is managed by conduct, not by contract alone: refuse-by-default (1), hold-on-classification (2), and prompt takedown (3) are what keep a platform in the good-faith-host posture; the indemnity allocates the residual loss to the party who caused it. Drew's stated instinct — "my servers wouldn't support non-commercial; I don't want to risk dealing with it" — is the shipped default for exactly this reason: the cheapest NC risk is the one never admitted.
- **What the design refuses to do:** accept NC "with uploader attestation" on commercial servers. An attestation from the uploader cannot transfer rights the actual holder never granted, and in the sharp case the attester is precisely the party who does not hold them; attestation-plus-acceptance would be the server knowingly building its catalogue on warranties it has reason to distrust (the work is *labeled* NC — the restriction is on its face). Attestation is the right instrument for *unlicensed own work* (§5), where the attester is normally the holder; it is the wrong instrument for work whose own terms announce a restriction the server would violate by using it commercially.

## 7. The open decision: GPL in the uploads regime (Drew decides)

**Status: OPEN.** This section is the decision surface, per the C1 ruling: the uploads regime is undecided, the "warn-and-allow" reading is provisional framing, and nothing else in this design depends on the outcome — the policy language (§4) expresses every option below, and the classifier (#5977) already keeps the uploads outcome configurable.

| Option | What it does | Pros | Cons / risks |
| --- | --- | --- | --- |
| **A. Warn-and-allow (provisional framing)** | GPL uploads admitted where admin policy allows; default ceiling stays MPL, so GPL needs an admin allow — but *our* servers' default could ship GPL as strong-warning + click-through for the uploader's own GPL work | No content class is banned platform-wide; world authors who write GPL code keep a route; matches the ecosystem users arrive from | Our servers commercially distributing GPL user content is a posture Drew has not endorsed; the click-through becomes the de-facto policy without a decision ever being made |
| **B. Refuse GPL on our servers** | Shipped default adds `deny family="strong-copyleft"` + `deny family="network-copyleft"`; self-hosted servers may allow | Clean, matches the adoption regime's instinct; one story everywhere: "GPL isn't how this platform works" | Bans a large, legitimate body of user work from our servers specifically; pushes GPL authors to self-host or away; contradicts nothing in the C1 ruling but forecloses the "there is use for it" half of it |
| **C. Allow silently (treat as ordinary content)** | GPL is just another entry under the ceiling machinery; no warning beyond its ladder level | Least machinery | Decides by inaction the thing the C1 ruling explicitly left open; strong-warning (its #5977 level) *is* a warning, so "silently" is not honestly available without re-leveling the taxonomy — rejected as an option in disguise |

**Recommendation: A, as a provisional posture with a review date, until Drew rules.** It is the only option that keeps both halves of the C1 ruling alive: no ban (the "not ruled out" half), no silent support commitment (the "not particularly wanting to support it" half — GPL stays above the default ceiling, behind an admin allow and an uploader-facing strong warning, so every GPL artifact on a server is there by two deliberate acts). The decision Drew is actually being asked for is narrow: *the shipped default for our own servers* (options A vs B); the platform capability exists either way because self-hosted admins decide for their servers under §4 regardless.

## 8. Non-goals

- The taxonomy, ladder levels, and tag format (#5977) — consumed, never amended, here.
- Combination rules between admitted artifacts (#5985) — admission (this doc) is necessary, not sufficient, for combination.
- Detection mechanics (#6008) — this policy consumes classified facts, including `LicenseRef-Unknown`.
- Legal advice. §2 is a design artifact for lawyer review; nothing here asserts the grant's enforceability in any jurisdiction.
- The adoption regime (§1.3) — closed by the C1 ruling; dependency-side tooling cites the ruling, not this doc.
