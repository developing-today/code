# Amendment 2 to the decision-model supplement

Drew, 2026-11-13 05:59 CST, amending `brief-decisions-supplement.md` (same program, code#9399). Transcribed faithfully and sectioned. Where this amendment corrects the supplement or the intake record, the correction is marked.

## 1. More free models, now that error sources are fixed

Incorporate **more free models** — several sources that looked dead were error sources we have since fixed (dead doors, plan-coverage artifacts, exhausted-credit masking), so the free inventory for decision pools (and pools generally) should be re-swept against live evidence, not the stale failure record.

## 2. Extend the System 1 contract for our own uses

The supplement's extension layer (§7 there) generalizes: **extend the systemone contract for our own uses** as a superset — the standard stays intact for interop; our extensions (clauses, gauges, tags, classifications below) ride on top.

## 3. Correction + all sources of decision models

- **Correction (Drew): Jev is not a product of Command Code, and many places support Jev.** The intake record's "Command Code's decision model" phrasing is superseded: Jev is a decision model available from multiple sources. The design must **add, support, or consider all sources of decision models**, not one vendor's.
- There are contexts where we might use **Clef** and can **choose between falling back to a small or fast real LLM or using a default, depending on context** — the fallback choice (LLM fallback vs configured default answer) is itself context-dependent and configurable per use.

## 4. Feedback: free-form tags + partial scoring

- Allow **adding tags of any kind with feedback** — the fixed outcome-label set (supplement §5) is a starting vocabulary, not a closed list.
- **Support answering just one scoring gauge**, with the other marked **no / not answered**. Partial feedback is valid feedback; neither gauge requires the other.

## 5. Classifications: build out and align

- Build out and align the **full pool selection**, **how auto works**, and the **provided classifications**.
- Potentially consider **editing classification descriptions**, and **editing the System 1 prompt**.
- Support **adding new classifications**, providing **the default** for each, and **documenting how to override specific classifications for specific session launches using auto**.

## 6. Effort as sway — in all pools, light or strong, per facet

Extends supplement §6:

- Incorporate **low, medium, and high into all pools**, allowing its use to **sway decision-model results lightly or strongly** in various ways — such as **towards cheaper, less capable, or faster**.
- **Generic low/medium/high** expresses how much capability to add — or **speed**, if it's a speed pool.
- **Each facet** can be set to the equivalent of low / medium / high, with a richer stance vocabulary: **slightly favor · strongly favor · slightly disfavor · strongly disfavor · neutral · no answer · defer to orchestrator · etc.**

So effort is not one dial: it is per-facet stances (capability, speed, cost, …) that a decision can weigh, at strengths from slight to strong, including explicit abstention and deferral.
