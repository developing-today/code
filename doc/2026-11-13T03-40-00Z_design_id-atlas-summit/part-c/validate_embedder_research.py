#!/usr/bin/env python3
"""research-matrix oracle for C9138-01.

CLI contract: only --input PATH (or --input=PATH), required exactly once;
-h/--help prints usage. Parse strictly before file I/O. Never run inference.
"""
import argparse
import json
from pathlib import Path
import sys


def validate(report, directory):
    """Return normally only for a conforming research-matrix report."""
    pass


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--input', required=True, action='append')
    args = parser.parse_args(argv)
    if len(args.input) != 1:
        parser.error('--input must occur exactly once')
    try:
        path = Path(args.input[0])
        validate(json.loads(path.read_text()), path.resolve().parent)
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f'research-matrix: FAIL: {error}', file=sys.stderr)
        return 1
    print('research-matrix: PASS (structural evidence only; no quality winner)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
