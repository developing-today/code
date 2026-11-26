"""The arithmetic oracle for the #9098 storage-cost bound."""

import json
import sys
import unittest
from decimal import Decimal
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
from storage_bound import calculate_storage_bound  # noqa: E402

MEASUREMENTS = REPO / "doc/2026-11-25T11-33-10Z_report_atlas_embed_storage_composition_9098/measurements.json"


class StorageBoundTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.measurements = json.loads(MEASUREMENTS.read_text())
        cls.result = calculate_storage_bound(
            cls.measurements,
            r2_standard_rate_usd_per_gb_month=Decimal("0.015"),
            r2_billable_unit_gb=Decimal("1"),
            r2_free_gb_month_available=Decimal("10"),
            turso_overages_enabled=False,
        )

    def test_sums_only_measured_raw_text_from_every_store_class(self):
        self.assertEqual(self.result["raw_text_bytes_by_store"], {
            "monolith": 639_187_867,
            "topic_stores": 173_545_247,
            "content_stores": 464_959_914,
        })
        self.assertEqual(self.result["turso_bytes_freed_if_all_raw_text_is_offloaded"], 1_277_693_028)

    def test_r2_storage_upper_bound_rounds_to_billable_gigabytes(self):
        self.assertEqual(self.result["r2_unique_content_bytes_upper_bound"], 1_277_693_028)
        self.assertEqual(self.result["r2_decimal_gb_upper_bound"], Decimal("1.277693028"))
        self.assertEqual(self.result["r2_billable_gb_months_if_free_tier_exhausted"], Decimal("2"))
        self.assertEqual(self.result["r2_gross_monthly_storage_usd_if_free_tier_exhausted"], Decimal("0.030"))

    def test_available_free_tier_and_turso_overage_state_are_explicit(self):
        self.assertEqual(self.result["r2_incremental_storage_usd_if_free_tier_available"], Decimal("0"))
        self.assertEqual(self.result["turso_invoice_savings_usd_under_current_overage_setting"], Decimal("0"))
        self.assertFalse(self.result["turso_overages_enabled"])


if __name__ == "__main__":
    unittest.main()
