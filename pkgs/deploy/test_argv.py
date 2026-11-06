#!/usr/bin/env python3
"""argv behaviour of deploy.py main() (issue #6376).

Oracle: exit codes and stderr text of main() for each documented
invocation shape. Stdlib-only, like test_dupkeys_property.py.

Usage: test_argv.py [path-to-deploy.py]
"""

import contextlib
import importlib.util
import io
import json
import os
import sys
import tempfile


def load(path):
    spec = importlib.util.spec_from_file_location("deploy_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(mod, argv, env=None):
    out, err = io.StringIO(), io.StringIO()
    old = os.environ.pop("DEPLOY_REGISTRY", None)
    if env:
        os.environ["DEPLOY_REGISTRY"] = env
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = mod.main(argv)
    finally:
        os.environ.pop("DEPLOY_REGISTRY", None)
        if old is not None:
            os.environ["DEPLOY_REGISTRY"] = old
    return code, out.getvalue(), err.getvalue()


def definition():
    return {
        "package": "/nix/store/x-pkg",
        "entryPoint": "bin/x",
        "args": [],
        "environment": {},
        "secrets": [],
        "user": "svc",
        "ports": [8080],
        "restart": "always",
        "healthCheck": None,
        "targets": ["laptop"],
        "class": "systemd",
    }


def main(argv):
    path = argv[0] if argv else os.path.join(os.path.dirname(__file__), "deploy.py")
    mod = load(path)
    with tempfile.TemporaryDirectory() as tmp:
        regs = {}
        for name in ("A", "B"):
            regs[name] = os.path.join(tmp, f"reg{name}.json")
            with open(regs[name], "w") as fh:
                json.dump({f"svc-{name.lower()}": definition()}, fh)
        a, b = regs["A"], regs["B"]

        def check(label, argv, code, out_has=None, err_has=None, env=None):
            got, out, err = run(mod, argv, env)
            assert got == code, f"{label}: exit {got} != {code}\n{out}{err}"
            if out_has:
                assert out_has in out, f"{label}: stdout lacks {out_has!r}: {out}"
            if err_has:
                assert err_has in err, f"{label}: stderr lacks {err_has!r}: {err}"

        check("plain list", ["--registry", a, "list"], 0, out_has="svc-a")
        check("registry after subcommand", ["list", "--registry", a], 0, out_has="svc-a")
        check("registry= form", ["--registry=" + a, "list"], 0, out_has="svc-a")
        check("env fallback", ["list"], 0, out_has="svc-b", env=b)
        check("flag overrides env", ["--registry", a, "list"], 0, out_has="svc-a", env=b)
        check("unknown subcommand", ["--registry", a, "bogus"], 2, err_has="'bogus'")
        check("no args", [], 2)
        check("trailing junk", ["--registry", a, "list", "extra-junk"], 2, err_has="'extra-junk'")
        check("typo flag", ["--registry", a, "list", "--dry-rnu"], 2, err_has="'--dry-rnu'")
        check("duplicate registry", ["--registry", a, "--registry", b, "list"], 2, err_has="more than once")
        check("duplicate registry reversed", ["--registry", b, "list", "--registry", a], 2, err_has="more than once")
        check("duplicate mixed forms", ["--registry=" + a, "--registry", b, "list"], 2, err_has="more than once")
        check("registry missing path", ["list", "--registry"], 2, err_has="needs a path")
        check("registry= empty", ["--registry=", "list"], 2, err_has="needs a path")
    print("deploy argv checks: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
