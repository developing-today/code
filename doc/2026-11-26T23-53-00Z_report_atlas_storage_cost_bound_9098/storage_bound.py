"""Conservative storage and cost bounds from the Atlas composition census."""

from decimal import Decimal, ROUND_CEILING
from typing import Any


def _sum_bytes(stores: list[dict[str, Any]], field: str) -> int:
    total = 0
    for store in stores:
        section = store.get(field)
        if section is None:
            continue
        if not isinstance(section, dict):
            raise ValueError(f"{field} section is not an object")
        value = section.get("bytes")
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            raise ValueError(f"{field}.bytes must be a non-negative integer")
        total += value
    return total


def _ceil_units(amount: Decimal, unit: Decimal) -> Decimal:
    if amount < 0 or unit <= 0:
        raise ValueError("amount must be non-negative and unit must be positive")
    if amount == 0:
        return Decimal(0)
    return (amount / unit).to_integral_value(rounding=ROUND_CEILING) * unit


def calculate_storage_bound(
    measurements: dict[str, Any],
    *,
    r2_standard_rate_usd_per_gb_month: Decimal,
    r2_billable_unit_gb: Decimal,
    r2_free_gb_month_available: Decimal,
    turso_storage_used_gb_issue_snapshot: Decimal,
    turso_storage_included_gb: Decimal,
    turso_overages_enabled: bool,
) -> dict[str, Any]:
    """Return bounds; raw bytes are summed across copies, without assuming dedup."""
    if not isinstance(measurements, dict):
        raise ValueError("measurements must be an object")
    monolith = measurements.get("monolith")
    tiers = measurements.get("tier_stores")
    if not isinstance(monolith, dict) or not isinstance(tiers, list):
        raise ValueError("measurements must contain monolith and tier_stores")
    monolith_bytes = monolith.get("raw_text_bytes")
    if (
        isinstance(monolith_bytes, bool)
        or not isinstance(monolith_bytes, int)
        or monolith_bytes < 0
    ):
        raise ValueError("monolith.raw_text_bytes must be a non-negative integer")
    if r2_standard_rate_usd_per_gb_month < 0:
        raise ValueError("R2 storage rate must be non-negative")
    if r2_free_gb_month_available < 0:
        raise ValueError("R2 free allowance must be non-negative")
    if turso_storage_used_gb_issue_snapshot < 0 or turso_storage_included_gb <= 0:
        raise ValueError(
            "Turso storage values must be non-negative and included storage positive"
        )

    topic_text_bytes = _sum_bytes(tiers, "chunk_text")
    content_text_bytes = _sum_bytes(tiers, "messages")
    total_bytes = monolith_bytes + topic_text_bytes + content_text_bytes
    decimal_gb = Decimal(total_bytes) / Decimal(1_000_000_000)
    billable_gb_if_free_exhausted = _ceil_units(decimal_gb, r2_billable_unit_gb)
    paid_gb_after_free = max(Decimal(0), decimal_gb - r2_free_gb_month_available)
    billable_paid_gb = _ceil_units(paid_gb_after_free, r2_billable_unit_gb)
    turso_after_offload = max(
        Decimal(0), turso_storage_used_gb_issue_snapshot - decimal_gb
    )

    return {
        "measurement_scope": measurements.get("measurement_scope", "unspecified"),
        "measured_at_utc": measurements.get("measured_at_utc"),
        "raw_text_bytes_by_store": {
            "monolith": monolith_bytes,
            "topic_stores": topic_text_bytes,
            "content_stores": content_text_bytes,
        },
        "turso_bytes_freed_if_all_measured_raw_text_is_offloaded": total_bytes,
        "r2_unique_content_bytes_upper_bound_if_no_dedup": total_bytes,
        "r2_decimal_gb_upper_bound": decimal_gb,
        "r2_billable_gb_months_if_free_tier_exhausted": billable_gb_if_free_exhausted,
        "r2_gross_monthly_storage_usd_if_free_tier_exhausted": (
            billable_gb_if_free_exhausted * r2_standard_rate_usd_per_gb_month
        ),
        "r2_incremental_storage_usd_if_full_free_allowance_remains": (
            billable_paid_gb * r2_standard_rate_usd_per_gb_month
        ),
        "turso_storage_used_gb_issue_snapshot": turso_storage_used_gb_issue_snapshot,
        "turso_storage_after_offload_gb_if_snapshot_held": turso_after_offload,
        "turso_storage_included_gb": turso_storage_included_gb,
        "turso_storage_overages_enabled": turso_overages_enabled,
        "turso_invoice_savings_usd_under_issue_overage_setting": (
            None if turso_overages_enabled else Decimal(0)
        ),
        "r2_operation_cost_measured": False,
        "hydration_latency_measured": False,
    }


def _json_default(value: object) -> str:
    if isinstance(value, Decimal):
        return str(value)
    raise TypeError(f"cannot encode {type(value).__name__}")


def main(argv: list[str]) -> int:
    if argv:
        raise SystemExit("usage: python3 -I storage_bound.py")
    import json
    from pathlib import Path

    here = Path(__file__).resolve().parent
    repo = here.parents[1]
    measurements = json.loads(
        (
            repo
            / "doc/2026-11-25T11-33-10Z_report_atlas_embed_storage_composition_9098/measurements.json"
        ).read_text()
    )
    assumptions = json.loads((here / "cost_inputs.json").read_text())
    report = calculate_storage_bound(
        measurements,
        r2_standard_rate_usd_per_gb_month=Decimal(
            assumptions["r2_standard_rate_usd_per_gb_month"]
        ),
        r2_billable_unit_gb=Decimal(assumptions["r2_billable_unit_gb"]),
        r2_free_gb_month_available=Decimal(
            assumptions["r2_free_gb_month_available_scenario"]
        ),
        turso_storage_used_gb_issue_snapshot=Decimal(
            assumptions["turso_storage_used_gb_issue_snapshot"]
        ),
        turso_storage_included_gb=Decimal(assumptions["turso_storage_included_gb"]),
        turso_overages_enabled=assumptions["turso_storage_overages_enabled"],
    )
    report["pricing_assumptions"] = assumptions
    print(json.dumps(report, indent=2, default=_json_default))
    return 0


if __name__ == "__main__":
    import sys

    raise SystemExit(main(sys.argv[1:]))
