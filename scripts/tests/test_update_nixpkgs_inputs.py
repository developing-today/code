#!/usr/bin/env python3
"""Contract tests for update-nixpkgs-inputs (both copies) — #2783, #603.

The repo carries two copies of this tool: the fleet copy at scripts/
and the pkgs/id copy at pkgs/id/scripts/, which the pkgs/id just
recipes run. #598 fixed a silent-failure bug (failed updates exited 0;
the wrapper re-ran the whole update after a genuine failure) in the
root copy only, and the fix silently never reached pkgs/id — the
drift this file exists to prevent:

- The two .py copies must stay byte-identical, and so must the two
  .sh wrappers. A fix that lands in one copy and not the other fails
  here instead of shipping silently.
- The #598 exit-code contract is pinned for BOTH .py copies: exit 1
  after the summaries when any input update failed, 0 when all
  succeed / are already up-to-date.
- The #6941 unmatched-ref contract is pinned for BOTH .py copies: a
  requested ref matching no inputs exits 1 (typo-only, --help, and
  mixed valid+unmatched runs), the unmatched refs stay named in the
  output, and the no-argument zero-input run also exits 1.
- The wrapper contract is pinned for BOTH .sh copies: fall back to
  python3 only when the script cannot be executed at all (126/127),
  propagate the exit status, and never re-run after a genuine failure
  (the update body executes exactly once).

- The #4553 cache-gate contract is pinned for BOTH .py copies: inputs
  with original.ref "nixos-unstable" pin to the newest Hydra-fully-built
  revision (never branch HEAD), only their `locked` entry is rewritten,
  an already-pinned input is a no-op, and a Hydra failure exits non-zero
  with flake.lock untouched.

Run: python3 -m unittest discover -s scripts/tests -p 'test_update_nixpkgs_inputs.py'
"""

import json
import os
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASH = shutil.which("bash") or "bash"
COPIES = {
    "root": ROOT / "scripts",
    "pkgs/id": ROOT / "pkgs" / "id" / "scripts",
}

FLAKE_LOCK = {
    "nodes": {
        "root": {"inputs": {"nixpkgs": "nixpkgs"}},
        "nixpkgs": {
            "original": {"type": "github", "owner": "NixOS", "repo": "nixpkgs"},
            "locked": {
                "type": "github",
                "owner": "NixOS",
                "repo": "nixpkgs",
                "rev": "0123456789abcdef0123456789abcdef01234567",
            },
        },
    },
    "root": "root",
    "version": 7,
}


def _write_executable(path: Path, content: str) -> None:
    path.write_text(content)
    path.chmod(0o755)


class TestCopiesCannotDrift(unittest.TestCase):
    def test_py_copies_byte_identical(self):
        root = (COPIES["root"] / "update-nixpkgs-inputs.py").read_bytes()
        pkgs_id = (COPIES["pkgs/id"] / "update-nixpkgs-inputs.py").read_bytes()
        self.assertEqual(
            root,
            pkgs_id,
            "scripts/ and pkgs/id/scripts/ update-nixpkgs-inputs.py diverged — "
            "port the change to both copies (see #2783)",
        )

    def test_sh_copies_byte_identical(self):
        root = (COPIES["root"] / "update-nixpkgs-inputs.sh").read_bytes()
        pkgs_id = (COPIES["pkgs/id"] / "update-nixpkgs-inputs.sh").read_bytes()
        self.assertEqual(
            root,
            pkgs_id,
            "scripts/ and pkgs/id/scripts/ update-nixpkgs-inputs.sh diverged — "
            "port the change to both copies (see #2783)",
        )


