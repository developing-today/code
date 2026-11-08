# Lawyer review packet: external GitHub content in Atlas

**Prepared:** 2026-11-08; **For:** counsel review of the five unresolved questions under [#6055](https://github.com/developing-today/code/issues/6055)
**Related records:** [licensing gate #6024](https://github.com/developing-today/code/issues/6024), [policy decision #6052](https://github.com/developing-today/code/issues/6052), and deletion propagation #6054.

> **Purpose and limit.** This is a source-backed question packet for counsel, not legal advice, a legal opinion, or a decision to ratify the provisional policy. The cited terms and statutes are included to frame questions; counsel should determine their application to the actual product, users, corpus, and jurisdictions.

## 1. Decision requested

Please advise whether, and under what conditions, Atlas may retain and search text from public GitHub issue, pull-request, and discussion comments authored by people other than the repository operator. The immediate scope is a private, single-user index for the owner. The review should separately address embeddings, stored text, snippets shown to the owner, and any future access by other people or agents.

The current evidence does not settle whether GitHub ToS §D.6 applies to comments, whether a private index is fair use, or what additional rules apply outside the United States. The questions below intentionally preserve those uncertainties.

## 2. Current interim posture (provisional, not ratified)

The licensing-gate research on #6024 recorded this provisional three-zone posture:

| Source zone | Interim posture recorded on #6024 |
|---|---|
| Repositories owned by Drew or the organization | Ingest may proceed, subject to content classification. |
| External repositories where Drew participated | Private personal search only; no redistribution. |
| External public repositories generally | Private personal search only for full text; factual metadata (such as titles, URLs, timestamps, counts, and hashes) remains outside that full-text gate. |

This packet does not ratify that posture. The separate policy decision #6052 remains open and records Drew's ruling that ratification waits behind router and Wi-Fi work. Until a ruling changes it, the gate research says external Zones 2–3 full text remains personal-search-only; metadata and Zone 1 are not waiting on this packet. The deletion-propagation design is the engineering counterpart to Question 5.

## 3. Questions for counsel

For each question, please identify the controlling facts, applicable jurisdictions, the legal rule or contractual provision, the practical boundary, and any product controls or records needed to rely on the answer.

### Q1 — Does §D.6 reach comment text?

GitHub ToS §D.6 says that when a user adds Content to a repository containing a license notice, the user licenses that Content under the same terms. Does “add Content to a repository” include issue, pull-request, and discussion comment text, or is the provision limited to contributed files, patches, or other repository content? Does the answer differ by surface, repository license, commenter, or whether the comment is a contribution to a change?

**Decision needed:** whether Atlas may treat a repository's license notice as a license for comments, and what evidence (if any) is sufficient to classify the comment as covered. Please distinguish a supported rule from a merely plausible interpretation.

### Q2 — Private, single-user search; text versus embeddings

On the assumed facts below, would a private, single-user, non-commercial index of other people's public comments qualify as fair use in the United States? Does the analysis materially differ among (a) retaining searchable source text, (b) retaining embeddings derived from that text, and (c) showing short snippets to the same user? Which facts would change the analysis (corpus scale, purpose, transformation, security, retention, output controls, or substitution for the source)?

**Decision needed:** whether each of those three operations may proceed, should be limited, or should wait for permission or a different legal basis. Please analyze embeddings separately rather than assuming they are either equivalent to or categorically different from stored text.

### Q3 — Access by anyone other than the owner

How does the answer change if an agent acts for the owner, a family member or collaborator can query the index, or a future ID-graph product publishes or serves results? Where is the practical line between private search and redistribution for (a) snippets in answers, (b) retrieval of larger passages, (c) bulk export, and (d) access to the underlying index or embeddings?

**Decision needed:** define an access/output boundary that the product can enforce. Please distinguish a tool acting solely under the owner's account from another person receiving access, and address whether a nominally private index changes character when its outputs are shared.

### Q4 — EU/UK text-and-data-mining exceptions and opt-outs

Do EU or UK text-and-data-mining exceptions apply to this US-based operator and this proposed personal index? Which connecting facts matter (operator location, users, place of copying, server location, target works, or commercial purpose)? Can GitHub's terms, a repository license, a `robots.txt` entry, or another rights-reservation signal opt out of the relevant exception, and what form and machine-readability are required?

**Decision needed:** identify the jurisdictions and opt-out signals the ingestion pipeline must recognize, and whether the proposed activity can rely on an exception at all. Please separate the EU and UK analyses and note any material post-Brexit divergence.

### Q5 — Deletion, takedown, and ToS license termination

GitHub's current ToS puts the termination language in §D.3; §D.5 contains the license grant to other users. What must Atlas do when a source comment or repository is deleted, made private, or subject to a takedown request? Does any exception permit retention of a tombstone, evidence hash, or derived embedding, and what must be removed from indexes, caches, backups, exports, and downstream copies?

**Decision needed:** specify the trigger, timing, scope, and audit record for deletion or suppression, including treatment of backups and derived data. The sibling engineering design #6054 addresses propagation mechanics; this question is about the legal duty and any lawful retention exception.

## 4. Assumed facts to confirm

Please correct any assumption that changes the analysis:

- Sources are public GitHub repositories and public issue, pull-request, or discussion comments; this is not a request to access private repositories or bypass access controls.
- The initial operator and sole user is the repository owner. There is no public search service, redistribution, or third-party access in the initial design.
- The proposed index may retain source text and embeddings and may return limited snippets to the owner; the packet asks whether those operations should be treated differently.
- Sources may be re-synced. A deleted or inaccessible source can be removed from active retrieval; a minimal tombstone/audit record may be useful to prevent re-ingestion.
- The corpus is collected from GitHub pages or APIs; GitHub's Acceptable Use Policies may apply regardless of collection path and are included below for counsel to assess.
- The system may be operated from the United States, but public sources and potential users can be in the EU or UK. Counsel should identify which additional location facts are outcome-determinative.
- Repository licenses and rights-reservation signals vary and can change over time; the system should not assume all public comments have the same license.

## 5. Source excerpts and source ledger

The quotations below are short excerpts and must be read in their full context at the linked primary sources. Ellipses indicate omitted text. Source wording is distinct from the questions and interim posture above. Pages checked 2026-11-08 unless otherwise stated.

### GitHub's contract and policies

The current [GitHub Terms of Service](https://docs.github.com/en/site-policy/github-terms/github-terms-of-service) state an effective date of **27 April 2026**.

| Source | Short excerpt | Why counsel is asked |
|---|---|---|
| ToS §D.4, License Grant to Us | GitHub may “store, host, archive, parse, display, and make copies of Your Content” for its Service, including AI/ML training. | This grant is to GitHub and its Affiliates; does it grant any right to Atlas? |
| ToS §D.5, License Grant to Other Users | Public content “including issues, comments, and contributions to other Users’ repositories, may be viewed by others”; permitted use is “through the Service as permitted by GitHub’s functionality.” | Viewing is explicit; does the grant extend to an independent corpus or copying comment text off-Service? |
| ToS §D.6, Contributions Under Repository License | “Whenever you add Content to a repository containing notice of a license, you license that Content under the same terms.” | Does a tracker comment count as Content “add[ed] … to a repository,” and which repository license, if any, applies? |
| ToS §D.8, Public Repositories and Lawful Access | “These Terms do not restrict lawful access to or use of the contents of public repositories.” | This does not itself supply a copyright license; what other restrictions determine whether a proposed use is lawful? |
| ToS §D.3, termination | D.4–D.8 licenses “end when you remove Your Content, unless other Users have forked it.” | What follows for Atlas's external copies when the source is removed? The fork exception is framed around GitHub's Service. |
| ToS §G.3, CC0 | The Agreement is CC0-licensed. | This refers to the Agreement; it does not say user comments are CC0. |
| ToS §D.9, Access Reciprocity | Current Terms separately address automated access to public content for development or training of commercially available AI systems. | Likely outside the stated personal-search scope, but counsel should identify what change in use would trigger this clause. |

**API and acceptable-use overlay.** [Section H (API Terms)](https://docs.github.com/en/site-policy/github-terms/github-terms-of-service#h-api-terms) is part of the same ToS; it addresses access, rate limits, sharing tokens, and API abuse, but does not expressly settle external storage of comment text. The separate [GitHub Acceptable Use Policies](https://docs.github.com/en/site-policy/acceptable-use-policies/github-acceptable-use-policies) say the usage restrictions apply whether public information is scraped, obtained by API, or collected another way. Relevant excerpts:

- **§7, Information Usage Restrictions:** the policy permits specified research use of public, non-personal information only if resulting publications are open access, and separately addresses archival use. Counsel should assess whether this private index fits either stated category and how the policy interacts with the ToS and copyright law.
- **§6, Services Usage Limits:** bars reproducing, copying, selling, reselling, or exploiting portions of the Service or access without written permission.
- **§8, Privacy:** calls for reasonable security and prompt responses to complaints and removal requests involving personal information.

The AUP page did not state an effective date in the version checked. These provisions are included for counsel's assessment, not as a conclusion about their application.

### United States: copyright scope and fair use

Primary statutory text: [17 U.S.C. §101](https://uscode.house.gov/view.xhtml?req=granuleid:USC-prelim-title17-section101&num=0&edition=prelim) (definitions), [§102](https://uscode.house.gov/view.xhtml?req=granuleid:USC-prelim-title17-section102&num=0&edition=prelim) (subject matter), [§106](https://uscode.house.gov/view.xhtml?req=granuleid:USC-prelim-title17-section106&num=0&edition=prelim) (exclusive rights), and [§107](https://uscode.house.gov/view.xhtml?req=granuleid:USC-prelim-title17-section107&num=0&edition=prelim) (fair use). The House Office of the Law Revision Counsel's §107 text reflects laws in effect **23 August 2026**.

- §101 defines “copies” as material objects from which a work can be perceived, reproduced, or communicated “with the aid of a machine or device.” Whether a particular embedding retains or permits recovery of protected expression remains a question, not a categorical answer.
- §102(b) excludes ideas, procedures, processes, systems, and methods of operation from copyright protection; §106(1)–(3) lists rights to reproduce, prepare derivative works, and distribute copies to the public.
- §107 directs consideration of purpose and character, nature of the work, amount and substantiality, and effect on the potential market. It supplies no fixed safe snippet length or blanket private-use rule.

Judicial authorities useful as analogies, not decisions about GitHub comments, embeddings, or this product:

- [*Authors Guild v. Google*, 804 F.3d 202, 216–21 (2d Cir. 2015)](https://www.govinfo.gov/content/pkg/USCOURTS-ca2-13-04829/pdf/USCOURTS-ca2-13-04829-0.pdf): upheld full-book digitization for a searchable index and limited snippets on its record.
- [*Authors Guild v. HathiTrust*, 755 F.3d 87, 97–100 (2d Cir. 2014)](https://www.govinfo.gov/content/pkg/USCOURTS-ca2-12-04547/pdf/USCOURTS-ca2-12-04547-0.pdf): upheld a full-text search corpus with no general-user display of underlying text on its record; institutional facts distinguish it from this proposal.
- [*Andy Warhol Foundation v. Goldsmith*, 598 U.S. 508 (2023)](https://www.supremecourt.gov/opinions/22pdf/21-869_87ad.pdf), decided 18 May 2023: emphasizes analysis of the specific challenged use; transformation in meaning alone is not dispositive.

No cited authority establishes a categorical rule for embeddings. Relevant facts for counsel include recoverability of expression from vectors, what source text is retained, what query outputs disclose, use purpose and commerciality, corpus scale, and market effects.

### EU and UK text-and-data mining

- **EU:** [Directive (EU) 2019/790, DSM Directive](https://eur-lex.europa.eu/eli/dir/2019/790/oj/eng), adopted 17 April 2019, published 17 May 2019, is marked in force by EUR-Lex. Article 3 provides a mandatory TDM exception for qualifying research organisations and cultural-heritage institutions. Article 4(1)–(3) provides a broader exception for lawfully accessible works, permits retention “for as long as is necessary” for TDM, and applies only if rights were not “expressly reserved … in an appropriate manner.” Recital 18 says online reservations may be machine-readable and may include “metadata and terms and conditions of a website or a service.” Article 7(1) makes certain contract terms contrary to Articles 3, 5, and 6 unenforceable; it does not list Article 4. Counsel should assess the relevant national transposition, territorial facts, and whether GitHub terms or repository signals qualify as an Article 4 reservation.
- **UK:** [Copyright, Designs and Patents Act 1988 §29A](https://www.legislation.gov.uk/ukpga/1988/48/section/29A), latest revised text, inserted effective 1 June 2014, permits copying by a person with lawful access for computational analysis “for the sole purpose of research for a non-commercial purpose,” with sufficient acknowledgement. Subsection (2) addresses transfer of a copy to another person or use for another purpose; subsection (5) makes contract terms that prevent or restrict the permitted copying unenforceable. This is not framed as a general rights-holder-opt-out exception. Counsel should assess owner-only research, delegation, sharing, and relevant connecting facts separately from EU law.
- The UK government's [18 March 2026 report on Copyright and Artificial Intelligence](https://www.gov.uk/government/publications/report-and-impact-assessment-on-copyright-and-artificial-intelligence/report-on-copyright-and-artificial-intelligence) says it will not introduce copyright reforms until confident they meet its objectives; the existing §29A text remains the statutory source cited here. The report also states “Copyright is territorial,” relevant to identifying where acts occur.

### Deletion, takedown, and personal data

- GitHub ToS §D.3 contains the license-termination language above; §D.5 contains the grant to other users. GitHub's §M.2 discusses its own handling of content after account cancellation and content others have forked; counsel should not assume it creates an external-indexer retention rule.
- [GitHub's DMCA Takedown Policy](https://docs.github.com/en/site-policy/content-removal-policies/dmca-takedown-policy) describes notices, counter-notices, and removal on GitHub. It does not specify what an independent indexer must do with its own copies.
- [17 U.S.C. §512(c)(1)(C), (3)](https://copyright.gov/title17/92chap5.html#512) frames notice-and-takedown conditions for qualifying service providers; it is not a general deletion rule for a private local index.
- EU DSM Directive Article 4(2) and UK CDPA §29A raise retention/transfer constraints but do not prescribe the complete propagation design. The sibling [#6054](https://github.com/developing-today/code/issues/6054) records the engineering proposal: purge text and vectors on source deletion or privatization, retain a non-content tombstone in the trace log, and make treatment zone-aware.
- If comments contain identifiable personal data, [GDPR Articles 3 and 17](https://eur-lex.europa.eu/eli/reg/2016/679/oj/eng) frame territorial scope and erasure rights/exceptions. Counsel should analyze data-protection deletion separately from copyright takedown.

The source texts frame the issues; none of these excerpts is a conclusion that Atlas may ingest or redistribute the material.

## 6. Requested form of counsel's response

For each question, please provide:

1. The conclusion and confidence level, with the facts and jurisdictions on which it depends.
2. The controlling source(s), including any contrary text or uncertainty.
3. A concrete allow / disallow / condition matrix for source text, embeddings, snippets, shared queries, exports, and deletion records.
4. Required controls, evidence to retain, and triggers for re-review (for example, a new jurisdiction, user, output mode, license, or source opt-out).
5. Any issue that requires a separate opinion or specialist review rather than a product-policy assumption.

## 7. References and provenance

- GitHub issue #6055: [lawyer-question packet task](https://github.com/developing-today/code/issues/6055).
- GitHub issue #6024: [external-content licensing gate and provisional research result](https://github.com/developing-today/code/issues/6024).
- GitHub issue #6052: [three-zone policy decision; not ratified](https://github.com/developing-today/code/issues/6052).
- GitHub issue #6054: [source-deletion propagation design](https://github.com/developing-today/code/issues/6054).

This document captures the questions and interim state for one counsel review. It does not change the policy decision in #6052 or authorize a broader corpus, shared search, or redistribution.
