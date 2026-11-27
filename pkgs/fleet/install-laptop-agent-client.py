#!/usr/bin/env python3
"""Install the guarded client pair without breaking concurrent legacy callers.

CLI contract (#3352): install-laptop-agent-client.py [--dest DIR]
Strict parsing precedes any write. The adjacent source files are required.
Publish the dependency first, then atomically replace the entrypoint. A failed
install may leave the old entrypoint and new (backward-compatible) legacy file;
it never publishes an entrypoint with a missing dependency. Keep the legacy
filename/API stable until broker import callers have migrated.
"""

import argparse
import os
from pathlib import Path
import tempfile


def install(dest, source=Path(__file__).resolve().parent):
    names = ("laptop-agent-legacy.py", "laptop-agent.py")
    # Check/read the complete pair before touching the destination.
    payloads = [(name, (source / name).read_bytes()) for name in names]
    dest.mkdir(parents=True, exist_ok=True)
    for name, payload in payloads:
        fd, tmp = tempfile.mkstemp(prefix=f".{name}.", dir=dest)
        try:
            with os.fdopen(fd, "wb") as out:
                out.write(payload)
                out.flush()
                os.fchmod(out.fileno(), 0o755)
                os.fsync(out.fileno())
            os.replace(tmp, dest / name)
            directory = os.open(dest, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)


def main():
    ap = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    ap.add_argument("--dest", type=Path, default=Path.home() / "tools")
    args = ap.parse_args()
    install(args.dest)


if __name__ == "__main__":
    main()
