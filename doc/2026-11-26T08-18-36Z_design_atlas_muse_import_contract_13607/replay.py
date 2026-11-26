#!/usr/bin/env python3
"""Reference replay of the atlas-muse-import/v1 contract over fixture.json.

This is an independent reading of note.md, not the importer: it feeds each
step's delivery to a small in-memory model of the contract and compares the
result with the hand-authored `expect` block. Standard library only.

    python3 replay.py [fixture.json]

Exits 0 when every step matches, 1 otherwise.
"""
import hashlib
import json
import sys
from pathlib import Path

SCHEMA = "atlas-muse-import/v1"
RANK = {"public": 0, "internal": 1, "private": 2, "sensitive": 3}
KINDS = {"conversation", "message", "memory", "artifact"}
MANIFEST_FIELDS = (
    "namespace", "export_id", "part", "parts", "scope_id", "scope_seq",
    "access_epoch", "scope_access", "inventory", "observed_at", "mapper",
    "producer_schema",
)
ASSEMBLY_FIELDS = (
    "namespace", "export_id", "parts", "scope_id", "scope_seq",
    "access_epoch", "scope_access", "inventory", "membership_attested",
)


class Refused(Exception):
    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def digest(delivery):
    canon = json.dumps(delivery, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(canon.encode()).hexdigest()


class Importer:
    def __init__(self):
        self.objects = {}     # key -> record
        self.deliveries = {}  # (export_id, part) -> digest
        self.staged = {}      # export_id -> {part: delivery}
        self.scopes = {}      # scope_id -> checkpoint

    # -- validation, all before any mutation ------------------------------
    def check(self, d):
        m = d.get("manifest")
        if not isinstance(m, dict) or m.get("schema") != SCHEMA:
            raise Refused("malformed")
        if any(f not in m for f in MANIFEST_FIELDS):
            raise Refused("malformed")
        if m["inventory"] not in ("complete", "delta") or m["scope_access"] not in RANK:
            raise Refused("malformed")
        if not (isinstance(m["parts"], int) and 1 <= m["part"] <= m["parts"]):
            raise Refused("malformed")
        if m["inventory"] == "complete" and m.get("membership_attested") is not True:
            raise Refused("incomplete-scope")
        scope = self.scopes.get(m["scope_id"])
        if scope:
            if m["access_epoch"] < scope["access_epoch"]:
                raise Refused("stale-access")
            if m["access_epoch"] == scope["access_epoch"] and m["scope_access"] != scope["scope_access"]:
                raise Refused("conflict")
        seen = set()
        for o in d.get("objects", []):
            key, rev = o.get("key"), o.get("revision")
            if (o.get("kind") not in KINDS or not isinstance(key, str) or not key
                    or not isinstance(rev, int) or isinstance(rev, bool) or rev < 1
                    or o.get("op") not in ("upsert", "delete")
                    or ("parent" in o and not isinstance(o["parent"], str))):
                raise Refused("unknown-identity")
        for o in d.get("objects", []):
            key = o["key"]
            if key in seen:
                raise Refused("conflict")
            seen.add(key)
            if o["op"] == "upsert":
                content = o.get("content")
                if not isinstance(content, str) or hashlib.sha256(content.encode()).hexdigest() != o.get("content_sha256"):
                    raise Refused("corrupt-member")
            have = self.objects.get(key)
            if have and (have["scope"] != m["scope_id"] or have["kind"] != o["kind"]):
                raise Refused("conflict")
            if (have and o["op"] == "upsert" and have["revision"] == o["revision"]
                    and have["sha"] != o["content_sha256"]):
                raise Refused("conflict")

    # -- one delivery -------------------------------------------------------
    def feed(self, d):
        m = d["manifest"] if isinstance(d.get("manifest"), dict) else {}
        ident = (m.get("export_id"), m.get("part"))
        dg = digest(d)
        if ident in self.deliveries:
            if self.deliveries[ident] == dg:
                return "duplicate", None, []
            return "rejected", "conflict", []
        try:
            self.check(d)
        except Refused as r:
            return "rejected", r.reason, []
        parts = self.staged.setdefault(m["export_id"], {})
        for other in parts.values():
            if any(other["manifest"].get(f) != m.get(f) for f in ASSEMBLY_FIELDS):
                return "rejected", "conflict", []
        self.deliveries[ident] = dg
        parts[m["part"]] = d
        if len(parts) < m["parts"]:
            return "staged", None, []
        del self.staged[m["export_id"]]
        return ("published", None, self.apply(m, [parts[i] for i in sorted(parts)]))

    def apply(self, m, parts):
        sid = m["scope_id"]
        scope = self.scopes.setdefault(sid, {"high_seq": 0, "access_epoch": m["access_epoch"],
                                             "scope_access": m["scope_access"], "last_complete": None})
        scope["access_epoch"] = m["access_epoch"]
        scope["scope_access"] = m["scope_access"]
        stale, present = [], set()
        for part in parts:
            for o in part["objects"]:
                key, rev = o["key"], o["revision"]
                present.add(key)
                have = self.objects.get(key)
                floor = max(have["revision"], have["retired_at"]) if have else 0
                if o["op"] == "delete":
                    if rev <= floor:
                        stale.append(key)
                        continue
                    rec = have or {"scope": sid, "kind": o["kind"], "revision": 0, "sha": None,
                                   "access": None, "parent": o.get("parent"), "edges": []}
                    rec.update(state="retired", retired_at=rev, by="delete")
                    self.objects[key] = rec
                    continue
                if have and have["state"] == "live" and rev == have["revision"]:
                    continue  # same revision, same bytes: no-op
                if rev <= floor:
                    stale.append(key)
                    continue
                self.objects[key] = {
                    "scope": sid, "kind": o["kind"], "parent": o.get("parent"),
                    "revision": rev, "sha": o["content_sha256"], "access": o.get("access"),
                    "edges": [(e["rel"], e["to"]) for e in o.get("edges", [])],
                    "state": "live", "retired_at": 0, "by": None,
                }
        if m["inventory"] == "complete":
            for key, rec in self.objects.items():
                if rec["scope"] == sid and rec["state"] == "live" and key not in present:
                    rec.update(state="retired", retired_at=rec["revision"], by="inventory")
            scope["last_complete"] = m["export_id"]
        scope["high_seq"] = max(scope["high_seq"], m["scope_seq"])
        return sorted(stale)

    # -- reads --------------------------------------------------------------
    def visible(self, key, caller):
        rank, seen = 0, set()
        rec = self.objects.get(key)
        if not rec or rec["state"] != "live" or rec["scope"] not in caller["scopes"]:
            return False
        scope = self.scopes[rec["scope"]]
        rank = RANK[scope["scope_access"]]
        while rec is not None:
            if key in seen:
                return False
            seen.add(key)
            rank = max(rank, RANK.get(rec["access"], RANK["sensitive"]))
            parent = rec["parent"]
            if parent is None:
                break
            prec = self.objects.get(parent)
            if not prec or prec["state"] != "live" or prec["scope"] != rec["scope"]:
                return False
            key, rec = parent, prec
        return rank <= RANK[caller["ceiling"]]

    def read(self, key, caller):
        if not self.visible(key, caller):
            return {"result": "not-found"}
        return {"result": "found", "revision": self.objects[key]["revision"]}

    def related(self, key, caller):
        if not self.visible(key, caller):
            return "not-found"
        out = {to for _, to in self.objects[key]["edges"] if self.visible(to, caller)}
        out |= {k for k, r in self.objects.items()
                if any(to == key for _, to in r["edges"]) and self.visible(k, caller)}
        return sorted(out)

    def project(self):
        out = {}
        for key, r in self.objects.items():
            if r["state"] == "live":
                out[key] = {"state": "live", "revision": r["revision"]}
            else:
                out[key] = {"state": "retired", "retired_at": r["retired_at"], "by": r["by"]}
        return out


def main():
    path = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).with_name("fixture.json"))
    fx = json.loads(path.read_text())
    steps = {s["id"]: s for s in fx["steps"]}
    imp, want_objects, want_scopes, failures = Importer(), {}, {}, 0
    for step in fx["steps"]:
        d = step["delivery"]
        if isinstance(d, str):
            d = steps[d.split(":", 1)[1]]["delivery"]
        exp = step["expect"]
        outcome, reason, stale = imp.feed(d)
        want_objects.update(exp["effects"])
        for sid, fields in exp["checkpoint"].items():
            want_scopes.setdefault(sid, {}).update(fields)
        problems = []
        if outcome != exp["outcome"]:
            problems.append(f"outcome {outcome!r}, expected {exp['outcome']!r}")
        if reason != exp.get("reason"):
            problems.append(f"reason {reason!r}, expected {exp.get('reason')!r}")
        if stale != sorted(exp.get("stale", [])):
            problems.append(f"stale {stale}, expected {exp.get('stale', [])}")
        if imp.project() != want_objects:
            problems.append(f"objects {imp.project()}, expected {want_objects}")
        got_scopes = {s: {k: v for k, v in c.items() if k in want_scopes.get(s, {})}
                      for s, c in imp.scopes.items()}
        if got_scopes != want_scopes:
            problems.append(f"checkpoint {imp.scopes}, expected {want_scopes}")
        for r in exp["reads"]:
            caller = fx["callers"][r["as"]]
            if "read" in r:
                got = imp.read(r["read"], caller)
                want = {k: r[k] for k in ("result", "revision") if k in r}
                if got != want:
                    problems.append(f"{r['as']} read {r['read']}: {got}, expected {want}")
            else:
                got = imp.related(r["related"], caller)
                if got != r["result"]:
                    problems.append(f"{r['as']} related {r['related']}: {got}, expected {r['result']}")
        status = "ok  " if not problems else "FAIL"
        print(f"{status} {step['id']}: {outcome}{' (' + reason + ')' if reason else ''}")
        for p in problems:
            print(f"     {p}")
        failures += bool(problems)
    print(f"{len(fx['steps']) - failures}/{len(fx['steps'])} steps match")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
