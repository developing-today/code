#!/usr/bin/env python3
"""Build/check a source-occurrence preservation ledger (not atomic requirements).

Usage: python3 -I check_inventory.py --build | --check
Build is deterministic and writes only the two sibling inventory artifacts.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

SOURCES = [
    ("H", "01_handoff_pm_stack_fleet_orchestration.md"),
    ("C", "02_charter_v1_1.md"),
    ("P", "03_full_orchestration_proposal.md"),
    ("O", "04_recovered_original_combined_design.md"),
    ("S", "05_supplemental_raw_note.md"),
]
RULES = [
    ("priority", r"priorit|urgenc|importance|score|band|aging|clump|hyster|explor|sampling|triage|rank|boost|order"),
    ("distribution", r"distribut|producer|consumer|provenance|payload|reconcil|replay|projection|projector|single writer|authority"),
    ("pm-mobile", r"mobile|board|project|pm stack|dashboard|touch|browser|view|status column"),
    ("dispatch-events", r"dispatch|slash|webhook|comment|session|command|event rout|acknowledg|receipt|launch"),
    ("queue-selection", r"queue|claim|eligib|lease|selection|candidate|fairness|capabilit|filter|lane"),
    ("lifecycle-cli", r"lifecycle|fleet agent|subcommand|cli|checkpoint|park|resume|handoff|complete|state"),
    ("hydration-activity", r"hydrat|context|cursor|activity|diff|commit|manifest|timeline|attribution|refresh"),
    ("dependencies-authority", r"depend|block|field|managed|note|journal|title|summary|label|ownership|authority"),
    ("leases-outbox-completion", r"fenc|outbox|transaction|verified|completion|mutation|retry|conflict|recovery|atomic"),
    ("crosscutting", r"scenario|acceptance|test|evidence|error|security|risk|phase|migration|source|requirement|report"),
]
TARGETS = {
    "priority": ("components/9199-distributed-priority.md", "# Priority model, urgency, importance, effective pressure"),
    "distribution": ("components/9200-priority-distribution.md", "# Producers, consumers, precedence, provenance and reconciliation"),
    "pm-mobile": ("components/9201-pm-projection-mobile-control.md", "# PM projection and mobile control surface"),
    "dispatch-events": ("components/9203-direct-dispatch-slash-commands-event-routing.md", "# Dispatch, command grammar and event routing"),
    "queue-selection": ("components/9204-queue-selection-bounded-exploration.md", "# Queue selection and bounded exploration"),
    "lifecycle-cli": ("components/9206-work-item-lifecycle-fleet-subcommands.md", "# Lifecycle and Fleet command surface"),
    "hydration-activity": ("components/9207-context-hydration-incremental-activity.md", "# Hydration bundle and incremental activity"),
    "dependencies-authority": ("components/9208-typed-dependencies-notes-journal-field-authority.md", "# Dependencies, notes, journal and field authority"),
    "leases-outbox-completion": ("components/9209-leases-fencing-outbox-direct-completion.md", "# Leases, outbox and direct verified completion"),
    "crosscutting": ("components/9198-requirement-crosswalk-current-state-census.md", "# Cross-cutting requirements, evidence and acceptance"),
}

def split_units(text):
    # Mechanical occurrence segmentation only. Whitespace and sentence-ending
    # punctuation are delimiters (not occurrences); dots attached to path or
    # identifier text, e.g. .fleet, remain part of the occurrence.
    for m in re.finditer(r"\S(?:.*?\S)?(?=(?:[.!?](?:[\"')\]]*)?\s+)|\n|$)", text, re.S):
        s = m.group().strip()
        if s:
            a = m.start() + len(m.group()) - len(m.group().lstrip())
            # The prior sentence's terminal mark can become the next match's
            # leading token; exclude it only when followed by a separator.
            lead = re.match(r"[.!?]+\s+", s)
            if lead:
                a += lead.end()
                s = s[lead.end():].lstrip()
            if not s:
                continue
            yield a, a + len(s), s

def domain_for(s):
    low=s.lower()
    hits=[name for name,pat in RULES if re.search(pat,low)]
    return hits or ["crosscutting"]

def source_section(text, offset, source_id):
    pattern = r"(?m)^\s{0,3}#{1,6}\s+([^\n]+)"
    if source_id == "O":
        pattern = r"(?<!\S)#{1,6}\s+([^\n]+)"
    found = list(re.finditer(pattern, text[:offset]))
    return found[-1].group(1).strip() if found else "front matter / unsectioned source text"

def conflicts_for(s):
    low=s.lower(); refs=[]
    if re.search(r"\b1250\b|450\s*\(?high|400.?799|priority-high",low): refs.append("CF-001")
    if re.search(r"\bavailable\b|\bfailed\b|\btriage\b|ready\s*/\s*todo|in_progress",low): refs.append("CF-002")
    if re.search(r"authority|authoritative|single writer|source.of.truth|synchroniz",low): refs.append("CF-003")
    if re.search(r"priority label.*remov|label.*removed|priority removed",low): refs.append("CF-004")
    if re.search(r"explor|weighted|density|bucket|epsilon|alpha",low): refs.append("CF-005")
    if re.search(r"dispatch|do.now|immediate",low): refs.append("CF-006")
    return refs

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    g=ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--build", action="store_true")
    g.add_argument("--check", action="store_true")
    args=ap.parse_args()
    # No path resolution, reads, writes, or directory creation before strict
    # argparse has accepted the complete invocation.
    ROOT = Path(__file__).resolve().parents[1]
    OUT = ROOT / "inventory"
    rows=[]; coverage={}; counts={}
    for prefix,name in SOURCES:
        p=ROOT/"sources"/name
        raw=p.read_bytes(); text=raw.decode("utf-8")
        digest=hashlib.sha256(raw).hexdigest()
        units=list(split_units(text))
        counts[name]=len(units)
        coverage[name]={"sha256_repo_bytes":digest,"bytes":len(raw),"characters":len(text),"lines":text.count("\n")+1,
                        "unit_count":len(units),
                        "coverage_method":"all non-whitespace spans segmented on sentence punctuation/newlines; original byte/character spans retained for flattened sources"}
        for n,(a,b,s) in enumerate(units,1):
            # Byte offsets are included for every source; O is explicitly flattened.
            byte_a=len(text[:a].encode("utf-8")); byte_b=len(text[:b].encode("utf-8"))
            line=text.count("\n",0,a)+1
            domains=domain_for(s)
            targets=[{"filename":TARGETS[d][0],"section":TARGETS[d][1]} for d in domains]
            rows.append({"id":f"{prefix}-{n:03d}","source_file":name,
                "locator":{"section":source_section(text,a,prefix),"line":line,"char_start":a,"char_end_exclusive":b,
                           "byte_start":byte_a,"byte_end_exclusive":byte_b,
                           **({"exact_byte_span":True,"flattened_source":True} if prefix=="O" else {})},
                "quote":s[:240],"statement":s,"modality":"example" if re.search(r"\b(example|e\.g\.|for instance|illustrative|suggested|proposed|may)\b",s,re.I) else "requirement",
                "domains":domains,"design_targets":targets,"conflict_refs":conflicts_for(s)})
    # Export hashes are provenance supplied by note.md, not hashes of repository bytes.
    export={"01_handoff_pm_stack_fleet_orchestration.md":"e293349ec31e997ac3330d8df947f121e8aba537477b1bfd6e6f8290b85675c0",
      "02_charter_v1_1.md":"293925e58f5b19f319ec494665ec7646d36f9bdb1f823501ed8211ad8dfd9136",
      "03_full_orchestration_proposal.md":"7d24690d14409e4dc32876a50d1a7ac19fe3599b0ecaed7ae909009baec695d1",
      "04_recovered_original_combined_design.md":"6607fba35aee81e7e4e521e8ff7cb6ee31280823c667b9c22f3766fd30edff86",
      "05_supplemental_raw_note.md":"d5855c278a7f7276adb7bf32ea25da1b41ab978fb3d9e207154d1803d4cf1121"}
    for n,h in export.items(): coverage[n]["sha256_imported_export_as_recorded_in_note"]=h
    jsonl="".join(json.dumps(r,ensure_ascii=False,sort_keys=True)+"\n" for r in rows)
    coverage_doc={"validation":"mechanical source-occurrence preservation only; not an atomic requirement or implementation/issue coverage claim","source_coverage":coverage,
      "inventory_rows":len(rows),"domains":{d:sum(d in r["domains"] for r in rows) for d,_ in RULES},
      "coverage_limits":["Sentence/line segmentation is a traceable preservation aid, not a semantic completeness oracle.","Punctuation separators and whitespace are excluded; punctuation-only spans are not requirements. Attached path punctuation such as .fleet remains in text.","No occurrence is asserted as implemented or covered by an issue; design target suggestions are heuristic and planned only.","Export hashes are transcribed from note.md; repository-byte hashes are computed independently.","Conflict refs are heuristic links to adjudication records in conflicts.md, not automated resolution."]}
    md=["# Source-occurrence preservation ledger","",f"Generated rows: **{len(rows)}**. These are mechanically segmented occurrences, explicitly **not atomic requirements**. Repetitions are retained, not coalesced.","","Targets below mean planned design destinations, not implementation/issue coverage.","","| Source | Units | SHA-256 (repo bytes) |","|---|---:|---|"]
    md += [f"| `{n}` | {counts[n]} | `{coverage[n]['sha256_repo_bytes']}` |" for _,n in SOURCES]
    md += ["","## Coverage by domain","", "| Domain | Row occurrences |","|---|---:|"]
    md += [f"| {d} | {coverage_doc['domains'][d]} |" for d,_ in RULES]
    md += ["","## Validation boundary","", "`check_inventory.py --check` compares sources against the saved manifest and verifies the entire deterministic occurrence ledger (IDs, byte/character spans, quotes, order, and count). It cannot establish semantic atomicity, implementation status, or named-issue coverage. It excludes whitespace and sentence-ending separator punctuation; it retains punctuation attached to identifiers/paths such as `.fleet`.","", "The ledger is a mechanical preservation seed, not the curated requirement/crosswalk deliverable. `source-section-map.md` maps source sections to candidate starter domains. `conflicts.md` records known conflicts and precedence decisions. Do not infer complete requirement coverage from the ledger or from a design-target suggestion.",""]
    if args.build:
        OUT.mkdir(parents=True,exist_ok=True)
        (OUT/"statements.jsonl").write_text(jsonl)
        (OUT/"source-coverage.json").write_text(json.dumps(coverage_doc,indent=2,ensure_ascii=False)+"\n")
        (OUT/"README.md").write_text("\n".join(md))
        human=["# Source-occurrence ledger (mechanical preservation layer)","", "Not an atomic requirement inventory. Repetitions remain separate. Each occurrence is verbatim source text with a stable ID and heuristic planned design destination.",""]
        for r in rows:
            loc=r["locator"]
            human += [f"## {r['id']} — {r['source_file']}:{loc['line']}","",f"**Domains:** {', '.join(r['domains'])}  ",f"**Modality:** {r['modality']}  ",f"**Design targets:** {'; '.join(t['filename']+' '+t['section'] for t in r['design_targets'])}","",r["statement"],""]
        (OUT/"statements.md").write_text("\n".join(human))
        print(f"built {len(rows)} rows")
        return 0
    errors=[]
    try: saved_coverage=json.loads((OUT/"source-coverage.json").read_text())
    except Exception as e: errors.append(f"cannot read saved source-coverage.json: {e}"); saved_coverage={}
    try: existing=[json.loads(x) for x in (OUT/"statements.jsonl").read_text().splitlines() if x]
    except Exception as e: errors.append(f"cannot parse statements.jsonl: {e}"); existing=[]
    if len({r.get("id") for r in existing})!=len(existing): errors.append("duplicate or missing IDs")
    for r in existing:
        name=r.get("source_file"); loc=r.get("locator",{}); p=ROOT/"sources"/str(name)
        if not p.exists(): errors.append(f"missing source {name}"); continue
        t=p.read_text(); a=loc.get("char_start"); b=loc.get("char_end_exclusive")
        if not isinstance(a,int) or not isinstance(b,int) or not (0<=a<b<=len(t)): errors.append(f"bad span {r.get('id')}"); continue
        if t[a:b]!=r.get("statement") or not r.get("statement", "").startswith(r.get("quote", "")): errors.append(f"quote/span mismatch {r.get('id')}")
        raw=p.read_bytes(); ba=loc.get("byte_start"); bb=loc.get("byte_end_exclusive")
        expected_ba=len(t[:a].encode("utf-8")); expected_bb=len(t[:b].encode("utf-8"))
        if not isinstance(ba,int) or not isinstance(bb,int) or (ba,bb)!=(expected_ba,expected_bb) or raw[ba:bb]!=r["statement"].encode("utf-8"): errors.append(f"bad byte span {r.get('id')}")
        if not r.get("domains"): errors.append(f"missing domains {r.get('id')}")
        if not r.get("design_targets"): errors.append(f"missing design target {r.get('id')}")
    for _,name in SOURCES:
        raw=(ROOT/"sources"/name).read_bytes()
        expected=saved_coverage.get("source_coverage",{}).get(name,{})
        if hashlib.sha256(raw).hexdigest()!=expected.get("sha256_repo_bytes"): errors.append(f"saved source hash mismatch {name}")
    # Compare the complete sequence, not only individually valid rows: deletion,
    # duplication, reordering, altered IDs, or uncovered non-whitespace spans fail.
    expected_jsonl="".join(json.dumps(r,ensure_ascii=False,sort_keys=True)+"\n" for r in rows)
    if (OUT/"statements.jsonl").read_text()!=expected_jsonl: errors.append("statements.jsonl differs from complete expected segmentation (missing/extra/reordered/altered occurrences)")
    if saved_coverage!=coverage_doc: errors.append("saved coverage manifest differs from the deterministic source inventory")
    if {d for r in existing for d in r.get("domains",[])}!={d for d,_ in RULES}: errors.append("required domains missing")
    if errors: print("\n".join(errors),file=sys.stderr); return 1
    print(f"OK: {len(existing)} mechanical source occurrences; saved hashes and full segmentation match. Not a semantic-completeness or issue-coverage check.")
    return 0

if __name__=="__main__": raise SystemExit(main())