class TestDiscoverInputs(unittest.TestCase):
    def _discover(self, script_dir: Path, lock_data, ref=None):
        script = script_dir / "update-nixpkgs-inputs.py"
        return runpy.run_path(str(script))["discover_inputs"](lock_data, ref)

    def _lock_for_original(self, original):
        return {
            "nodes": {
                "root": {"inputs": {"candidate": "candidate"}},
                "candidate": {"original": original},
            }
        }

    def test_owner_matches_exactly_ignoring_case_in_both_copies(self):
        accepted = ("NixOS", "nixos", "Nixos", "NIXOS", "NiXoS", "nIxOs")
        rejected = ("NixOS2", "nixos-fork", "NixOSx", "nixoss", "evilNixOS")
        non_strings = (None, 7, {}, [])

        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                for owner in accepted:
                    with self.subTest(owner=owner):
                        original = {"type": "github", "owner": owner, "repo": "nixpkgs"}
                        self.assertEqual(
                            self._discover(
                                script_dir, self._lock_for_original(original)
                            ),
                            ["candidate"],
                        )
                for owner in (*rejected, *non_strings):
                    with self.subTest(owner=owner):
                        original = {"type": "github", "owner": owner, "repo": "nixpkgs"}
                        self.assertEqual(
                            self._discover(
                                script_dir, self._lock_for_original(original)
                            ),
                            [],
                        )

    def test_repository_type_follows_and_ref_filters_are_preserved(self):
        inputs = {
            "default": "default",
            "master": "master",
            "unstable": "unstable",
            "stable": "stable",
            "wrong-repo": "wrong-repo",
            "wrong-type": "wrong-type",
            "follows": ["master"],
        }
        nodes = {
            "root": {"inputs": inputs},
            "default": {
                "original": {"type": "github", "owner": "NixOS", "repo": "nixpkgs"}
            },
            "master": {
                "original": {
                    "type": "github",
                    "owner": "NixOS",
                    "repo": "nixpkgs",
                    "ref": "master",
                }
            },
            "unstable": {
                "original": {
                    "type": "github",
                    "owner": "NixOS",
                    "repo": "nixpkgs",
                    "ref": "nixos-unstable",
                }
            },
            "stable": {
                "original": {
                    "type": "github",
                    "owner": "NixOS",
                    "repo": "nixpkgs",
                    "ref": "nixos-24.11",
                }
            },
            "wrong-repo": {
                "original": {"type": "github", "owner": "NixOS", "repo": "other"}
            },
            "wrong-type": {
                "original": {"type": "git", "owner": "NixOS", "repo": "nixpkgs"}
            },
        }
        lock_data = {"nodes": nodes}
        expected = ["default", "master", "stable", "unstable"]

        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                discover = runpy.run_path(str(script_dir / "update-nixpkgs-inputs.py"))[
                    "discover_inputs"
                ]
                self.assertEqual(discover(lock_data, None), expected)
                self.assertEqual(discover(lock_data, "master"), ["default", "master"])
                self.assertEqual(discover(lock_data, "nixos-unstable"), ["unstable"])
                self.assertEqual(discover(lock_data, "nixos-24.11"), ["stable"])

    def test_current_flake_locks_expose_only_their_direct_nixpkgs_inputs(self):
        # The root flake intentionally dropped the master/stable inputs (#3327);
        # nixpkgs-unstable follows fleet/nixpkgs and is not independently updated.
        # The id sub-flake has its own direct nixpkgs input. Keep both lock graphs
        # explicit so a valid change to one cannot make this test lie about the other.
        flake_locks = {
            "root": (ROOT / "flake.lock", ["nixpkgs-25"]),
            "pkgs/id": (ROOT / "pkgs" / "id" / "flake.lock", ["nixpkgs"]),
        }

        for copy_label, script_dir in COPIES.items():
            discover = runpy.run_path(str(script_dir / "update-nixpkgs-inputs.py"))[
                "discover_inputs"
            ]
            for flake_label, (lock_path, expected) in flake_locks.items():
                with self.subTest(copy=copy_label, flake=flake_label):
                    lock_data = json.loads(lock_path.read_text())
                    self.assertEqual(discover(lock_data, None), expected)


