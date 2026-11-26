#!/usr/bin/env python3
"""Synthetic conformance oracle for id:// v2 failure semantics (D1 slice 3, #13886).

Stdlib only. Run with:  python3 failure_reference.py
(or: python3 -I failure_reference.py self-test)

This is a model, not a resolver. It encodes the slice-3 matrix as one small
function over a synthetic object/caller state and checks the cases the design
record names:

  resolve(state) -> "ok" | "unavailable" | "tombstone" | "stale_head"

The rules it pins, from the decision record on #13886
(issuecomment-7157414471) and slice 1 §7:

  * A URI names an object; it never grants authority. Presenting the URI, or
    any via hint, never changes the outcome.
  * A lapsed grant is not renewed by a URI or a hint. Under the non-renewal
    posture a lapsed caller is exactly an unauthorised caller.
  * A caller without current access gets the one coalesced answer,
    "unavailable", for an unknown object, a pruned object and an existing
    object they may not read. The three are indistinguishable: no cause
    field, no difference in shape.
  * Only a caller with current access may see retained detail: a retained
    tombstone for a pruned object ("tombstone"), or, for the head/at form,
    that the pinned head is no longer current ("stale_head").
"""

from __future__ import annotations

import sys

sys.dont_write_bytecode = True

# Caller access states. "lapsed" is kept distinct in the model only so the
# cases can state it; resolve() treats it exactly like "unauthorised".
AUTHORISED = "authorised"
UNAUTHORISED = "unauthorised"
LAPSED = "lapsed"


def resolve(
    *,
    exists: bool,
    pruned: bool = False,
    tombstone_retained: bool = False,
    caller: str = AUTHORISED,
    head_stale: bool = False,
    uri_presented: bool = True,
    hint_presented: bool = False,
) -> str:
    """The slice-3 outcome for one synthetic resolution.

    `uri_presented` and `hint_presented` are accepted so the cases can state
    them; by design they never affect the result (naming is not authority,
    and a lapsed grant is not renewed by either).
    """
    del uri_presented, hint_presented  # never authority, never renewal
    if caller != AUTHORISED:
        # Unknown, pruned, existing-but-unreadable, lapsed: one answer.
        return "unavailable"
    if not exists:
        # Nothing retained exists to describe; even an authorised caller
        # cannot be told more than that the name does not resolve here.
        return "unavailable"
    if pruned:
        # Detail only where a tombstone was retained, and only for a caller
        # who could have read the object.
        return "tombstone" if tombstone_retained else "unavailable"
    if head_stale:
        # head/at only: the pinned head is readable but no longer current.
        return "stale_head"
    return "ok"


CASES = [
    # (name, state, expected outcome)
    ("existing object, authorised caller", dict(exists=True), "ok"),
    (
        "unknown object, authorised caller",
        dict(exists=False),
        "unavailable",
    ),
    (
        "unknown object, unauthorised caller",
        dict(exists=False, caller=UNAUTHORISED),
        "unavailable",
    ),
    (
        "existing object, unauthorised caller",
        dict(exists=True, caller=UNAUTHORISED),
        "unavailable",
    ),
    (
        "pruned object, authorised caller, tombstone retained",
        dict(exists=True, pruned=True, tombstone_retained=True),
        "tombstone",
    ),
    (
        "pruned object, authorised caller, no tombstone retained",
        dict(exists=True, pruned=True, tombstone_retained=False),
        "unavailable",
    ),
    (
        "pruned object, unauthorised caller, tombstone retained",
        dict(exists=True, pruned=True, tombstone_retained=True, caller=UNAUTHORISED),
        "unavailable",
    ),
    (
        "lapsed grant, object still exists",
        dict(exists=True, caller=LAPSED),
        "unavailable",
    ),
    (
        "lapsed grant, URI and hint presented (no renewal)",
        dict(exists=True, caller=LAPSED, uri_presented=True, hint_presented=True),
        "unavailable",
    ),
    (
        "lapsed grant, pruned object with retained tombstone",
        dict(exists=True, pruned=True, tombstone_retained=True, caller=LAPSED),
        "unavailable",
    ),
    (
        "stale pinned head, authorised caller",
        dict(exists=True, head_stale=True),
        "stale_head",
    ),
    (
        "stale pinned head, unauthorised caller",
        dict(exists=True, head_stale=True, caller=UNAUTHORISED),
        "unavailable",
    ),
    (
        "current pinned head, authorised caller",
        dict(exists=True, head_stale=False),
        "ok",
    ),
]


def fail(message: str) -> int:
    print(f"FAIL {message}", file=sys.stderr)
    return 1


def self_test() -> int:
    for name, state, want in CASES:
        got = resolve(**state)
        if got != want:
            return fail(f"{name}: {got} != {want}")

    # The collapse property, stated directly: for any caller without current
    # access, unknown, pruned (retained or not) and existing objects all
    # return the identical outcome, whatever is presented with the name.
    for caller in (UNAUTHORISED, LAPSED):
        outcomes = {
            resolve(exists=False, caller=caller),
            resolve(exists=True, caller=caller),
            resolve(exists=True, pruned=True, tombstone_retained=True, caller=caller),
            resolve(exists=True, pruned=True, tombstone_retained=False, caller=caller),
            resolve(exists=True, head_stale=True, caller=caller),
            resolve(
                exists=True,
                caller=caller,
                uri_presented=True,
                hint_presented=True,
            ),
        }
        if outcomes != {"unavailable"}:
            return fail(f"collapse broken for {caller}: {outcomes}")

    # Authority is the only key: flipping nothing but the caller's current
    # access is the only way retained detail ever appears.
    detailed = {
        resolve(exists=True, pruned=True, tombstone_retained=True),
        resolve(exists=True, head_stale=True),
    }
    if detailed != {"tombstone", "stale_head"}:
        return fail(f"authorised detail wrong: {detailed}")

    print(
        f"ok: {len(CASES)} conformance cases, collapse holds for "
        f"unauthorised and lapsed callers, detail only for authorised callers"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) == 1 or (len(argv) == 2 and argv[1] == "self-test"):
        return self_test()
    print("usage: failure_reference.py [self-test]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
