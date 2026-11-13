#!/usr/bin/env python3
"""Property-based coverage for the duplicate-key invariant (issue #6384).

Invariant: registry text that repeats a key in any object, at any
nesting level, must never load — `load_registry` raises a ValueError
naming the duplicated key. Registry text with no repeated key must
load to exactly what the standard parser produces.

Oracle: for duplicate-free text, Python's stdlib `json.loads` (the
reference parser this module wraps). For duplicate-carrying text, the
invariant itself decides: any successful load is a counterexample,
whatever value it returns, because one of the two written values was
silently discarded.

The generator builds random object trees as pair lists (unique keys
per object, depth <= 3), renders them to text, and injects a duplicate
pair into a randomly chosen object node at a random depth. Stdlib-only
(no hypothesis in this repo's Python tooling); a fixed default seed
keeps CI deterministic, and a failing case prints its seed and text.

Usage: test_dupkeys_property.py [path-to-deploy.py] [--seed N] [--cases N]
"""

import importlib.util
import json
import os
import random
import sys
import tempfile

KEY_POOL = ["svc", "ports", "environment", "A", "B", "class", "targets", "x", "y"]


# A JVal is ("obj", [(key, JVal), ...]) | ("arr", [JVal, ...]) | ("leaf", text).
def gen_tree(rng, depth):
    kind = rng.choice(
        ["str", "int", "list", "obj"] if depth < 3 else ["str", "int", "list"]
    )
    if kind == "str":
        return ("leaf", json.dumps(rng.choice(["v", "systemd", "", "1", "notalist"])))
    if kind == "int":
        return ("leaf", str(rng.randint(0, 70000)))
    if kind == "list":
        return ("arr", [gen_tree(rng, depth + 1) for _ in range(rng.randint(0, 2))])
    keys = rng.sample(KEY_POOL, rng.randint(1, 4))
    return ("obj", [(k, gen_tree(rng, depth + 1)) for k in keys])


def gen_root(rng):
    keys = rng.sample(KEY_POOL, rng.randint(1, 4))
    return ("obj", [(k, gen_tree(rng, 1)) for k in keys])


def render(node):
    kind, val = node
    if kind == "leaf":
        return val
    if kind == "arr":
        return "[" + ", ".join(render(v) for v in val) + "]"
    return "{" + ", ".join(f"{json.dumps(k)}: {render(v)}" for k, v in val) + "}"


def object_nodes(node, out):
    kind, val = node
    if kind == "obj":
        out.append(val)
        for _, v in val:
            object_nodes(v, out)
    elif kind == "arr":
        for v in val:
            object_nodes(v, out)
    return out


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    seed, cases = 6384, 300
    for i, a in enumerate(argv):
        if a == "--seed":
            seed = int(argv[i + 1])
        if a == "--cases":
            cases = int(argv[i + 1])
    deploy_path = (
        args[0]
        if args
        else os.path.join(os.path.dirname(os.path.abspath(__file__)), "deploy.py")
    )
    spec = importlib.util.spec_from_file_location("deploy", deploy_path)
    deploy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(deploy)

    rng = random.Random(seed)
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "reg.json")

        def load(text):
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(text)
            return deploy.load_registry(path)

        for case in range(cases):
            tree = gen_root(rng)
            text = render(tree)
            # Duplicate-free: must match the stdlib oracle exactly.
            if load(text) != json.loads(text):
                print(f"counterexample (clean load) seed={seed} case={case}: {text}")
                return 1
            # Inject a duplicate pair into a random object node.
            nodes = object_nodes(tree, [])
            pairs = rng.choice(nodes)
            key = rng.choice([k for k, _ in pairs])
            pairs.insert(rng.randint(0, len(pairs)), (key, ("leaf", json.dumps("dup"))))
            dup_text = render(tree)
            try:
                load(dup_text)
            except ValueError as exc:
                if f"duplicate key {key!r}" not in str(exc):
                    print(
                        f"counterexample (unnamed key) seed={seed} case={case}: {dup_text}\nerror={exc}"
                    )
                    return 1
            else:
                print(
                    f"counterexample (duplicate accepted) seed={seed} case={case}: {dup_text}"
                )
                return 1
    print(
        f"property holds: {cases} cases (seed {seed}); clean loads match json.loads, duplicates at any depth rejected naming the key"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
