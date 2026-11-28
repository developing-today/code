# Tip-hygiene census (code#3205)

One quarterly-census pass for one repo is one command:

```bash
python3 scripts/fleet/tip-hygiene/check_tip.py --repo-dir <checkout> --ref origin/main
```

It prints a JSON census of the ref's tip tree: tracked-blob count,
total tip bytes, per-class counts/bytes for the junk classes named in
code#3205 (build outputs, binaries, logs, scratch/dump dirs, committed
databases — exact definitions in the script header), the junk totals,
and the largest blobs at or above `--large-blob-mib` (default 10 MiB).

The census is observe-only: it writes nothing and exits 0 whenever it
ran. Junk at the tip is a finding for the #3205 quarterly pass (file or
update the per-repo hygiene note; never rewrite history under that
task), not a failure of this tool.

Offline mode for tests and fixtures:

```bash
python3 scripts/fleet/tip-hygiene/check_tip.py --fixture entries.json
```

where `entries.json` is `{"entries": [{"path": ..., "size": ...}]}` —
the same shape `git ls-tree -r -l` produces, one object per blob.

Tests: `python3 -m unittest test_check_tip -v` in this directory. The
oracle is a hand-computed classification of the fixture entries, plus
a partition invariant (classes sum to the tip totals) over generated
entry lists.
