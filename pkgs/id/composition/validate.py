#!/usr/bin/env python3
"""Validate the S1 feature catalogue against today's tree.

S1 of developing-today/code#14056. Design-of-record evidence: the §4
catalogue (catalogue.toml) validates against the tree — every shipped
feature appears, every row names crates that exist, every wiring that
claims to be shipped exists today.

Checks (all strict; any failure exits non-zero):
 1. Every root-cargo feature of pkgs/id is catalogued (by `tree` name)
    or declared unrepresentable (harness/test-only rule).
 2. Every shipped catalogue entry names a root feature (`tree`) unless
    flagged `unconditional` (core-default code with no switch yet).
 3. Every `forwards` entry resolves: `crate/feature` names a feature
    that exists in that crate today; `crate` names a crate dir that
    exists; `dep:name` names an optional root dependency.
 4. Every `requires`, group member, profile feature, `extends`, alias,
    and `group_select` entry names something that exists; aliases never
    collide with canonical profile names; no dependency cycles.
 5. Exclusivity: at most one selected member per group per resolved
    profile (core-default overlap warns until S6 wires the switches).
 6. Unrepresentable names (currency-poc-tb, test-*) appear in no
    feature, profile, or group.
  7. The golden hash vector's feature set equals the catalogue
    resolution of examples/basement-store-node.toml, with owning-crate
    versions matching today's workspace crates.
 8. The S2 variant source matches root defaults, names existing root
    features, and is consumed by Nix, just and build.sh.

Usage: python3 -I validate.py   (run from this directory)
"""
import json
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
ID = HERE.parent  # pkgs/id


def load_toml(path: Path) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)


def cargo_features(cargo_toml: Path) -> set:
    return set(load_toml(cargo_toml).get("features", {}).keys())


errors: list[str] = []
warnings: list[str] = []


def fail(msg: str) -> None:
    errors.append(msg)


def warn(msg: str) -> None:
    warnings.append(msg)


