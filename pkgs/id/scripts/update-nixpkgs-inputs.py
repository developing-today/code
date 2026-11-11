#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""update-nixpkgs-inputs.py

Discover NixOS/nixpkgs inputs from flake.lock and update them.
Parses flake.lock directly to find inputs by source URL (github:NixOS/nixpkgs).

Usage:
    ./scripts/update-nixpkgs-inputs.py [ref...]

Arguments:
    ref  - optional branch filter(s): "master", "nixos-unstable", or empty for all
           multiple refs can be passed to produce a combined summary

Examples:
    ./scripts/update-nixpkgs-inputs.py                         # all NixOS/nixpkgs inputs
    ./scripts/update-nixpkgs-inputs.py master                  # master (or no ref) inputs
    ./scripts/update-nixpkgs-inputs.py nixos-unstable           # nixos-unstable inputs
    ./scripts/update-nixpkgs-inputs.py master nixos-unstable    # both categories, combined summary

Cache gate (#4553): inputs whose original.ref is "nixos-unstable" are NOT
updated to branch HEAD — HEAD runs days ahead of what Hydra has built and
cache.nixos.org serves, so a HEAD pin can turn the next system build into
a multi-day source build (see #2715). Gated inputs are pinned instead to
the newest revision Hydra has fully built for the channel: the `tested`
job of jobset nixos:unstable, resolved via
    GET <hydra>/job/nixos/unstable/tested/latest-finished -> jobsetevals[0]
    GET <hydra>/eval/<id>                                 -> jobsetevalinputs.nixpkgs.revision
The pin rewrites only the input's `locked` entry (rev + narHash +
lastModified from a real `nix flake metadata` fetch of that revision);
`original` — including ref "nixos-unstable" — is left untouched so input
discovery keeps matching. The gate fails closed: if the Hydra lookups
fail or return an unexpected shape, the script exits non-zero before
touching flake.lock. The Hydra base URL can be overridden with the
UPDATE_NIXPKGS_HYDRA_URL environment variable (used by the tests).
"""

import json
import os
import re
import ssl
import subprocess
import sys
import urllib.request
from pathlib import Path

GATED_REF = "nixos-unstable"
HYDRA_BASE_URL_ENV = "UPDATE_NIXPKGS_HYDRA_URL"
DEFAULT_HYDRA_BASE_URL = "https://hydra.nixos.org"
HYDRA_LATEST_FINISHED_PATH = "/job/nixos/unstable/tested/latest-finished"
REV_RE = re.compile(r"^[0-9a-f]{40}$")


class HydraGateError(Exception):
    """The Hydra cache gate could not produce a trustworthy revision."""


def https_context():
    """SSL context for Hydra requests, with verification ON.

    The .py shebang runs under `uv run --script`, whose standalone
    Python builds can fail to locate a CA bundle at all (0 certs
    loaded, every HTTPS request CERTIFICATE_VERIFY_FAILED) even on a
    healthy system. When the default context comes up empty, point it
    at the usual system bundle locations instead of failing."""
    ctx = ssl.create_default_context()
    if ctx.get_ca_certs():
        return ctx
    for cafile in (
        os.environ.get("SSL_CERT_FILE"),
        "/etc/ssl/certs/ca-certificates.crt",  # Debian / NixOS
        "/etc/pki/tls/certs/ca-bundle.crt",  # RHEL family
        "/etc/ssl/cert.pem",  # macOS / Alpine
    ):
        if cafile and os.path.exists(cafile):
            return ssl.create_default_context(cafile=cafile)
    return ctx


def fetch_hydra_json(url):
    """GET a Hydra JSON endpoint (follows redirects; Hydra content-
    negotiates on the Accept header and serves HTML without it)."""
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=60, context=https_context()) as resp:
        return json.load(resp)


def parse_latest_finished_eval_id(data):
    """Extract the jobset eval id from a latest-finished build payload."""
    if not isinstance(data, dict):
        raise HydraGateError("latest-finished payload is not a JSON object")
    status = data.get("buildstatus")
    if status is not None and status != 0:
        raise HydraGateError(
            f"latest finished 'tested' build did not succeed (buildstatus={status})"
        )
    evals = data.get("jobsetevals")
    if not isinstance(evals, list) or not evals:
        raise HydraGateError("latest-finished payload has no jobsetevals")
    eval_id = evals[0]
    if isinstance(eval_id, str) and eval_id.isdigit():
        eval_id = int(eval_id)
    if not isinstance(eval_id, int) or isinstance(eval_id, bool):
        raise HydraGateError(f"unexpected jobsetevals[0]: {eval_id!r}")
    return eval_id


def parse_eval_revision(data):
    """Extract the nixpkgs revision from a Hydra eval payload."""
    if not isinstance(data, dict):
        raise HydraGateError("eval payload is not a JSON object")
    inputs = data.get("jobsetevalinputs")
    if not isinstance(inputs, dict):
        raise HydraGateError("eval payload has no jobsetevalinputs")
    nixpkgs = inputs.get("nixpkgs")
    if not isinstance(nixpkgs, dict):
        raise HydraGateError("eval payload has no nixpkgs jobset eval input")
    rev = nixpkgs.get("revision")
    if not isinstance(rev, str) or not REV_RE.match(rev):
        raise HydraGateError(f"unexpected nixpkgs revision: {rev!r}")
    return rev


def hydra_cached_revision(base_url=None):
    """Newest nixos-unstable revision Hydra has fully built.

    Returns (revision, eval_id). Raises HydraGateError on any lookup
    failure or unexpected response shape — callers must fail closed.
    """
    base = (
        base_url or os.environ.get(HYDRA_BASE_URL_ENV) or DEFAULT_HYDRA_BASE_URL
    ).rstrip("/")
    try:
        latest = fetch_hydra_json(base + HYDRA_LATEST_FINISHED_PATH)
    except Exception as e:
        raise HydraGateError(
            f"cannot fetch {base + HYDRA_LATEST_FINISHED_PATH}: {e}"
        ) from e
    eval_id = parse_latest_finished_eval_id(latest)
    try:
        eval_data = fetch_hydra_json(f"{base}/eval/{eval_id}")
    except Exception as e:
        raise HydraGateError(f"cannot fetch {base}/eval/{eval_id}: {e}") from e
    return parse_eval_revision(eval_data), eval_id


def fetch_locked_for_rev(rev):
    """Fetch github:NixOS/nixpkgs@<rev> with nix and return the locked
    attrs (rev, narHash, lastModified, ...) a flake.lock entry needs.
    Returns None if the fetch fails or the result does not check out."""
    result = subprocess.run(
        ["nix", "flake", "metadata", f"github:NixOS/nixpkgs/{rev}", "--json"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return None
    try:
        locked = json.loads(result.stdout)["locked"]
    except (ValueError, KeyError, TypeError):
        return None
    if not isinstance(locked, dict):
        return None
    # `nix flake metadata` annotates its result with __final; flake.lock
    # locked entries do not carry it.
    locked = {k: v for k, v in locked.items() if k != "__final"}
    if locked.get("rev") != rev or not locked.get("narHash"):
        return None
    return locked


def load_flake_lock(repo_dir):
    """Load and parse flake.lock."""
    with open(repo_dir / "flake.lock") as f:
        return json.load(f)


# --- flake.lock shape discipline (#6847) -----------------------------------
# flake.lock is normally nix-generated, but it is also hand-edited,
# hand-pinned (pin_input below writes it), merged, and occasionally
# truncated or conflicted — exactly the states this repair tool is
# re-run against. Every reader below is therefore a TOTAL function
# over any JSON shape: a non-object node, a null/non-object
# "original"/"locked", a non-string type/ref/rev, and a missing
# node/root are all treated the way discover_inputs has always
# treated a missing node — the input is skipped / reported unknown —
# never a raw AttributeError/TypeError/KeyError. The planning pass
# (discover_inputs over the pristine lock) decides what runs, and
# because the execution loop's readers share the same discipline, a
# malformed lock cannot crash the run after earlier inputs have
# already been updated.


def _as_dict(value):
    """`value` if it is a JSON object, else an empty dict."""
    return value if isinstance(value, dict) else {}


def _as_str(value):
    """`value` if it is a string, else None."""
    return value if isinstance(value, str) else None


def _lock_nodes(lock_data):
    """The lock's "nodes" object ({} for any wrong shape)."""
    return _as_dict(_as_dict(lock_data).get("nodes"))


def _root_inputs(lock_data):
    """The root node's "inputs" object ({} for any wrong shape)."""
    return _as_dict(_as_dict(_lock_nodes(lock_data).get("root")).get("inputs"))


def get_input_info(lock_data, input_name):
    """Extract rev, source URL, and ref for an input from parsed flake.lock.

    Returns: (short_rev, source_url, ref_display) — "unknown" fields
    for a missing or wrong-shape node, mirroring discover's skip.
    """
    node = get_input_node(lock_data, input_name)
    if node is None:
        return ("unknown", "unknown", "unknown")

    original = _as_dict(node.get("original"))
    locked = _as_dict(node.get("locked"))

    type_ = (_as_str(original.get("type")) or _as_str(locked.get("type")) or "").lower()
    full_rev = _as_str(locked.get("rev")) or "unknown"
    rev = full_rev[:12] if len(full_rev) > 12 else full_rev
    branch = _as_str(original.get("ref"))
    ref_display = branch if branch else "(default)"

    if type_ == "github":
        owner = _as_str(original.get("owner")) or _as_str(locked.get("owner")) or "?"
        repo = _as_str(original.get("repo")) or _as_str(locked.get("repo")) or "?"
        slug = f"{owner}/{repo}"
        url = f"github:{slug}/{branch}" if branch else f"github:{slug}"
    elif type_ == "tarball":
        url = _as_str(original.get("url")) or _as_str(locked.get("url")) or "?"
    else:
        url = f"{type_}:{json.dumps(original or locked)}"

    return (rev, url, ref_display)


def get_input_node(lock_data, input_name):
    """Return the lock node for a root input name, or None.

    None covers every "not usable" shape uniformly: input not in the
    root inputs, a non-string node reference (follows), a node
    missing from "nodes", and a node that is not an object.
    """
    node_name = _root_inputs(lock_data).get(input_name)
    if not isinstance(node_name, str):
        return None
    return _as_dict(_lock_nodes(lock_data).get(node_name)) or None


def get_input_ref(lock_data, input_name):
    """Return the input's original.ref (None when unset / not found /
    not a string)."""
    node = get_input_node(lock_data, input_name)
    if not node:
        return None
    return _as_str(_as_dict(node.get("original")).get("ref"))


def get_locked_rev(lock_data, input_name):
    """Return the input's full locked revision (None when not locked /
    not a string)."""
    node = get_input_node(lock_data, input_name)
    if not node:
        return None
    return _as_str(_as_dict(node.get("locked")).get("rev"))


def discover_inputs(lock_data, ref):
    """Discover NixOS/nixpkgs inputs by parsing flake.lock directly.

    Walks root inputs, skips follows (array values), and matches by URL:
    original.type == "github", original.owner ~= NixOS (case-insensitive),
    original.repo == "nixpkgs".

    Filters by original.ref when ref is specified:
      ref=None  → all NixOS/nixpkgs inputs
      ref="master" → inputs with ref="master" or no ref (default branch)
      ref=<other>  → inputs with that exact ref
    """
    root_inputs = _root_inputs(lock_data)
    nodes = _lock_nodes(lock_data)
    results = []

    for name, node_ref in root_inputs.items():
        # Skip follows (array values like ["nixpkgs-master"])
        if not isinstance(node_ref, str):
            continue

        node = nodes.get(node_ref)
        if not isinstance(node, dict):
            continue

        original = _as_dict(node.get("original"))

        # Match by URL: github:NixOS/nixpkgs
        if (_as_str(original.get("type")) or "").lower() != "github":
            continue
        owner = original.get("owner")
        if not isinstance(owner, str) or owner.lower() != "nixos":
            continue
        if original.get("repo", "") != "nixpkgs":
            continue

        # Filter by ref. A ref that is present but not a string is a
        # poison entry (#6847): the input cannot be classified, so it
        # is skipped in every pass rather than read as no ref.
        input_ref = original.get("ref")
        if input_ref is not None and not isinstance(input_ref, str):
            continue
        if ref is not None:
            if ref == "master":
                # master matches explicit "master" or no ref (github default)
                if input_ref is not None and input_ref != "master":
                    continue
            else:
                if input_ref != ref:
                    continue

        results.append(name)

    return sorted(results)


def update_input(input_name, repo_dir):
    """Run nix flake update for a single input. Returns True on success."""
    result = subprocess.run(
        ["nix", "flake", "update", input_name, "--refresh"],
        cwd=str(repo_dir),
    )
    return result.returncode == 0


def pin_input(input_name, repo_dir, locked):
    """Pin an input to a pre-fetched locked entry, rewriting only the
    node's `locked` in flake.lock (`original` stays untouched, so the
    input's ref — and therefore discovery — is preserved). The dump
    format matches nix's own flake.lock formatting byte for byte.
    Returns True on success."""
    lock_data = load_flake_lock(repo_dir)
    node = get_input_node(lock_data, input_name)
    if node is None:
        return False
    node["locked"] = dict(locked)
    with open(repo_dir / "flake.lock", "w") as f:
        f.write(json.dumps(lock_data, indent=2) + "\n")
    return get_locked_rev(load_flake_lock(repo_dir), input_name) == locked["rev"]


def format_result_lines(results):
    """Format result detail lines grouped by source URL."""
    url_order = []
    url_groups = {}
    for r in results:
        url = r["url"]
        if url not in url_groups:
            url_order.append(url)
            url_groups[url] = []
        url_groups[url].append(r)

    lines = []
    for url in url_order:
        lines.append("")
        lines.append(f" {url}")
        lines.append(f" {'─' * len(url)}")
        for r in url_groups[url]:
            if r["status"] == "updated":
                lines.append(
                    f"   ✓ {r['name']}  {r['old_rev']} → {r['new_rev']}  [ref: {r['ref']}]"
                )
            elif r["status"] == "skipped":
                lines.append(
                    f"   - {r['name']}  {r['old_rev']}  [ref: {r['ref']}] (already up-to-date)"
                )
            elif r["status"] == "failed":
                lines.append(f"   ✗ {r['name']}  {r['old_rev']}  [ref: {r['ref']}]")
    return lines


def print_ref_summary(label, results, not_found_refs=None):
    """Print the summary for a single ref category."""
    updated = [r for r in results if r["status"] == "updated"]
    skipped = [r for r in results if r["status"] == "skipped"]
    failed = [r for r in results if r["status"] == "failed"]

    print()
    print("=" * 40)
    print(f" Summary: NixOS/nixpkgs ({label}) inputs")
    print("=" * 40)
    print(f" Total:   {len(results)}")
    print(f" Updated: {len(updated)}")
    print(f" Skipped: {len(skipped)}")
    print(f" Failed:  {len(failed)}")
    print("=" * 40)

    for line in format_result_lines(results):
        print(line)

    if not_found_refs:
        print()
        print(" No inputs found for:")
        for nf in not_found_refs:
            print(f"   · {nf}")

    print()
    print("=" * 40)


def print_combined_summary(all_ref_data):
    """Print a combined overall summary across all ref categories."""
    all_results = []
    not_found_refs = []
    ref_labels = []

    for rd in all_ref_data:
        ref_labels.append(rd["label"])
        all_results.extend(rd["results"])
        if rd.get("not_found"):
            not_found_refs.append(rd["label"])

    updated = [r for r in all_results if r["status"] == "updated"]
    skipped = [r for r in all_results if r["status"] == "skipped"]
    failed = [r for r in all_results if r["status"] == "failed"]

    sep = "═" * 48
    print()
    print(sep)
    print(f" Overall Summary")
    print(sep)
    print(f" Categories: {', '.join(ref_labels)}")
    print(f" Total:      {len(all_results)}")
    print(f" Updated:    {len(updated)}")
    print(f" Skipped:    {len(skipped)}")
    print(f" Failed:     {len(failed)}")
    print(sep)

    for line in format_result_lines(all_results):
        print(line)

    if not_found_refs:
        print()
        print(" No inputs found for:")
        for nf in not_found_refs:
            print(f"   · {nf}")

    print()
    print(sep)


def main():
    script_dir = Path(__file__).resolve().parent
    repo_dir = script_dir.parent

    refs = sys.argv[1:] if len(sys.argv) > 1 else [None]

    # Planning pass: resolve, against the pristine lock, which inputs each
    # requested ref will process (same discovery + cross-ref dedupe as the
    # execution pass below). The cache gate must be resolved — and must
    # fail closed — before any input is touched, so gated inputs are
    # identified up front.
    plan = []
    seen_inputs = set()
    gated_inputs = set()
    initial_lock = load_flake_lock(repo_dir)
    for ref in refs:
        inputs = discover_inputs(initial_lock, ref)
        new_inputs = [i for i in inputs if i not in seen_inputs]
        seen_inputs.update(new_inputs)
        plan.append((ref, inputs, new_inputs))
        for name in new_inputs:
            if get_input_ref(initial_lock, name) == GATED_REF:
                gated_inputs.add(name)

    gate_rev = None
    gate_locked = None
    if gated_inputs:
        try:
            gate_rev, gate_eval = hydra_cached_revision()
        except HydraGateError as e:
            print(
                f"ERROR: Hydra cache gate failed: {e}\n"
                f"Refusing to update {GATED_REF} inputs to uncached "
                "revisions; flake.lock left untouched.",
                file=sys.stderr,
            )
            sys.exit(1)
        gate_locked = fetch_locked_for_rev(gate_rev)
        if gate_locked is None:
            print(
                f"ERROR: could not fetch github:NixOS/nixpkgs@{gate_rev} "
                "with nix flake metadata; flake.lock left untouched.",
                file=sys.stderr,
            )
            sys.exit(1)
        print(
            f"Hydra gate: {GATED_REF} inputs pin to {gate_rev[:12]} "
            f"(newest revision fully built by Hydra jobset nixos:unstable, "
            f"eval {gate_eval})"
        )

    all_ref_data = []

    for ref, inputs, new_inputs in plan:
        label = ref if ref else "all"
        ref_data = {"label": label, "results": [], "not_found": False}

        if not inputs:
            ref_data["not_found"] = True
            all_ref_data.append(ref_data)
            # Print this ref's summary now: without it, a run where
            # every ref is unmatched prints nothing at all, and the
            # non-zero exit below arrives with no named cause (#6941).
            print_ref_summary(label, ref_data["results"], [label])
            continue

        if not new_inputs:
            all_ref_data.append(ref_data)
            continue

        print(f"Updating NixOS/nixpkgs ({label}) inputs: {' '.join(new_inputs)}")

        for input_name in new_inputs:
            print(f"--- Updating {input_name} ---")

            lock_data = load_flake_lock(repo_dir)
            old_rev, source_url, input_ref = get_input_info(lock_data, input_name)

            if input_name in gated_inputs:
                if get_locked_rev(lock_data, input_name) == gate_rev:
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "skipped",
                            "old_rev": old_rev,
                            "new_rev": None,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
                elif pin_input(input_name, repo_dir, gate_locked):
                    lock_data = load_flake_lock(repo_dir)
                    new_rev, _, _ = get_input_info(lock_data, input_name)
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "updated",
                            "old_rev": old_rev,
                            "new_rev": new_rev,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
                else:
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "failed",
                            "old_rev": old_rev,
                            "new_rev": None,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
                    print(
                        f"WARNING: failed to pin {input_name} to {gate_rev}",
                        file=sys.stderr,
                    )
                continue

            if update_input(input_name, repo_dir):
                lock_data = load_flake_lock(repo_dir)
                new_rev, _, _ = get_input_info(lock_data, input_name)

                if old_rev != new_rev:
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "updated",
                            "old_rev": old_rev,
                            "new_rev": new_rev,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
                else:
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "skipped",
                            "old_rev": old_rev,
                            "new_rev": None,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
            else:
                ref_data["results"].append(
                    {
                        "name": input_name,
                        "status": "failed",
                        "old_rev": old_rev,
                        "new_rev": None,
                        "url": source_url,
                        "ref": input_ref,
                    }
                )
                print(f"WARNING: failed to update {input_name}", file=sys.stderr)

        all_ref_data.append(ref_data)

        # Print per-ref summary as we go
        not_found_in_ref = [rd["label"] for rd in all_ref_data if rd["not_found"]]
        print_ref_summary(label, ref_data["results"], not_found_in_ref)

    # Print combined overall summary if multiple refs were requested
    if len(refs) > 1:
        print_combined_summary(all_ref_data)

    # A run that failed to update any input is a failure, even though the
    # summaries above printed normally: callers (&& chains, CI, cron) must
    # not proceed as if the inputs were updated.
    failed_total = sum(
        1 for rd in all_ref_data for r in rd["results"] if r["status"] == "failed"
    )
    if failed_total:
        sys.exit(1)

    # A requested ref that matched no inputs is also a failure (#6941):
    # a typo'd ref (or --help) otherwise exits 0 after updating nothing,
    # and callers proceed as if the pin was refreshed. The summaries
    # above already name the unmatched refs ("No inputs found for:").
    # This also pins the no-argument run against a lock with zero
    # nixpkgs inputs (label "all"): nothing to update is not a success
    # for a tool whose only job is updating those inputs.
    if any(rd["not_found"] for rd in all_ref_data):
        sys.exit(1)


if __name__ == "__main__":
    main()