class ScriptTree:
    """One copy of the tool staged as <tmp>/scripts/ + <tmp>/flake.lock,

    with a stub `nix` (and a stub `uv`, so the .py shebang executes the
    body under python3) on PATH. The stub nix appends one line to a
    counter file per invocation and exits with the configured status.
    With `interpreter_missing=True` there is no uv stub and no uv on
    PATH, so direct execution fails 127, and a python3 shim pointing at
    the running interpreter serves the wrapper's fallback.
    """

    def __init__(
        self,
        src_dir: Path,
        nix_status: int,
        interpreter_missing: bool = False,
        lock_data=None,
    ):
        self._tmp = tempfile.TemporaryDirectory()
        base = Path(self._tmp.name)
        self.repo_dir = base / "repo"
        scripts = self.repo_dir / "scripts"
        scripts.mkdir(parents=True)
        shutil.copy2(src_dir / "update-nixpkgs-inputs.py", scripts)
        shutil.copy2(src_dir / "update-nixpkgs-inputs.sh", scripts)
        (self.repo_dir / "flake.lock").write_text(
            json.dumps(FLAKE_LOCK if lock_data is None else lock_data)
        )
        self.bin = base / "bin"
        self.bin.mkdir()
        self.counter = base / "nix-calls"
        _write_executable(
            self.bin / "nix",
            f'#!/bin/sh\necho call >> "{self.counter}"\nexit {nix_status}\n',
        )
        if interpreter_missing:
            # No uv stub and a PATH with no uv on it: the .py shebang
            # (`env -S uv run --script`) cannot find its interpreter, so
            # the wrapper's direct-execution attempt exits 127. The
            # python3 shim keeps the wrapper's fallback interpreter
            # hermetic — a host python3 may live outside /usr/bin:/bin
            # (e.g. on NixOS) and must not leak into the 127 setup.
            # The shell wrapper also needs dirname, absent from /bin and
            # /usr/bin on NixOS; expose only that executable in the test PATH.
            dirname = shutil.which("dirname")
            if dirname:
                (self.bin / "dirname").symlink_to(dirname)
            _write_executable(
                self.bin / "python3",
                f'#!/bin/sh\nexec {sys.executable} "$@"\n',
            )
            self.env = dict(os.environ, PATH=f"{self.bin}:/usr/bin:/bin")
            return
        # The .py shebang is `env -S uv run --script`; this stub makes
        # the wrapper's direct-execution attempt run the body under
        # python3, so wrapper behaviour is testable without uv.
        _write_executable(
            self.bin / "uv",
            "#!/bin/sh\n"
            '[ "$1" = run ] && shift\n'
            '[ "$1" = --script ] && shift\n'
            'exec python3 "$@"\n',
        )
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}")

    def cleanup(self):
        self._tmp.cleanup()

    def nix_calls(self) -> int:
        if not self.counter.exists():
            return 0
        return len(self.counter.read_text().splitlines())

    def run_py(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable,
                str(self.repo_dir / "scripts" / "update-nixpkgs-inputs.py"),
                *args,
            ],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def run_sh(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            [BASH, str(self.repo_dir / "scripts" / "update-nixpkgs-inputs.sh")],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )


class TestExitCodeContract(unittest.TestCase):
    def _check_copy(self, label: str, src_dir: Path):
        with self.subTest(copy=label, case="failed update exits 1"):
            tree = ScriptTree(src_dir, nix_status=1)
            try:
                r = tree.run_py()
                self.assertIn("Failed:  1", r.stdout)
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="up-to-date exits 0"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_py()
                self.assertIn("Failed:  0", r.stdout)
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            finally:
                tree.cleanup()

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