def main() -> int:
    catalogue = load_toml(HERE / "catalogue.toml")
    root_manifest = load_toml(ID / "Cargo.toml")
    root_features = set(root_manifest.get("features", {}).keys()) - {"default"}
    root_defaults = set(root_manifest.get("features", {}).get("default", []))
    root_deps = root_manifest.get("dependencies", {})
    features = {f["name"]: f for f in catalogue.get("feature", [])}
    groups = catalogue.get("groups", {})
    profiles = {p["name"]: p for p in catalogue.get("profile", [])}
    unrepresentable = set(catalogue.get("unrepresentable", []))

    # --- 8. build variant source pin (S2) -------------------------------
    variants = load_toml(HERE / "variants.toml")
    variant_features = variants.get("variant", {})
    source_defaults = set(variants.get("root", {}).get("default", []))
    if source_defaults != root_defaults:
        fail(f"variant source root defaults {sorted(source_defaults)} != Cargo defaults {sorted(root_defaults)}")
    for variant, config in variant_features.items():
        for feature in config.get("features", []):
            if feature not in root_features:
                fail(f"variant `{variant}` names unknown root feature `{feature}`")
    consumers = {
        "flake.nix": (ID / "flake.nix", "variantsConfig"),
        "justfile": (ID / "justfile", "variant-features.sh"),
        "scripts/build.sh": (ID / "scripts" / "build.sh", "variant-features.sh"),
    }
    for name, (path, marker) in consumers.items():
        if marker not in path.read_text():
            fail(f"variant consumer `{name}` does not read the single variant source")

    # --- 6. unrepresentable hygiene -------------------------------------
    for name in list(features) + [f for p in profiles.values() for f in p.get("features", [])]:
        if name in unrepresentable or name.startswith("test-") or name in ("test-utils", "test-provider"):
            fail(f"unrepresentable feature selected or catalogued: {name}")

    # --- 1. every root feature catalogued or declared -------------------
    tree_names = {f.get("tree") for f in features.values() if f.get("tree")}
    for rf in sorted(root_features):
        if rf not in tree_names and rf not in unrepresentable:
            fail(f"root cargo feature `{rf}` is neither catalogued (tree) nor unrepresentable")

    # --- 2/3. entry checks ----------------------------------------------
    for name, f in features.items():
        status = f.get("status")
        if status not in ("shipped", "seam", "designed", "placeholder"):
            fail(f"feature `{name}`: unknown status `{status}`")
        if status == "shipped" and not f.get("tree") and not f.get("unconditional"):
            fail(f"feature `{name}`: shipped needs `tree` or `unconditional`")
        if f.get("tree") and f["tree"] not in root_features:
            fail(f"feature `{name}`: tree `{f['tree']}` not a root cargo feature")
        for req in f.get("requires", []):
            if req not in features:
                fail(f"feature `{name}`: requires unknown feature `{req}`")
        for grp in f.get("groups", []):
            if grp not in groups:
                fail(f"feature `{name}`: unknown group `{grp}`")
        for fwd in f.get("forwards", []):
            if fwd.startswith("dep:"):
                dep = fwd[4:]
                if dep not in root_deps or not root_deps[dep].get("optional", False):
                    fail(f"feature `{name}`: forward `{fwd}` not an optional root dep")
            elif "/" in fwd:
                crate, feat = fwd.split("/", 1)
                crate_toml = ID / "crates" / crate / "Cargo.toml"
                if not crate_toml.exists():
                    fail(f"feature `{name}`: forward crate `{crate}` missing")
                elif feat not in cargo_features(crate_toml):
                    fail(f"feature `{name}`: forward `{fwd}` — no such feature in {crate}")
            else:
                if not (ID / "crates" / fwd / "Cargo.toml").exists():
                    fail(f"feature `{name}`: forward crate `{fwd}` missing")

    # --- requires cycles -------------------------------------------------
    def closure(name: str, stack: tuple) -> set:
        if name in stack:
            fail(f"requires cycle: {' -> '.join([*stack, name])}")
            return set()
        out = {name}
        for req in features[name].get("requires", []):
            out |= closure(req, (*stack, name))
        return out

    for name in features:
        closure(name, ())

    # --- groups ----------------------------------------------------------
    for gname, g in groups.items():
        for m in g.get("members", []):
            if m not in features:
                fail(f"group `{gname}`: unknown member `{m}`")
            if gname not in features[m].get("groups", []):
                fail(f"group `{gname}` lists `{m}` but the feature does not claim the group")

    # --- 4/5. profiles ----------------------------------------------------
    alias_to_canonical = {}
    for pname, p in profiles.items():
        for a in p.get("aliases", []):
            if a in profiles:
                fail(f"profile `{pname}`: alias `{a}` collides with a canonical profile name")
            if a in alias_to_canonical:
                fail(f"alias `{a}` claimed by two profiles")
            alias_to_canonical[a] = pname
        base = p.get("extends")
        if base is not None and base not in profiles:
            fail(f"profile `{pname}`: extends unknown profile `{base}`")
        for f in p.get("features", []):
            if f not in features:
                fail(f"profile `{pname}`: unknown feature `{f}`")
        for gname, member in p.get("group_select", {}).items():
            if gname not in groups:
                fail(f"profile `{pname}`: group_select on unknown group `{gname}`")
            elif member not in groups[gname].get("members", []):
                fail(f"profile `{pname}`: group_select `{member}` not in group `{gname}`")

    def resolve(pname: str) -> set:
        p = profiles[pname]
        selected = set(p.get("features", []))
        if p.get("extends"):
            selected |= resolve(p["extends"])
        closed = set()
        for f in selected:
            closed |= closure(f, ())
        Emily = dict(p.get("group_select", {}))
        if p.get("extends"):
            base_sel = dict(profiles[p["extends"]].get("group_select", {}))
            base_sel.update(Emily)
            Emily = base_sel
        # group_select swaps the member: drop other selected members first
        for gname, member in Emily.items():
            for m in groups[gname].get("members", []):
                if m != member:
                    closed.discard(m)
            closed.add(member)
        return closed

    group_of = {}
    for gname, g in groups.items():
        for m in g.get("members", []):
            group_of.setdefault(m, []).append(gname)

    for pname in profiles:
        resolved = resolve(pname)
        per_group: dict[str, list[str]] = {}
        for f in resolved:
            for gname in group_of.get(f, []):
                per_group.setdefault(gname, []).append(f)
        for gname, members in per_group.items():
            explicit = [m for m in members if not features[m].get("unconditional")]
            if len(explicit) > 1:
                fail(f"profile `{pname}`: group `{gname}` has two members: {explicit}")
            elif len(members) > 1:
                warn(f"profile `{pname}`: group `{gname}` overlaps core default {members} (S6 hardens)")
        readiness = {}
        for f in resolved:
            readiness[features[f]["status"]] = readiness.get(features[f]["status"], 0) + 1
        print(f"profile {pname}: {len(resolved)} resolved {readiness}")

    # --- 7. golden vector cross-check --------------------------------------
    example = load_toml(HERE / "examples" / "basement-store-node.toml")
    base_profile = example.get("profile")
    if base_profile in alias_to_canonical:
        base_profile = alias_to_canonical[base_profile]
    expected = resolve(base_profile) | set(example.get("features", []))
    closed_expected = set()
    for f in expected:
        closed_expected |= closure(f, ())
    golden = json.loads((HERE / "golden" / "inputs.json").read_text())
    golden_names = {f["name"] for f in golden["features"]}
    if golden_names != closed_expected:
        fail(f"golden inputs features {sorted(golden_names)} != resolved {sorted(closed_expected)}")
    if golden.get("profile") != base_profile:
        fail(f"golden profile `{golden.get('profile')}` != canonical `{base_profile}`")
    for gf in golden["features"]:
        # compare crate parts of forwards (dep: entries are cargo-level, not versioned here)
        want_crates = set()
        for fwd in features[gf["name"]].get("forwards", []):
            if fwd.startswith("dep:"):
                continue
            crate = fwd.split("/")[0]
            ver = load_toml(ID / "crates" / crate / "Cargo.toml")["package"]["version"]
            want_crates.add(f"{crate}@{ver}")
        if set(gf["crates"]) != want_crates:
            fail(f"golden `{gf['name']}` crates {gf['crates']} != tree {sorted(want_crates)}")

    for w in warnings:
        print(f"WARNING: {w}")
    if errors:
        print(f"\n{len(errors)} ERROR(S):")
        for e in errors:
            print(f"  - {e}")
        return 1
    print("\ncatalogue validates against today's tree.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
