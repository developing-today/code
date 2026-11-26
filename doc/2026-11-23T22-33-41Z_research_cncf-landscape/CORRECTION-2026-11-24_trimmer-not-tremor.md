# CORRECTION — "Trimmer" is not Tremor

**Correction date:** 2026-11-24. Drew's correction was stated 2026-11-23
~19:34 CST.
**Applies to:** this record folder — the index page
(`2026-11-23T22-33-41Z_research_cncf-landscape.md`: the status list's
"Tremor is archived" in context, the "Uncertain names" entry
**"Trimmer" → Tremor**, and the data-layer contents list) and
`data-layer.md` §17 ("Trimmer" — an identification, honestly labelled)
and its summary-table row `Tremor ("Trimmer"?)`.
**Convention:** dated records are not rewritten silently. The original
text stands; this note corrects it.

## What the record said

The census interpreted one name in Drew's dictated list, "Trimmer", as
**Tremor** — the Rust event-processing engine, formerly a CNCF sandbox
project, now archived — and recorded that identification as the
overwhelmingly likely referent.

## The correction

Drew has stated plainly: **"Trimmer" was Trimmer — NOT Tremor; the
research's reading was wrong.**

Every place this record equates "Trimmer" with Tremor is therefore
withdrawn. Tremor's own facts as recorded (an event _processor_ that
sits on top of brokers, never a Kafka/NATS rival, now archived) are
unaffected — what is withdrawn is the claim that Tremor is the thing
Drew named.

## Identification result: unresolved

A fresh identification attempt was made on 2026-11-24. It did not find
a credible referent, and no substitute is offered:

- **Original-list context.** The name reached the census's data-layer
  lane as part of Drew's NATS / Kafka-alternative questions. Its
  neighbours in `data-layer.md` §11–17 are NATS, Kafka, Redpanda,
  Pulsar, Redis Streams, and WarpStream/Bufstream — a
  streaming/messaging neighbourhood.
- **CNCF landscape.** The canonical landscape data
  (`cncf/landscape`, `landscape.yml`, checked 2026-11-24) contains no
  entry named "Trimmer" anywhere, and none named "Trimaran" or
  "Streamr". In Streaming & Messaging, Strimzi and Tremor sit adjacent
  to each other — which is how the original misreading was available,
  and is not evidence for it.
- **This repo.** A whole-repo grep for "trimmer" (case-insensitive)
  finds it only in the two CNCF record copies of `data-layer.md`. No
  other project, document, or code here is named Trimmer.
- **Web.** No streaming, messaging, or CNCF project named Trimmer was
  found. Things actually named Trimmer exist in unrelated fields:
  TRIMMER, a software-debloating / application-specialisation system
  (ASE 2018; IEEE TSE 2021; `github.com/ashish-gehani/Trimmer`);
  Trimmer, a deep-learning auto-tuning framework for cloud datacenters
  (IEEE CLOUD 2022, built on Apache TVM); and the Hubble Timescape
  Trimmer, an Isovalent Cilium Enterprise component that limits flows
  ingested into ClickHouse. Sound-alikes weighed and not accepted:
  Strimzi (already covered separately in §12 of the data-layer page —
  a Kafka operator, not a broker), Trimaran (Kubernetes load-aware
  scheduler plugins — platform/scheduling, not streaming), Streamr
  (a decentralised pub/sub network — streaming-shaped, but not in the
  CNCF landscape and with no evidence connecting it to Drew's list),
  and Trireme (Aporeto's container-security project). None fits the
  streaming neighbourhood credibly, and Drew's correction fixes the
  name itself as "Trimmer".

**Conclusion: identification unresolved.** If Drew can say where he
encountered "Trimmer", this note can be amended with the real
referent. Until then, do not re-substitute Tremor (or any other name)
for it.

## Where else the wrong reading appears

Follow-ups — listed here, not edited by this correction, except where
noted:

- The duplicate record of this census:
  `doc/2026-11-23T23-46-55Z_research_cncf-investigation/research/data-layer.md`
  §17 and its summary table. A copy of this correction note is placed
  in that record folder by the same change that adds this one.
- `doc/README.md`, the index entry for this record — annotated by the
  same change.
- The workspace census source:
  `~/workspace/cncf-investigation/research/data-layer.md` §17 and its
  summary table (hatch workspace, outside this repo).
- The CNCF Space: `~/workspace/ts-spaces/cncf-landscape/index.html`
  (data table row `Tremor ("Trimmer"?)`), its built copies under
  `.space-build/` and `.space-build-cvm/`, its `audits/` snapshots,
  and `~/workspace/space-versions/cncf-landscape/v1/`. A Space rebuild
  is a follow-up for the Space's owner.
- The landing PR's body,
  [PR #13827](https://github.com/developing-today/code/pull/13827)
  ("Trimmer→Tremor") — historical text, not edited.
- The 2026-11-23 daily memory log's final-identifications entry
  ("Trimmer → almost certainly Tremor") — memory is maintained
  separately; the curated `MEMORY.md` already carries Drew's
  correction.