class TestUnmatchedRefExitCodes(unittest.TestCase):
    """#6941: a requested ref matching no inputs must exit non-zero."""

    ZERO_INPUT_LOCK = {
        "nodes": {"root": {"inputs": {}}},
        "root": "root",
        "version": 7,
    }

    def _check_copy(self, label: str, src_dir: Path):
        with self.subTest(copy=label, case="typo-only ref exits 1, names the ref"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_py("nixos-unstabl")
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertIn("No inputs found for:", r.stdout)
                self.assertIn("nixos-unstabl", r.stdout)
                self.assertEqual(tree.nix_calls(), 0)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="--help is an unmatched ref, exits 1"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_py("--help")
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertIn("No inputs found for:", r.stdout)
                self.assertIn("--help", r.stdout)
                self.assertEqual(tree.nix_calls(), 0)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="mixed valid + unmatched exits 1"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_py("master", "nixos-unstabl")
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertIn("No inputs found for:", r.stdout)
                self.assertIn("nixos-unstabl", r.stdout)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="no-arg zero-input run exits 1"):
            tree = ScriptTree(src_dir, nix_status=0, lock_data=self.ZERO_INPUT_LOCK)
            try:
                r = tree.run_py()
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertIn("No inputs found for:", r.stdout)
            finally:
                tree.cleanup()

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


class TestWrapperContract(unittest.TestCase):
    def _check_copy(self, label: str, src_dir: Path):
        with self.subTest(copy=label, case="genuine failure runs once, exits 1"):
            tree = ScriptTree(src_dir, nix_status=1)
            try:
                r = tree.run_sh()
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(
                    tree.nix_calls(),
                    1,
                    "wrapper re-ran the update after a genuine failure",
                )
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="success exits 0"):
            tree = ScriptTree(src_dir, nix_status=0)
            try:
                r = tree.run_sh()
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()
        with self.subTest(copy=label, case="cannot-execute falls back to python3"):
            tree = ScriptTree(src_dir, nix_status=1)
            try:
                # No exec bit: direct execution fails 126, the wrapper
                # must fall back to python3 exactly once and still
                # propagate the genuine failure status.
                (tree.repo_dir / "scripts" / "update-nixpkgs-inputs.py").chmod(0o644)
                r = tree.run_sh()
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()
        with self.subTest(
            copy=label, case="missing interpreter (127) falls back to python3"
        ):
            tree = ScriptTree(src_dir, nix_status=1, interpreter_missing=True)
            try:
                # uv absent: the shebang interpreter cannot be found, so
                # direct execution fails 127 — the other cannot-execute
                # exit. Same contract as 126: fall back to python3
                # exactly once and propagate the genuine failure status.
                r = tree.run_sh()
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertEqual(tree.nix_calls(), 1)
            finally:
                tree.cleanup()

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


import contextlib
import http.server
import threading

# --- #4553 cache gate ---------------------------------------------------

GATE_OLD_REV = "1" * 40
GATE_TARGET_REV = "2" * 40
GATE_OTHER_REV = "3" * 40
GATE_NAR_HASH = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
GATE_EVAL_ID = 424242

GATE_FLAKE_LOCK = {
    "nodes": {
        "root": {
            "inputs": {"nixpkgs": "nixpkgs", "nixpkgs-unstable": "nixpkgs-unstable"}
        },
        "nixpkgs": {
            "original": {"type": "github", "owner": "NixOS", "repo": "nixpkgs"},
            "locked": {
                "type": "github",
                "owner": "NixOS",
                "repo": "nixpkgs",
                "rev": GATE_OTHER_REV,
                "narHash": "sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=",
                "lastModified": 1700000000,
            },
        },
        "nixpkgs-unstable": {
            "original": {
                "type": "github",
                "owner": "NixOS",
                "repo": "nixpkgs",
                "ref": "nixos-unstable",
            },
            "locked": {
                "type": "github",
                "owner": "NixOS",
                "repo": "nixpkgs",
                "rev": GATE_OLD_REV,
                "narHash": "sha256-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC=",
                "lastModified": 1700000001,
            },
        },
    },
    "root": "root",
    "version": 7,
}


