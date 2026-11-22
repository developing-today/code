"""Shared deterministic renderer and token oracle for issue/source bindings."""
from __future__ import annotations

from typing import Any


def _ranges(value: Any) -> str:
    if isinstance(value, list) and len(value) == 2 and all(type(n) is int for n in value):
        value = [value]
    if not isinstance(value, list) or not value:
        raise ValueError("source_lines must be a nonempty list of inclusive ranges")
    out = []
    previous = 0
    for pair in value:
        if (not isinstance(pair, list) or len(pair) != 2 or
                any(type(n) is not int for n in pair) or pair[0] < 1 or pair[1] < pair[0] or pair[0] <= previous):
            raise ValueError(f"invalid source line span: {pair!r}")
        out.append(str(pair[0]) if pair[0] == pair[1] else f"{pair[0]}-{pair[1]}")
        previous = pair[1]
    return ", ".join(out)


def group_rows(bindings: list[tuple[dict[str, Any], str]]) -> list[tuple[dict[str, Any], str, list[str], list[str]]]:
    """Group byte-identical clause/oracle witnesses while retaining every exact ID."""
    grouped: dict[tuple[Any, ...], list[dict[str, Any]]] = {}
    for row, role in bindings:
        line_key = tuple(tuple(span) for span in row.get("source_lines", []))
        key = (role, row.get("row_type"), row.get("canonical_requirement_id"),
               row.get("source_path"), line_key, row.get("source_file_and_section"),
               row.get("source_statement"), row.get("source_specific_oracle"),
               row.get("expert_test_oracle"), row.get("design_section"), row.get("test_oracle"),
               row.get("disposition"), row.get("disposition_reason"),
               tuple(row.get("acceptance_enrichment", [])))
        grouped.setdefault(key, []).append(row)
    result = []
    for key, rows in grouped.items():
        role = key[0]
        source_ids = sorted({str(row.get("member_id") or row["source_id"]) for row in rows})
        parent_ids = sorted({str(row["parent_source_id"]) for row in rows if row.get("parent_source_id")})
        result.append((rows[0], role, source_ids, parent_ids))
    return sorted(result, key=lambda item: (item[0].get("source_path", ""), item[0].get("source_id", ""), item[1]))


def binding_tokens(row: dict[str, Any], role: str, source_ids: list[str],
                   parent_ids: list[str]) -> list[str]:
    expert = row.get("expert_test_oracle", "")
    oracles = ([row.get("source_specific_oracle", ""), expert] if expert else [row.get("test_oracle", "")])
    tokens = [*source_ids, row.get("canonical_requirement_id", ""),
              f"- Ownership role in this issue: {role}", row.get("source_statement", ""),
              row.get("design_section", ""), *oracles, row.get("disposition_reason", "")]
    tokens.extend(parent_ids)
    tokens.extend(row.get("acceptance_enrichment", []))
    return [token for token in tokens if token]


def render_binding_block(row: dict[str, Any], role: str, source_pin: dict[str, str],
                         source_ids: list[str], parent_ids: list[str]) -> str:
    ranges = _ranges(row["source_lines"])
    path = row["source_path"]
    if path == "note.md":
        relative_path = source_pin["auxiliary_source"]
    else:
        relative_path = source_pin["source_root"] + "/" + path
    first_range = row["source_lines"][0]
    last_range = row["source_lines"][-1]
    anchor = f"#L{first_range[0]}" + (f"-L{last_range[1]}" if last_range[1] != first_range[0] else "")
    url = (f"https://github.com/{source_pin['repository']}/blob/"
           f"{source_pin['snapshot_commit']}/{relative_path}{anchor}")
    label = row.get("row_type", "source-statement")
    ids = ", ".join(f"`{sid}`" for sid in source_ids)
    lines = [f"### {label}: {ids}",
             f"- Canonical requirement: `{row['canonical_requirement_id']}`",
             f"- Ownership role in this issue: {role}"]
    if parent_ids:
        lines.append("- Parent source IDs: " + ", ".join(f"`{pid}`" for pid in parent_ids))
    lines += [f"- Pinned source: [`{path}` lines {ranges}]({url}); `{row['source_file_and_section']}`",
              f"- Source clause: {row['source_statement']}",
              f"- Design target: {row['design_section']}"]
    if row.get("expert_test_oracle"):
        lines += [f"- Source-specific oracle (preserved verbatim, not replaced): {row.get('source_specific_oracle', '')}",
                  f"- Expert planned acceptance oracle (not yet run): {row['expert_test_oracle']}"]
    else:
        lines.append(f"- Planned acceptance oracle (not yet run): {row['test_oracle']}")
    lines.append(f"- Traceability: {row['disposition']}; {row['disposition_reason']}")
    enrichments = row.get("acceptance_enrichment", [])
    if enrichments:
        lines.append("- Additional owner-ruling acceptance clauses:")
        lines.extend(f"  - {clause}" for clause in enrichments)
    return "\n".join(lines)
