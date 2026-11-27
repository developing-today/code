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
ASSUMPTIONS = HERE / "cost_inputs.json"


class StorageBoundTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.measurements = json.loads(MEASUREMENTS.read_text())
        cls.assumptions = json.loads(ASSUMPTIONS.read_text())
        cls.result = calculate_storage_bound(
            cls.measurements,
            r2_standard_rate_usd_per_gb_month=Decimal(cls.assumptions["r2_standard_rate_usd_per_gb_month"]),
            r2_billable_unit_gb=Decimal(cls.assumptions["r2_billable_unit_gb"]),
            r2_free_gb_month_available=Decimal(cls.assumptions["r2_free_gb_month_available_scenario"]),
            turso_storage_used_gb_issue_snapshot=Decimal(cls.assumptions["turso_storage_used_gb_issue_snapshot"]),
            turso_storage_included_gb=Decimal(cls.assumptions["turso_storage_included_gb"]),
            turso_overages_enabled=cls.assumptions["turso_storage_overages_enabled"],
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
        self.assertEqual(self.result["r2_incremental_storage_usd_if_full_free_allowance_remains"], Decimal("0"))
        self.assertFalse(self.result["turso_storage_overages_enabled"])
        self.assertEqual(self.result["turso_invoice_savings_usd_under_issue_overage_setting"], Decimal("0"))
        self.assertEqual(self.result["turso_storage_after_offload_gb_if_snapshot_held"], Decimal("9.722306972"))


if __name__ == "__main__":
    unittest.main()