class _HydraHandler(http.server.BaseHTTPRequestHandler):
    payloads = {}

    def do_GET(self):
        body = self.payloads.get(self.path)
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        data = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


@contextlib.contextmanager
def _hydra_server(payloads=None):
    if payloads is None:
        payloads = {
            "/job/nixos/unstable/tested/latest-finished": {
                "buildstatus": 0,
                "jobsetevals": [GATE_EVAL_ID],
            },
            f"/eval/{GATE_EVAL_ID}": {
                "jobsetevalinputs": {"nixpkgs": {"revision": GATE_TARGET_REV}}
            },
        }
    handler = type("Handler", (_HydraHandler,), {"payloads": payloads})
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()


class TestHydraGateParsing(unittest.TestCase):
    """Black-box: hydra_cached_revision runs against a local HTTP server
    (via its base-URL parameter), so the real fetch + parse + fail-closed
    behaviour is exercised without the network."""

    GOOD_PAYLOADS = {
        "/job/nixos/unstable/tested/latest-finished": {
            "buildstatus": 0,
            "jobsetevals": [GATE_EVAL_ID],
        },
        f"/eval/{GATE_EVAL_ID}": {
            "jobsetevalinputs": {"nixpkgs": {"revision": GATE_TARGET_REV}}
        },
    }

    def _check_copy(self, label: str, script_dir: Path):
        mod = runpy.run_path(str(script_dir / "update-nixpkgs-inputs.py"))
        gate_error = mod["HydraGateError"]
        resolve = mod["hydra_cached_revision"]

        with self.subTest(copy=label, case="happy path resolves eval + revision"):
            with _hydra_server(self.GOOD_PAYLOADS) as base:
                rev, eval_id = resolve(base)
            self.assertEqual(rev, GATE_TARGET_REV)
            self.assertEqual(eval_id, GATE_EVAL_ID)

        with self.subTest(copy=label, case="unreachable hydra is a gate error"):
            with self.assertRaises(gate_error):
                resolve("http://127.0.0.1:9")

        with self.subTest(copy=label, case="bad latest-finished shapes fail closed"):
            for bad in (
                {"buildstatus": 1, "jobsetevals": [GATE_EVAL_ID]},
                {"buildstatus": 0, "jobsetevals": []},
                {"buildstatus": 0},
                ["not", "an", "object"],
            ):
                payloads = dict(self.GOOD_PAYLOADS)
                payloads["/job/nixos/unstable/tested/latest-finished"] = bad
                with _hydra_server(payloads) as base:
                    with self.assertRaises(gate_error):
                        resolve(base)

        with self.subTest(copy=label, case="bad eval shapes fail closed"):
            for bad_eval in (
                {"jobsetevalinputs": {"nixpkgs": {"revision": "not-a-rev"}}},
                {"jobsetevalinputs": {}},
                {},
            ):
                payloads = dict(self.GOOD_PAYLOADS)
                payloads[f"/eval/{GATE_EVAL_ID}"] = bad_eval
                with _hydra_server(payloads) as base:
                    with self.assertRaises(gate_error):
                        resolve(base)

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


