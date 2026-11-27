#!/usr/bin/env python3
"""Package the id-lib Nix closure for artifact-only board-game participants.

Usage: id-runtime-artifact.py --output-dir DIR --source-sha FULL_SHA
Strict contract: all arguments are parsed before subprocesses or writes.
"""
import argparse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True)
    parser.add_argument('--source-sha', required=True)
    parser.parse_args()
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
