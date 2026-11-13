#!/usr/bin/env python3
"""Read-only coherence checks for the PM/Fleet orchestration handoff package.

Default mode prints JSON to stdout and never writes, contacts GitHub, or invokes
subprocesses. --live is intentionally a separate, explicit future extension.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import unicodedata
from pathlib import Path

ROOT: Path | None = None


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    modes = p.add_mutually_exclusive_group()
    modes.add_argument("--offline", action="store_true", help="run local package checks (default)")
    modes.add_argument("--self-test", action="store_true", help="exercise pure validation fixtures")
    modes.add_argument("--live", action="store_true", help="reserved until parent confirms publication")
    p.add_argument("--report", type=Path, help="explicitly write JSON report to a coverage/ path")
    return p


def jsonl(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            if line.strip():
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError as e:
                    raise ValueError(f"{path}:{n}: invalid JSON: {e}") from e
    return rows


def alias_errors(specs: list[dict]) -> list[str]:
    by_key = {x.get("key"): x for x in specs}
    errors = []
    if len(by_key) != len(specs):
        errors.append("duplicate issue keys")
    for row in specs:
        target = row.get("alias_of")
        # Existing issue and composite aliases are explicit terminal targets;
        # they are not new spec keys and must not be treated as broken links.
        if target and target not in by_key and not target.startswith(("existing:", "composite:")):
            errors.append(f"{row.get('key')}: missing alias target {target}")
        if target and target.startswith("composite:"):
            parts=target.split(":",1)[1].split(",")
            if not parts or any(part not in by_key for part in parts): errors.append(f"{row.get('key')}: missing composite target {target}")
        seen, key = set(), row.get("key")
        while key in by_key and by_key[key].get("alias_of"):
            if key in seen:
                errors.append(f"alias cycle at {key}")
                break
            seen.add(key)
            key = by_key[key]["alias_of"]
            if key.startswith(("existing:", "composite:")):
                break
    return errors


def heading_slugs(text: str) -> set[str]:
    slugs: set[str] = set(); counts: dict[str,int] = {}
    for line in text.splitlines():
        m = re.match(r"^#{1,6}\s+(.+?)\s*#*\s*$", line)
        if not m: continue
        h = re.sub(r"[`*_~]", "", m.group(1))
        h = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", h)
        base = "".join("-" if c.isspace() else c for c in h.casefold() if c.isspace() or c == "-" or unicodedata.category(c)[0] in {"L","N","M"}).strip("-")
        n = counts.get(base, 0); counts[base] = n + 1
        slugs.add(base if not n else f"{base}-{n}")
    return slugs


def validate_trace(reqs: list[dict], crosswalk: list[dict], statements: list[dict], dispositions: list[dict], commands: list[dict], slices: list[dict], root: Path) -> list[str]:
    errors=[]; ids={r.get("id") for r in reqs}; cids={r.get("requirement_id") for r in crosswalk}
    if ids != cids: errors.append(f"crosswalk ID mismatch missing={sorted(ids-cids)[:10]} extra={sorted(cids-ids)[:10]}")
    dids=[r.get("occurrence_id") for r in dispositions]; sids=[r.get("id") for r in statements]
    if len(dids)!=3571 or len(set(dids))!=3571 or set(dids)!=set(sids): errors.append(f"seed/disposition ID-set mismatch: statements={len(set(sids))}, dispositions={len(set(dids))}")
    aliases={r["id"]:r.get("duplicate_of") for r in reqs if r.get("node_kind")=="alias"}
    groups={r["id"]:r.get("child_requirement_ids",[]) for r in reqs if r.get("node_kind")=="group"}
    if len(aliases)!=40 or len(groups)!=9: errors.append(f"expected 40 alias records and 9 groups, got {len(aliases)}/{len(groups)}")
    for a,t in aliases.items():
        if t not in ids: errors.append(f"{a}: alias target missing: {t}")
        elif next(r for r in reqs if r["id"]==a).get("outcome") != next(r for r in reqs if r["id"]==t).get("outcome"): errors.append(f"{a}: alias outcome differs from canonical {t}")
    for g,children in groups.items():
        if not children or any(x not in ids for x in children): errors.append(f"{g}: group has empty/dangling children")
    # Alias and group edges must be acyclic.
    edges={**aliases,**{g:children for g,children in groups.items()}}
    def visit(k, stack):
        if k in stack: errors.append(f"requirement group/alias cycle at {k}"); return
        v=edges.get(k)
        for child in ([v] if isinstance(v,str) else v or []):
            if child in edges: visit(child, stack|{k})
    for k in edges: visit(k,set())
    for r in crosswalk:
        status=r.get("status")
        if not status or status in {"verified-existing","partially-implemented"} and not r.get("existing_evidence"):
            errors.append(f"{r.get('requirement_id')}: missing status/evidence fields")
        for ref in r.get("design_refs",[]):
            p,sep,frag=ref.partition("#"); file=root/p
            if not file.is_file() or (sep and frag not in heading_slugs(file.read_text(encoding="utf-8"))): errors.append(f"{r.get('requirement_id')}: invalid design anchor {ref}")
    cmap={r.get("key"):r for r in slices}; ckeys={r.get("key") for r in commands}
    if ckeys != set(cmap): errors.append("command inventory and slice-map keys differ")
    for key,row in cmap.items():
        targets=[row.get("primary_slice")]+row.get("supporting_slices",[])
        if not row.get("primary_slice"): errors.append(f"{key}: missing primary slice")
        for target in targets:
            if target not in {x.get("key") for x in SPECS_CACHE} and target not in {"V1-BASELINE","SCOPE-OUT-OF-SCOPE","COORD-DECISION","#9241"}:
                errors.append(f"{key}: unresolved command slice {target}")
    return errors


SPECS_CACHE: list[dict] = []


def validate_specs(specs: list[dict], req_ids: set[str], index: dict, root: Path) -> list[str]:
    errors=alias_errors(specs); by={r.get("key"):r for r in specs}
    existing={k for k in index.get("existing",{})}
    for row in specs:
        key=row.get("key"); target=row.get("alias_of"); alias=bool(target)
        if target and target.startswith("existing:") and target.split(":",1)[1] not in {str(v.get("issue")) for v in index.get("existing",{}).values()}:
            errors.append(f"{key}: unknown existing issue alias {target}")
        if target and target.startswith("composite:"):
            parts=target.split(":",1)[1].split(",")
            if not parts or any(p not in by for p in parts): errors.append(f"{key}: invalid composite alias {target}")
        refs=row.get("requirements",[])+row.get("requirement_refs",[])
        vals={x if isinstance(x,str) else x.get("requirement_id") or x.get("id") for x in refs}
        if vals-req_ids: errors.append(f"{key}: invalid requirement refs {sorted(str(x) for x in vals-req_ids)}")
        if not alias and row.get("issue_required") and (not vals or row.get("requirements_status","").startswith("DRAFT") or row.get("mapping_gaps")):
            errors.append(f"{key}: canonical issue has empty/unallocated requirement mapping")
        bodypath=row.get("bodypath")
        if alias:
            if bodypath: errors.append(f"{key}: alias should not own a duplicate body")
            continue
        if not bodypath:
            errors.append(f"{key}: canonical issue body path missing"); continue
        path=root/"issues"/bodypath
        if not path.is_file(): errors.append(f"{key}: missing body {bodypath}"); continue
        body=path.read_text(encoding="utf-8")
        must=[f"<!-- pmo-9196:{key} -->",row.get("title","")]
        for token in must:
            if token and token not in body: errors.append(f"{key}: body omits required content: {token[:60]}")
        for rid in sorted((x for x in vals if x)):
            if rid and rid not in body: errors.append(f"{key}: body omits mapped requirement {rid}")
        for section in ("Owned files", "Starting failing test, oracle, and done check", "Acceptance and negative cases", "Compatibility", "Rollback", "runtime"):
            if section.casefold() not in body.casefold(): errors.append(f"{key}: body missing section/content {section}")
        for ref in row.get("design_refs",[]):
            p,sep,frag=ref.partition("#")
            if sep and (not (root/p).is_file() or frag not in heading_slugs((root/p).read_text(encoding="utf-8"))): errors.append(f"{key}: bad design anchor {ref}")
        if any("no code path owned" in x.casefold() for x in row.get("owned_files",[])) and row.get("kind")=="implementation": errors.append(f"{key}: placeholder owned files on implementation issue")
    return errors


def validate_graph(specs: list[dict]) -> list[str]:
    errors=[]; by={r.get("key"):r for r in specs}; hard={k:set() for k in by}; soft={k:set() for k in by}
    aliases={r.get("key"):r.get("alias_of") for r in specs if r.get("alias_of")}
    def expand(key):
        if key in aliases:
            target=aliases[key]
            if target in by: return [target]
            if target.startswith("composite:"): return target.split(":",1)[1].split(",")
            return []
        return [key] if key in by else []
    domain_parents={"#9196","#9198","#9199","#9200","#9201","#9203","#9204","#9206","#9207","#9208","#9209"}
    for row in specs:
        k=row.get("key")
        for field,dst in (("hard_blockers",hard),("recommended_after",soft)):
            vals=row.get(field,[])
            if not isinstance(vals,list): errors.append(f"{k}: {field} must be a list"); continue
            for val in vals:
                if val in domain_parents: errors.append(f"{k}: whole-domain parent {val} is an invalid hard/soft target")
                for resolved in expand(val):
                    if resolved==k: errors.append(f"{k}: self-edge in {field}")
                    dst[k].add(resolved)
        if any(x in row.get("hard_blockers",[]) for x in row.get("recommended_after",[])): errors.append(f"{k}: hard/soft edge duplicated")
    def cyclic(graph):
        visiting=set(); done=set()
        def walk(k):
            if k in visiting: return True
            if k in done:return False
            visiting.add(k)
            for v in graph[k]:
                if v in graph and walk(v):return True
            visiting.remove(k);done.add(k);return False
        return any(walk(k) for k in graph)
    if cyclic(hard):errors.append("hard dependency graph contains a cycle")
    # Detect exact writable-path overlap. Mentions/consumer references are not
    # treated as ownership; publisher's explicit read-only-consumer notation is.
    owners={}
    for row in specs:
        if row.get("alias_of"): continue
        for raw in row.get("owned_files",[]):
            text=str(raw)
            if "(parent integration owns" in text or "read-only consumer" in text.casefold() or "does not own" in text.casefold(): continue
            path=text.split(" ",1)[0].strip("`")
            if "/" in path and not path.startswith("http"):
                owners.setdefault(path,[]).append(row.get("key"))
    def reaches(start,target):
        todo=list(hard.get(start,[]));seen=set()
        while todo:
            x=todo.pop()
            if x==target:return True
            if x not in seen:seen.add(x);todo.extend(hard.get(x,[]))
        return False
    for path,keys in owners.items():
        unique=sorted(set(keys))
        for i,a in enumerate(unique):
            for b in unique[i+1:]:
                if not reaches(a,b) and not reaches(b,a): errors.append(f"writable-file collision {path}: {a}/{b} lack hard sequencing or reviewed read-only-consumer declaration")
    return errors


def run() -> dict:
    assert ROOT is not None
    inv = ROOT / "inventory"
    census = ROOT / "census"
    issues = ROOT / "issues"
    reqs = jsonl(inv / "requirements.jsonl")
    disp = jsonl(inv / "dispositions.jsonl")
    req_ids = {r.get("id") for r in reqs}
    specs = jsonl(issues / "specs.jsonl")
    global SPECS_CACHE
    SPECS_CACHE = specs
    index = json.loads((issues / "index.json").read_text(encoding="utf-8"))
    commands = jsonl(census / "commands.jsonl")
    slices = jsonl(census / "command-slice-map.jsonl")
    problems: list[str] = []
    crosswalk = jsonl(census / "requirements-crosswalk.jsonl")
    if len(reqs) != 389 or len(req_ids) != 389:
        problems.append(f"requirements expected 389 unique IDs, got {len(reqs)}/{len(req_ids)}")
    if len(disp) != 3571:
        problems.append(f"expected 3571 occurrence dispositions, got {len(disp)}")
    refs = [x.get("occurrence_id") for r in reqs for x in r.get("source_occurrence_refs", [])]
    if not refs:
        problems.append("requirement inventory has no occurrence references")
    if sum(bool(r.get("duplicate_of")) for r in reqs) != 40:
        problems.append("expected 40 preserved duplicate/alias requirement records")
    if len(commands) != 215 or len(slices) != 215:
        problems.append(f"command/map cardinality expected 215/215, got {len(commands)}/{len(slices)}")
    ckeys = {r.get("key") for r in commands}
    skeys = {r.get("key") for r in slices}
    if ckeys != skeys:
        problems.append(f"command-key mismatch: missing={sorted(ckeys-skeys)[:8]} extra={sorted(skeys-ckeys)[:8]}")
    statements=jsonl(inv / "statements.jsonl")
    problems.extend(validate_trace(reqs,crosswalk,statements,disp,commands,slices,ROOT))
    problems.extend(validate_specs(specs,req_ids,index,ROOT))
    problems.extend(validate_graph(specs))
    refs_by_spec = set()
    for row in specs:
        for ref in row.get("requirements", []) + row.get("requirement_refs", []):
            refs_by_spec.add(ref if isinstance(ref, str) else ref.get("id"))
    existing_ids = set()
    for item in index.get("existing", {}).values():
        existing_ids.update(item.get("requirement_ids", []))
    unresolved = req_ids - refs_by_spec - existing_ids
    # Explicit completion/intake dispositions may live in the curated join.
    join = index.get("requirement_join", {})
    for item in join.get("non_issue_dispositions", []):
        if item.get("requirement_id") in unresolved:
            unresolved.discard(item["requirement_id"])
    if unresolved:
        problems.append(f"requirements without spec/existing/disposition join: {sorted(unresolved)[:12]}")
    body_errors = []
    for row in specs:
        bp = row.get("bodypath")
        if not bp:
            continue
        path = issues / bp
        if not path.is_file():
            body_errors.append(f"{row.get('key')}: body missing: {bp}")
            continue
        body = path.read_text(encoding="utf-8")
        if f"<!-- pmo-9196:{row.get('key')} -->" not in body:
            body_errors.append(f"{row.get('key')}: marker missing")
    problems.extend(body_errors)
    return {"mode": "offline", "scope": "offline-package-audit", "complete_package_verdict": not problems, "counts": {"requirements": len(reqs), "dispositions": len(disp), "commands": len(commands), "command_slices": len(slices), "specs": len(specs), "canonical_specs": sum(not x.get("alias_of") for x in specs)}, "unmapped_requirements": sorted(unresolved), "unmapped_command_keys": sorted(ckeys-skeys), "canonical_specs_with_mapping_gaps": sorted(x.get("key") for x in specs if not x.get("alias_of") and x.get("mapping_gaps")), "publication_pending": sorted(x.get("key") for x in specs if not x.get("alias_of") and not index.get("operations",{}).get(x.get("key"),{}).get("issue_number")), "problems": problems, "live_verification": "not run; no publication approval/notification received", "ok": not problems}


def self_test() -> list[str]:
    failures = []
    if alias_errors([{"key": "A", "alias_of": "B"}, {"key": "B"}]):
        failures.append("valid alias fixture rejected")
    if not alias_errors([{"key": "A", "alias_of": "B"}]):
        failures.append("dangling alias fixture accepted")
    if not alias_errors([{"key": "A", "alias_of": "B"}, {"key": "B", "alias_of": "A"}]):
        failures.append("alias cycle fixture accepted")
    if alias_errors([{"key":"A","alias_of":"composite:B,C"},{"key":"B"},{"key":"C"}]): failures.append("valid composite fixture rejected")
    if not alias_errors([{"key":"A","alias_of":"composite:B,C"},{"key":"B"}]): failures.append("missing composite target accepted")
    if not validate_trace([{"id":"R"}],[{"requirement_id":"X"}],[],[],[],[],Path(".")): failures.append("crosswalk ID mismatch accepted")
    if not validate_graph([{"key":"A","hard_blockers":["A"]}]): failures.append("self edge accepted")
    if not validate_specs([{"key":"A","issue_required":True,"requirements":["NOPE"],"bodypath":None}],set(),{},Path(".")): failures.append("invalid requirement/body fixture accepted")
    if not validate_specs([{"key":"A","issue_required":True,"requirements":["R"],"requirement_refs":[],"bodypath":"missing.md","title":"t"}],{"R"},{},Path("/definitely-not-a-package")): failures.append("deleted body fixture accepted")
    bad_anchor=validate_trace([{"id":"R"}],[{"requirement_id":"R","status":"open implementation","design_refs":["absent.md#not-a-real-heading"]}],[],[],[],[],Path("/definitely-not-a-package"))
    if not any("invalid design anchor" in e for e in bad_anchor): failures.append("bad anchor fixture accepted")
    bad_slice=validate_trace([{"id":"R"}],[{"requirement_id":"R","status":"open implementation","design_refs":[]}],[],[],[],[{"key":"cmd","primary_slice":"NOT-A-KEY","supporting_slices":[]}],Path("."))
    if not any("unresolved command slice" in e for e in bad_slice): failures.append("unresolved command fixture accepted")
    if not any("crosswalk ID mismatch" in e for e in validate_trace([{"id":"R"}],[{"requirement_id":"X"}],[],[],[],[],Path("."))): failures.append("crosswalk mismatch fixture accepted")
    return failures


def main() -> int:
    # argparse fully rejects unknown options before any filesystem access.
    args = parser().parse_args()
    global ROOT
    ROOT = Path(__file__).resolve().parents[1]
    if args.report:
        destination=args.report if args.report.is_absolute() else (Path.cwd()/args.report)
        if ROOT not in destination.resolve().parents or destination.resolve().parent != (ROOT/"coverage").resolve():
            parser().error("--report must name a file directly inside this package's coverage/ directory")
    if args.live:
        result={"mode":"live","ok":False,"scope":"not-run","problems":["Live verification withheld: parent publication notification has not arrived. No network call was made.","After approval, implement/read-only verify actual issue states, exact body readbacks, native parent/blocked-by graph with exhausted pagination, and live own-pilot queue coverage through sanctioned existing adapters."]}
        rendered=json.dumps(result,indent=2,sort_keys=True)+"\n"
        if args.report: args.report.write_text(rendered,encoding="utf-8")
        else: sys.stdout.write(rendered)
        return 2
    if args.self_test:
        failures = self_test()
        result = {"mode": "self-test", "ok": not failures, "problems": failures}
    else:
        try:
            result = run()
        except (OSError, ValueError, KeyError, TypeError) as e:
            result = {"mode": "offline", "ok": False, "problems": [str(e)]}
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.report:
        args.report.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