class GateTree:
    """One copy of the tool staged with a two-input flake.lock (one
    no-ref input, one nixos-unstable input) and a stub `nix` whose
    `flake metadata` answers with a canned locked entry for
    GATE_TARGET_REV. Every nix invocation is counted."""

    def __init__(self, src_dir: Path):
        self._tmp = tempfile.TemporaryDirectory()
        base = Path(self._tmp.name)
        self.repo_dir = base / "repo"
        scripts = self.repo_dir / "scripts"
        scripts.mkdir(parents=True)
        shutil.copy2(src_dir / "update-nixpkgs-inputs.py", scripts)
        shutil.copy2(src_dir / "update-nixpkgs-inputs.sh", scripts)
        self.lock_path = self.repo_dir / "flake.lock"
        self.lock_path.write_text(json.dumps(GATE_FLAKE_LOCK, indent=2) + "\n")
        self.bin = base / "bin"
        self.bin.mkdir()
        self.counter = base / "nix-calls"
        metadata = json.dumps(
            {
                "locked": {
                    "__final": True,
                    "lastModified": 1793540411,
                    "narHash": GATE_NAR_HASH,
                    "owner": "NixOS",
                    "repo": "nixpkgs",
                    "rev": GATE_TARGET_REV,
                    "type": "github",
                }
            }
        )
        _write_executable(
            self.bin / "nix",
            "#!/bin/sh\n"
            f'echo call >> "{self.counter}"\n'
            'if [ "$1" = flake ] && [ "$2" = metadata ]; then\n'
            f"  printf '%s' '{metadata}'\n"
            "fi\n"
            "exit 0\n",
        )
        _write_executable(
            self.bin / "uv",
            "#!/bin/sh\n"
            '[ "$1" = run ] && shift\n'
            '[ "$1" = --script ] && shift\n'
            'exec python3 "$@"\n',
        )

    def cleanup(self):
        self._tmp.cleanup()

    def nix_calls(self) -> int:
        if not self.counter.exists():
            return 0
        return len(self.counter.read_text().splitlines())

    def run_py(self, hydra_url: str) -> subprocess.CompletedProcess:
        env = dict(
            os.environ,
            PATH=f"{self.bin}:{os.environ['PATH']}",
            UPDATE_NIXPKGS_HYDRA_URL=hydra_url,
        )
        return subprocess.run(
            [
                sys.executable,
                str(self.repo_dir / "scripts" / "update-nixpkgs-inputs.py"),
                "nixos-unstable",
            ],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )


class TestGatedPinEndToEnd(unittest.TestCase):
    def _check_copy(self, label: str, src_dir: Path):
        with self.subTest(copy=label, case="pin rewrites only the gated locked entry"):
            tree = GateTree(src_dir)
            try:
                with _hydra_server() as hydra_url:
                    r = tree.run_py(hydra_url)
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                self.assertIn("Hydra gate", r.stdout)
                self.assertIn("Failed:  0", r.stdout)
                lock = json.loads(tree.lock_path.read_text())
                unstable = lock["nodes"]["nixpkgs-unstable"]
                self.assertEqual(unstable["locked"]["rev"], GATE_TARGET_REV)
                self.assertEqual(unstable["locked"]["narHash"], GATE_NAR_HASH)
                self.assertEqual(unstable["locked"]["lastModified"], 1793540411)
                self.assertNotIn("__final", unstable["locked"])
                self.assertEqual(
                    unstable["original"],
                    GATE_FLAKE_LOCK["nodes"]["nixpkgs-unstable"]["original"],
                )
                self.assertEqual(
                    lock["nodes"]["nixpkgs"], GATE_FLAKE_LOCK["nodes"]["nixpkgs"]
                )
                # Exactly one nix call: the metadata fetch. The gated
                # input must never go through `nix flake update` (HEAD).
                self.assertEqual(tree.nix_calls(), 1)

                with self.subTest(case="second run is a byte-identical no-op"):
                    before = tree.lock_path.read_bytes()
                    with _hydra_server() as hydra_url:
                        r2 = tree.run_py(hydra_url)
                    self.assertEqual(r2.returncode, 0, r2.stdout + r2.stderr)
                    self.assertIn("already up-to-date", r2.stdout)
                    self.assertEqual(tree.lock_path.read_bytes(), before)
            finally:
                tree.cleanup()

        with self.subTest(
            copy=label, case="hydra failure exits non-zero, lock untouched"
        ):
            tree = GateTree(src_dir)
            try:
                before = tree.lock_path.read_bytes()
                # Port 9 (discard) is closed: the lookup fails fast.
                r = tree.run_py("http://127.0.0.1:9")
                self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
                self.assertIn("Hydra cache gate failed", r.stderr)
                self.assertEqual(tree.lock_path.read_bytes(), before)
                self.assertEqual(tree.nix_calls(), 0)
            finally:
                tree.cleanup()

    def test_root_copy(self):
        self._check_copy("root", COPIES["root"])

    def test_pkgs_id_copy(self):
        self._check_copy("pkgs/id", COPIES["pkgs/id"])
        self._check_copy("pkgs/id", COPIES["pkgs/id"])


