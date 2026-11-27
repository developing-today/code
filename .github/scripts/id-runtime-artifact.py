#!/usr/bin/env python3
"""Package the id-lib Nix closure for artifact-only board-game participants.

Usage: id-runtime-artifact.py --output-dir DIR --source-sha FULL_SHA
Strict contract: exactly one of each option, no positionals or abbreviations;
parse and validate everything before subprocesses or writes. Existing output
is never overwritten. Nix builds only on the producer, never on consumers.
"""

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def capture(*command):
    return subprocess.run(
        command, cwd=ROOT, check=True, text=True, stdout=subprocess.PIPE
    ).stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--output-dir", required=True, action="append")
    parser.add_argument("--source-sha", required=True, action="append")
    args = parser.parse_args()
    if len(args.output_dir) != 1 or len(args.source_sha) != 1:
        parser.error("each option must appear exactly once")
    sha = args.source_sha[0]
    if re.fullmatch(r"[0-9a-f]{40}", sha) is None:
        parser.error("--source-sha must be a full lowercase 40-character commit SHA")
    if not args.output_dir[0]:
        parser.error("--output-dir must not be empty")
    output = Path(args.output_dir[0]).absolute()
    if output.exists():
        parser.error(f"--output-dir already exists: {output}")
    if capture("git", "rev-parse", "HEAD") != sha:
        raise RuntimeError("source SHA differs from the producer checkout")
    store = capture(
        "nix",
        "build",
        "--no-link",
        "--print-out-paths",
        str(ROOT / "pkgs/id") + "#id-lib",
    )
    if "\n" in store or not Path(store).is_absolute():
        raise RuntimeError("expected exactly one absolute package output")
    binary = str(Path(store) / "bin/id")
    for command in (("serve", "--help"), ("world", "join", "--help")):
        capture(binary, *command)
    closure = capture("nix-store", "--query", "--requisites", store).splitlines()
    if store not in closure:
        raise RuntimeError("runtime closure does not contain its package root")
    # Stage privately; failed export/probes never publish a partial artifact.
    with tempfile.TemporaryDirectory(prefix="id-runtime-") as temporary:
        staged = Path(temporary)
        raw = staged / "runtime.closure"
        with raw.open("wb") as stream:
            subprocess.run(
                ["nix-store", "--export", *closure], check=True, stdout=stream
            )
        if raw.stat().st_size == 0:
            raise RuntimeError("empty runtime closure export")
        with (
            raw.open("rb") as source,
            (staged / "runtime.closure.gz").open("wb") as target,
        ):
            with gzip.GzipFile(fileobj=target, mode="wb", mtime=0) as compressed:
                shutil.copyfileobj(source, compressed)
        raw.unlink()
        (staged / "manifest.json").write_text(
            json.dumps(
                {
                    "source_sha": sha,
                    "store_path": store,
                    "package": "id-lib",
                    "system": "x86_64-linux",
                    "closure_paths": closure,
                },
                indent=2,
            )
            + "\n"
        )
        (staged / "INSTALL.txt").write_text(
            "Requires x86_64 Linux with Nix; run from the downloaded artifact directory.\n"
            "No source checkout, compiler or profile changes are needed.\n\n"
            "sha256sum --check SHA256SUMS\n"
            "gzip -dc runtime.closure.gz | nix-store --import\n"
            f"{binary} serve --help\n{binary} world join --help\n"
            "\nUse this exact executable with the separately downloaded game wasm.\n"
        )
        names = sorted(path.name for path in staged.iterdir())
        (staged / "SHA256SUMS").write_text(
            "".join(
                f"{hashlib.file_digest((staged / name).open('rb'), 'sha256').hexdigest()}  {name}\n"
                for name in names
            )
        )
        # Copy to a sibling staging directory, then atomically publish it.
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix=".id-runtime-", dir=output.parent
        ) as sibling:
            package = Path(sibling) / "artifact"
            shutil.copytree(staged, package)
            if output.exists():
                raise RuntimeError(
                    "output appeared while packaging; refusing overwrite"
                )
            package.rename(output)
    print(f"Packaged {store} at {sha}: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
