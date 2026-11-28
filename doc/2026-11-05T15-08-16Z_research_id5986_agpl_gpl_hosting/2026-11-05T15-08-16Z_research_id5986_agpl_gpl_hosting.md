# AGPL/GPL hosting: inspectable worlds, uploads, and self-hosting

**Short answer:** GPLv2 and GPLv3 do not prohibit self-hosting, and server-side execution or network interaction alone does not create their source-distribution duty if users receive no copy of the covered program. AGPLv3 also permits self-hosting, but §13 adds a source-offer duty when a *modified* AGPL program is run so users can interact with it remotely: every such user must receive a prominent, no-charge opportunity to copy the Corresponding Source. A world being inspectable helps only if it exposes the complete source for the exact running version and makes copying it prominent and ordinary; passive visibility, a binary download, or a source hash is not by itself that offer. Whether importing an AGPL library makes an MIT world a covered combined work is a separate, fact-specific copyright question—not something inspectability answers. This is research, not legal advice.

## Scope and assumptions

This is the **upload-regime** question only: a user supplies code/content to a server whose world design is intended to make worlds inspectable by users. The separate adoption regime—our own dependencies remain subject to the 2026-10-24 hard filter “NOT AGPL / NOT GPL”—is unchanged and out of scope. This report does not decide final policy text; issue [#5984](https://github.com/developing-today/code/issues/5984) has the separate policy design and lawyer pass.

“Uploader,” “operator,” and “author” below are roles, not necessarily different people. The uploaded item may be an executable program, a library, source, or non-program content; a license label alone does not establish that it is a copyright-covered Program or that it is combined with other code.

## Findings from the license texts

### GPLv2: copying/distribution, not mere running

GPLv2 §0 grants permission to run the Program without restriction; §§1–2 govern copying, distribution, and modified works. The text has no network-interaction clause. A user interacting with a server-side program does not, merely by that interaction, receive a copy of the server program. If a covered program or derivative is instead distributed to users, GPLv2 §2 governs the covered work and §3 requires complete machine-readable source or an allowed written offer when object code is distributed. Section 3 also permits equivalent source-copy access from the same designated place as object-code access.

The text’s concept of distribution is not framed with GPLv3’s defined “convey” term, so the boundary for a particular upload or online copy must be assessed on its facts. The operational distinction is still important: *running a server* is not itself the same act as *making copies available to users*.

### GPLv3: “conveying” requires enabling copies

GPLv3 §0 defines “convey” as propagation that enables other parties to make or receive copies; it expressly says mere interaction with a user through a computer network, with no transfer of a copy, is not conveying. Section 2 also permits conveying a covered work to a service provider solely to provide running facilities, subject to conditions: it must be exclusively on the conveying party’s behalf and direction/control, and the provider must be barred from making copies outside that relationship.

When covered modified source is conveyed, §5 applies whole-work conditions; when object code is conveyed, §6 requires Corresponding Source by one of its listed methods. Online object-code access can be paired with equivalent source-copy access from the same place at no further charge (§6(d)). The private service-provider permission is not a blanket exception for an upload made available for other users to copy.

### AGPLv3: the additional §13 network offer

AGPLv3 uses GPLv3’s conveyance rules and adds §13. If **you modify the Program**, and your modified version supports remote interaction through a computer network, §13 requires a prominent offer to **all users interacting with it remotely** for an opportunity to obtain that version’s Corresponding Source, at no charge, through a standard or customary means of facilitating software copying. This does not depend on those users receiving a copy of the server program.

The operative condition is modification. Section 13 does not impose its network offer merely because someone runs an unmodified AGPL program remotely. Sections 5–6 still apply if a covered copy is conveyed. The exact party responsible where one person modifies uploaded code and another operates the server is not settled by the license text alone.

### What “source” means here

“Source” is not just a view of selected source files. GPLv2 §3 describes complete source for an executable as including modules, associated interface-definition files, and scripts used to control compilation and installation. GPLv3 and AGPLv3 §1 define Corresponding Source in terms of the source needed to generate, install, run, and modify the object code, including required scripts and certain linked components. A repository page, syntax-highlighted snippet, or provenance hash that omits required material may not be the Corresponding Source.

## Does inspectability satisfy the offer?

Treat inspectability as useful infrastructure, not as a compliance conclusion. The text does not prescribe a literal button, but AGPL §13 does prescribe an offer that is *prominent*, available to all remote users of the qualifying modified version, free of charge, and delivered through a customary copying means. A private source viewer may let a user read code without giving them a practical way to obtain a copy; an ordinary download/copy control prominently attached to the exact running version is much closer to the text. Counsel should review the chosen UX and legal boundary.

There is also a completeness and identity problem: the offer must correspond to the version actually run, not just a same-named repository branch. It should identify the running artifact/version and expose the full Corresponding Source, including build/installation material where required. “Everyone can inspect the world” says nothing by itself about whether server-side dependencies, generated files, build scripts, or linked subprograms are included.

**Current-repository observation (not a claim about the future product design):** at `origin/main` commit `cc45860abb4427fa873f26b81583682af2a7d548`, the world session exposes a hash/provenance record and an authorized participant can download the active compiled module (`pkgs/id/src/world_session/mod.rs:733,1922`; `pkgs/id/src/world_net.rs:1399,1682`). Source-build provenance records digests of compiler, platform, and source inputs, but does not store source text ([module provenance record](../2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md), “does not store source contents”; `pkgs/id/src/world_compile.rs:227,329–335`). Therefore the current artifact download plus hashes are not themselves proof that a user can obtain the corresponding source. The issue’s broader inspectability premise needs a source-package/offer path if it is to support §13.

## Direct answer: a third party self-hosts a world

Self-hosting is not prohibited by GPLv2, GPLv3, or AGPLv3. The answer changes according to what the uploaded thing is and what copies or modifications occur:

1. **An AGPL-licensed asset or text, not executable software:** its license applies to that copyrightable material; merely placing it in a hosted world does not automatically turn the world server into an AGPL program. The grant/permissions for reproducing and exposing the item still matter.
2. **An unmodified AGPL program run by the operator:** remote interaction alone does not trigger AGPL §13; the §13 text is conditional on modifying the Program. Conveying copies to users would still invoke the ordinary applicable terms.
3. **A modified AGPL program that users interact with remotely:** §13 calls for a prominent free source-copy opportunity to all remote users of that version. The operator is the likely product control point for the offer, but the text does not allocate responsibility between uploader, modifier, and operator in every hosting arrangement.
4. **A GPLv2/v3 program run only on the server:** network interaction alone does not trigger the GPLv2/v3 source-distribution conditions. If the operator or uploader lets users download/copy the covered program or a covered derivative, the distribution/conveyance conditions apply.
5. **The AGPL/GPL item is linked, imported, or combined with the world:** first determine whether the result is a covered derivative/combined work or separate independent works. If it is a covered whole, the relevant whole-work terms and source duties can apply. If it is mere aggregation or separate programs communicating at arm’s length, that conclusion does not follow from co-location alone. This boundary needs facts and legal review.

“Uploader” does not automatically mean “operator.” Under GPLv3/AGPLv3 §2, a transfer to a provider can be permitted when it is solely to run the work on the uploader’s behalf and direction/control, with the provider barred from making copies outside that relationship. That narrow permission is not a license to expose the copy to other users. If the uploader also distributes copies to participants, their role and duties change. The author’s authorship alone does not make them responsible for an operator’s later deployment; the license duties attach to the acts of conveying/modifying and, for AGPL §13, the modified remote-interactive version.

## Scenario table: operator × uploader × author

Assumptions for this table: the item is a covered program or relevant source/object code, not merely a name-tagged asset; “users” means participants distinct from the uploader; and roles can overlap. It is a routing aid, not a legal conclusion.

| Actor / act | GPLv2 | GPLv3 | AGPLv3 |
|---|---|---|---|
| **Operator** — runs the server | Running is permitted; there is no network clause. No user-facing source offer follows from execution alone. | Running and mere network interaction without copy transfer are not conveying (§§0,2). | Running is permitted. If the operator modifies a remote-interactive Program, §13 requires a prominent, free Corresponding Source offer to every remote user. An unmodified server program is not enough for §13. |
| **Operator** — lets participants copy code/object | If distributed, preserve notices and satisfy §§1–3; object-code distribution needs complete source or an allowed written offer (§3). | Conveyance triggers the applicable notices, licensing, and source conditions (§§5–6). Online binary access needs equivalent source-copy access under §6(d). | Same conveyance conditions as GPLv3, plus §13 if the remotely interactive version is modified. |
| **Uploader** — submits a copy privately for hosting | The upload makes a copy; the text’s distribution boundary for the particular service arrangement is fact-sensitive. Private server execution alone does not create obligations to every participant. | §2’s service-provider permission may cover a copy sent solely to run on uploader’s behalf and under their direction/control, if the operator cannot copy outside that relationship. | Same GPLv3 service-provider rule; if the uploader modifies and controls the remotely interactive version, §13 may apply to that version. Role allocation with operator needs review. |
| **Uploader** — exposes the copy to other participants | If participants can copy a covered program/derivative, distribution conditions and source rights apply. | Other users’ ability to receive copies is conveyance, not merely interaction; §§5–6 apply to covered works. | Conveyance conditions apply, and §13 also applies to a modified remote-interactive version whether or not each user downloads it. |
| **Author** — publishes own code under this license | A sole copyright owner is not bound by the license they grant: GPLv2’s conditions bind licensees, not the owner distributing their own code (GNU FAQ, “DeveloperViolate”). If the program incorporates third-party GPLv2 code, the author is a licensee of that code and does need its permissions for their acts. | Same owner principle under GPLv3: the owner’s own conveyance of wholly-owned code needs no license permission; §§5–6 govern recipients. Incorporating third-party GPLv3 code makes the author a licensee of that code for their acts. | Same owner principle under AGPLv3: §13 binds a licensee who modifies and remotely runs the Program; a sole owner hosting wholly-owned code owes no §13 offer for their own acts. §13 is relevant where the running version incorporates third-party AGPL code the author holds under license. |
| **Author** — licenses a library later imported by a world | The license governs the library. Whether an importing/linking world is a covered derivative and what must be conveyed depends on the work and distribution facts. | Same boundary; if the combined work is covered and conveyed, §5/§6 conditions apply. §13 permits certain GPLv3/AGPLv3 combinations but does not resolve whether this import is a combined work. | Same; if the resulting covered Program is modified and remotely interactive, §13 adds its source offer. AGPLv3 §13’s GPLv3-combination clause does not make GPLv2-only code compatible. |

**Owner vs licensee (applies to the whole table):** license duties in this table attach to *licensees*. A sole copyright owner who grants the platform rights directly is acting as licensor, not complying with the license, when they upload or host their own code; upload policy must not impose source-disclosure duties on wholly author-owned uploads on that basis. Duties reach an author’s own acts only insofar as the work incorporates third-party copyleft material for which the author is a licensee — which is why the uploader flow (design implication 2) separates “I own this and grant the stated platform rights” from “third-party material under license X.”

## The import/combination question for an MIT world

The label “MIT world” does not settle the scope of a binary or program that imports an AGPL library. A close link, plugin interface, generated/bundled module, intimate data/control-flow integration, or single executable may support a combined-work analysis; separate programs communicating through a general-purpose protocol may point the other way. GPLv3/AGPLv3 §1’s Corresponding Source definition explicitly discusses certain linked subprograms, while §5 says mere aggregation of separate independent works does not extend the license. The text does not define every language-level `import` or decide copyright derivative-work doctrine.

If counsel determines that the runtime combination is a covered whole, “our authored files are MIT” does not automatically permit distributing or operating that whole under only MIT terms. If the combined work is conveyed, GPL/AGPL whole-work terms matter; for a modified, remotely interactive AGPL-covered combination, §13 also matters. If it is a separate independent work, mere co-location is insufficient to impose those results. Treat the classification as a gated policy/checker decision, not as an inference from filename or package metadata.

## Precedents and limits

These are **platform-practice precedents**, not legal rulings on this platform’s license scope.

- **WordPress plugins/themes — software distribution plus explicit source access.** WordPress.org’s [Detailed Plugin Guidelines](https://developer.wordpress.org/plugins/wordpress-org/detailed-plugin-guidelines/) §1 requires code, data, and images in its plugin directory to be GPL or GPL-compatible. Section 4 requires public, maintained source-code and build-tool access, either in the deployed plugin or linked to its development location; it also bars obfuscation. The same guidelines permit service-interface plugins when the plugin’s own code is fully available. Its [Theme Review requirements](https://make.wordpress.org/themes/handbook/review/required/#1-licensing-copyright) apply GPL/GPL-compatible licensing to all files in a submitted theme ZIP. This gives a concrete source-access pattern—include source or link to maintained source—but these directories distribute installable software; they do not establish what counts as source access for a hosted running world.
- **Game-mod repositories — per-project license and uploader rights.** [Modrinth’s Terms](https://modrinth.com/legal/terms) and [Content Rules](https://modrinth.com/legal/rules) distinguish user-owned Gaming Content from platform rights: uploaders authorize display/distribution, must have rights to share, and are expected to identify licenses correctly. Its [project API](https://docs.modrinth.com/api/operations/createproject/) has a required license field and optional source URL. For example, [WorldEdit](https://modrinth.com/plugin/worldedit) is listed GPL-3.0-only, links to source, and offers downloadable versions; [CurseForge’s project-submission guide](https://support.curseforge.com/support/solutions/articles/9000197241-creating-and-submitting-a-project) similarly provides license selection and custom-license handling. These practices support per-artifact license metadata, rights representations, and source links. The catalogs distribute mods for installation; that is different from executing an uploaded mod on the catalog operator’s server.
- **Mastodon — an explicit source URL for AGPL server software, separate from user-post licenses.** Mastodon’s [repository license](https://github.com/mastodon/mastodon/blob/main/LICENSE) is AGPLv3. Its [Instance API `source_url`](https://docs.joinmastodon.org/entities/Instance/#source_url) is defined as the source-code URL for the software running on that server “per the AGPL license requirements,” and the public [instance API](https://docs.joinmastodon.org/methods/instance/#v2) exposes it. Mastodon’s default [Terms of Service template](https://github.com/mastodon/mastodon/blob/main/config/templates/terms-of-service.md) says users retain ownership but grant the operator rights needed to use and distribute content as part of the service, and warrant that they can grant those rights. Its [Status API](https://docs.joinmastodon.org/entities/Status/) documents content and visibility but no per-status license field; a [feature request for license tags](https://github.com/mastodon/mastodon/issues/20079) remains open. This is a particularly relevant product pattern for a discoverable server-software source link, but not a precedent for code users upload for the host to execute.
- **Federated media — content-license metadata can travel with content.** PeerTube’s [upload guide](https://docs.joinpeertube.org/use/create-upload-video) exposes a per-video license field; its [ActivityPub video documentation](https://docs.joinpeertube.org/api/activitypub#video) maps the license field to `schema:license`. This demonstrates content-level license metadata in a federated system, not a software-source offer: the documented choices target videos, not GPL programs.

**Transfer limit:** these systems separately solve (a) distributing installable software, (b) running AGPL-licensed server software, and (c) publishing/federating user media. The proposed world system combines these shapes when it executes uploaded programs. None of the cited precedents says ambient inspectability alone satisfies an AGPL source offer or decides whether a particular import is a combined work. The transferable product controls are explicit license/provenance metadata, a clear uploader-rights path, and version-specific source access that users can copy.

## Design implications for #5984 and #5985

1. **Separate the upload regime from dependency adoption.** This research does not relax the hard filter on the platform’s own dependencies.
2. **Make rights declaration explicit.** The uploader flow should distinguish “I own this and grant the stated platform rights” from “third-party material under license X,” and capture license/version and source/provenance metadata where available. Do not imply that an uploader can grant rights they do not hold.
3. **Model actor and copy paths.** Record who uploads, who controls a running instance, whether the operator modifies/builds the program, and whether another user can copy source or object code. Server execution, private service-provider copies, public downloads, and user interaction are different events.
4. **Treat source access as a product feature.** For a qualifying AGPL §13 case, provide a prominent, no-charge way to copy the Corresponding Source for the exact running version to every remote user. Include the source/build material required by the license; a compiled module hash or binary alone is inadequate evidence.
5. **Do not auto-label every AGPL upload as forbidden.** AGPL permits hosting; it adds conditions. Distinguish source offer/notice requirements from a prohibition, and leave unresolved combined-work cases for an explicit legal/admin decision.
6. **Checker should reason over a composition graph.** Track direct/transitive code relationships and versions; distinguish “same world” from “linked/combined”; flag GPLv2-only compatibility questions separately from GPLv3/AGPLv3 §13 combinations; show the precise source-offer trigger and affected users. Avoid a green result when composition is unknown.
7. **Preserve user choice and evidence.** Keep license notices, source offer links, artifact/version hashes, build metadata, and the decision/exception record together. Provide a user-visible report of the selected license path instead of silently treating inspectability as compliance.
8. **Keep final wording for counsel.** In particular, ask counsel to resolve the service-provider relationship, what counts as a covered combined work for Roc/Wasm imports and plugins, who makes a modification in a hosted-build pipeline, and what UI qualifies as prominent/customary copying access.

## Research boundary and remaining questions

This report reads license texts and platform documentation; it is not an opinion about copyright ownership, derivative-work status, enforceability, or the sufficiency of a particular product UI. The most consequential unresolved facts are: what “world imports library” means in the actual runtime/bundle; whether uploaded source is modified by the platform, uploader, or both; which users receive copies versus only interact; and whether the product’s source package includes all Corresponding Source. The separate lawyer packet #6055 concerns external GitHub ingestion and is not a blocker for this upload-regime analysis.

## Sources

### Primary license texts

- [GNU General Public License, version 2](https://www.gnu.org/licenses/old-licenses/gpl-2.0.html), §§0–3.
- [GNU General Public License, version 3](https://www.gnu.org/licenses/gpl-3.0.html), §§0–2, 5–7, 13.
- [GNU Affero General Public License, version 3](https://www.gnu.org/licenses/agpl-3.0.html), §§0–2, 5–7, 13.

### Repository and issue evidence

- Issue [#5986](https://github.com/developing-today/code/issues/5986), including the 2026-11-05 wave-2 scope comment: upload regime only; adoption regime unchanged.
- Issue comment [#6658978327](https://github.com/developing-today/code/issues/5986#issuecomment-6658978327): prior triage, current dependency map, and the requested deliverable.
- `doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md`: current and planned world hosting, source compilation, and artifact flow.
- `doc/2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md`: provenance is metadata, not source storage or an attestation.
- `pkgs/id/src/world_session/mod.rs`: participant-authenticated active-module download and module-byte serving.
- `pkgs/id/src/world_compile.rs`: source-build provenance hashes source inputs; compiler output is the artifact installed.
- `pkgs/id/src/world_net.rs`: participant module-info and download client APIs.
- [WordPress Detailed Plugin Guidelines](https://developer.wordpress.org/plugins/wordpress-org/detailed-plugin-guidelines/) and [Theme Review requirements](https://make.wordpress.org/themes/handbook/review/required/#1-licensing-copyright).
- [Modrinth Terms](https://modrinth.com/legal/terms), [Content Rules](https://modrinth.com/legal/rules), [project API](https://docs.modrinth.com/api/operations/createproject/), and [WorldEdit listing](https://modrinth.com/plugin/worldedit).
- [CurseForge project-submission guide](https://support.curseforge.com/support/solutions/articles/9000197241-creating-and-submitting-a-project).
- [Mastodon AGPLv3 license](https://github.com/mastodon/mastodon/blob/main/LICENSE), [Instance `source_url`](https://docs.joinmastodon.org/entities/Instance/#source_url), and [default Terms of Service template](https://github.com/mastodon/mastodon/blob/main/config/templates/terms-of-service.md).
- [Mastodon Status API](https://docs.joinmastodon.org/entities/Status/) and [license-tag feature request #20079](https://github.com/mastodon/mastodon/issues/20079).
- [PeerTube upload guide](https://docs.joinpeertube.org/use/create-upload-video) and [ActivityPub video schema](https://docs.joinpeertube.org/api/activitypub#video).

## Change log

- 2026-11-05: Initial research record for issue #5986; direct license-text analysis and repository implementation cross-check.
