"""Conservative storage and cost bounds from the Atlas composition census."""

from decimal import Decimal
from typing import Any


def calculate_storage_bound(
    measurements: dict[str, Any],
    *,
    r2_standard_rate_usd_per_gb_month: Decimal,
    r2_billable_unit_gb: Decimal,
    r2_free_gb_month_available: Decimal,
    turso_overages_enabled: bool,
) -> dict[str, Any]:
    raise NotImplementedError("the #9098 storage-cost bound is not implemented")
