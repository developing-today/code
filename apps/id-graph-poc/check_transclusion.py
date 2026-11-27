"""Done check for PoC 2 (#14037 slice 2).

Oracle: D2 fixtures/expected.json defines the render trees and
D2 §7 / slice-2-capability-matrix.md keeps delegated grants off by default.
Run: python3 -I apps/id-graph-poc/check_transclusion.py
"""

import copy
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from transclusion import DELEGATED_VIEW_GRANTS, load_graph, render_composed, render_object  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
D2_FIXTURES = REPO / "doc" / "2026-11-25T01-40-00Z_design_id-apps_d2_transclusion" / "fixtures"
EXPECTED = json.loads((D2_FIXTURES / "expected.json").read_text(encoding="utf-8"))

failures = []


def expect(cond, message):
    if cond:
        print(f"OK: {message}")
    else:
        failures.append(message)
        print(f"FAIL: {message}")


graph = load_graph()
composed = render_composed(graph, viewer="ana")
sections = composed.get("sections", [])
expect(
    len(sections) > 0 and sections[0] == EXPECTED["page-embeds-collection"],
    "one composed page renders the D2 live collection tree",
)
expect(
    len(sections) > 1 and sections[1] == EXPECTED["post-embeds-photo"],
    "the same composed page renders the D2 live object tree",
)

# A new live head must show on the next render, without editing the embed link.
edited_graph = copy.deepcopy(graph)
edited_graph["objects"]["img-group-photo-v3"] = {
    "object_id": "img-group-photo-v3",
    "area": "cyp-public",
    "author": "ed25519:cccc3333media",
    "created_claim": "2026-11-24T04:00:00Z",
    "kind": "image",
    "body": {"digest": "blake3:group-photo-v3-edited", "mime": "image/jpeg", "size": 1800000},
    "schema_version": 1,
}
edited_graph["links"].append(
    {
        "object_id": "lnk-supersede-photo-v3",
        "author": "ed25519:cccc3333media",
        "created_claim": "2026-11-24T04:00:00Z",
        "verb": "supersede",
        "source": "img-group-photo-v3",
        "target": "img-group-photo-v2",
        "schema_version": 1,
    }
)
updated = render_composed(edited_graph, viewer="ana")
updated_sections = updated.get("sections", [])
updated_embeds = updated_sections[1].get("embeds", []) if len(updated_sections) > 1 else []
updated_object = updated_embeds[0] if updated_embeds else {}
expect(
    updated_object.get("object") == "img-group-photo-v3" and updated_object.get("badges") == ["edited"],
    "a new superseding target appears on the next live render",
)

outsider = render_object(graph, "post-budget-pointer", viewer="ana")
expect(
    outsider == EXPECTED["private-target-outsider"],
    "outsider private-target slot exactly matches D2 opaque-unavailable oracle",
)
outsider_embed = outsider.get("embeds", [{}])[0]
expect(
    set(outsider_embed) == {"link", "state"}
    and outsider_embed.get("state") == "unavailable",
    "unavailable slot reveals no mode, scope, target id, title, author, or body",
)
expect(DELEGATED_VIEW_GRANTS is False, "delegated view grants remain disabled")

print(f"\n{len(failures)} failure(s).")
sys.exit(1 if failures else 0)