class TestHttpsContext(unittest.TestCase):
    def test_verification_stays_on_in_both_copies(self):
        import ssl

        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                mod = runpy.run_path(str(script_dir / "update-nixpkgs-inputs.py"))
                ctx = mod["https_context"]()
                self.assertEqual(ctx.verify_mode, ssl.CERT_REQUIRED)
                self.assertTrue(ctx.check_hostname)


UNKNOWN = ("unknown", "unknown", "unknown")


class TestLockReadersWrongShapes(unittest.TestCase):
    """#6847: every flake.lock reader is total over wrong shapes.

    A non-object node, a null/non-object original/locked, a
    non-string type/ref/rev, and a missing node/root are treated the
    way discover_inputs has always treated a missing node — the input
    is skipped / reported unknown — never a raw AttributeError /
    TypeError / KeyError. The mid-run case matters most: locked.rev
    as a number passes discovery (which never reads it) and used to
    crash get_input_info inside the execution loop, after earlier
    inputs had already been updated.
    """

    def _module(self, script_dir: Path):
        return runpy.run_path(str(script_dir / "update-nixpkgs-inputs.py"))

    def _lock(self, node, root_inputs=None, nodes_extra=None):
        nodes = {"root": {"inputs": root_inputs or {"nixpkgs": "nixpkgs-node"}}}
        if node is not _MISSING:
            nodes["nixpkgs-node"] = node
        if nodes_extra:
            nodes.update(nodes_extra)
        return {"nodes": nodes, "root": "root", "version": 7}

    def _readers(self, mod):
        return (
            mod["discover_inputs"],
            mod["get_input_info"],
            mod["get_input_ref"],
            mod["get_locked_rev"],
        )

    def test_well_formed_lock_unchanged_in_both_copies(self):
        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                discover, info, ref, locked_rev = self._readers(self._module(script_dir))
                self.assertEqual(discover(FLAKE_LOCK, None), ["nixpkgs"])
                self.assertEqual(
                    info(FLAKE_LOCK, "nixpkgs"),
                    ("0123456789ab", "github:NixOS/nixpkgs", "(default)"),
                )
                self.assertIsNone(ref(FLAKE_LOCK, "nixpkgs"))
                self.assertEqual(
                    locked_rev(FLAKE_LOCK, "nixpkgs"),
                    "0123456789abcdef0123456789abcdef01234567",
                )

    def test_wrong_shape_nodes_are_skipped_in_both_copies(self):
        good_original = {"type": "github", "owner": "NixOS", "repo": "nixpkgs"}
        good_locked = {"type": "github", "rev": "a" * 40}
        # (node, discover, info, ref, locked_rev)
        cases = {
            "node is a string": ("nixpkgs-node", [], UNKNOWN, None, None),
            "node is null": (None, [], UNKNOWN, None, None),
            "node is a list": (["nixpkgs"], [], UNKNOWN, None, None),
            "node missing from nodes": (_MISSING, [], UNKNOWN, None, None),
            # original unusable: discovery cannot match, but the
            # locked entry still reports (the readers' original ->
            # locked fallback chains), with owner/repo unknown.
            "original is null": (
                {"original": None, "locked": good_locked},
                [],
                ("aaaaaaaaaaaa", "github:?/?", "(default)"),
                None,
                "a" * 40,
            ),
            "original is a string": (
                {"original": "github", "locked": good_locked},
                [],
                ("aaaaaaaaaaaa", "github:?/?", "(default)"),
                None,
                "a" * 40,
            ),
            "original.type is a number": (
                {"original": {**good_original, "type": 7}, "locked": good_locked},
                [],
                ("aaaaaaaaaaaa", "github:NixOS/nixpkgs", "(default)"),
                None,
                "a" * 40,
            ),
            # locked unusable: discovery matches on original; the rev
            # readers report unknown/None instead of crashing.
            "locked is null": (
                {"original": good_original, "locked": None},
                ["nixpkgs"],
                ("unknown", "github:NixOS/nixpkgs", "(default)"),
                None,
                None,
            ),
        }
        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                discover, info, ref, locked_rev = self._readers(self._module(script_dir))
                for name, (node, want_discover, want_info, want_ref, want_rev) in cases.items():
                    with self.subTest(shape=name):
                        lock = self._lock(node)
                        self.assertEqual(discover(lock, None), want_discover)
                        self.assertEqual(info(lock, "nixpkgs"), want_info)
                        self.assertEqual(ref(lock, "nixpkgs"), want_ref)
                        self.assertEqual(locked_rev(lock, "nixpkgs"), want_rev)

    def test_non_string_rev_never_crashes_the_execution_loop_readers(self):
        # The dangerous shape: discovery passes it (rev is not read
        # there), so get_input_info/get_locked_rev must not crash
        # when the execution loop reaches this input.
        node = {
            "original": {"type": "github", "owner": "NixOS", "repo": "nixpkgs"},
            "locked": {"type": "github", "rev": 123},
        }
        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                discover, info, ref, locked_rev = self._readers(self._module(script_dir))
                lock = self._lock(node)
                self.assertEqual(discover(lock, None), ["nixpkgs"])
                self.assertIsNone(locked_rev(lock, "nixpkgs"))
                rev, url, _ = info(lock, "nixpkgs")
                self.assertEqual(rev, "unknown")
                self.assertEqual(url, "github:NixOS/nixpkgs")

    def test_non_string_ref_is_unclassifiable_in_both_copies(self):
        node = {
            "original": {
                "type": "github",
                "owner": "NixOS",
                "repo": "nixpkgs",
                "ref": 7,
            },
            "locked": {"type": "github", "rev": "a" * 40},
        }
        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                discover, info, ref, locked_rev = self._readers(self._module(script_dir))
                lock = self._lock(node)
                self.assertEqual(discover(lock, None), [])
                self.assertEqual(discover(lock, "master"), [])
                self.assertIsNone(ref(lock, "nixpkgs"))

    def test_missing_root_and_missing_nodes_in_both_copies(self):
        locks = (
            {"nodes": {"nixpkgs-node": {"original": {}}}},
            {"root": "root", "version": 7},
            {"nodes": None},
            {"nodes": {"root": None}},
            {"nodes": {"root": {"inputs": None}}},
            ["not", "an", "object"],
        )
        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                discover, info, ref, locked_rev = self._readers(self._module(script_dir))
                for lock in locks:
                    self.assertEqual(discover(lock, None), [])
                    self.assertEqual(
                        info(lock, "nixpkgs"), ("unknown", "unknown", "unknown")
                    )
                    self.assertIsNone(ref(lock, "nixpkgs"))
                    self.assertIsNone(locked_rev(lock, "nixpkgs"))

    def test_pin_input_refuses_a_wrong_shape_node_in_both_copies(self):
        for label, script_dir in COPIES.items():
            with self.subTest(copy=label):
                mod = self._module(script_dir)
                tree = Path(tempfile.mkdtemp(prefix="uni-shape-pin-"))
                (tree / "flake.lock").write_text(
                    json.dumps(self._lock("nixpkgs-node"))
                )
                self.assertFalse(
                    mod["pin_input"]("nixpkgs", tree, {"rev": "a" * 40})
                )


class _Missing:
    pass


_MISSING = _Missing()


if __name__ == "__main__":
    unittest.main()
