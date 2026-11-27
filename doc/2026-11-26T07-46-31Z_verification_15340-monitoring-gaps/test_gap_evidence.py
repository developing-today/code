#!/usr/bin/env python3
# test_gap_evidence.py — evidence anchors for the #15340 criterion-1
# verification record (this directory's .md file).
#
# Command-line contract (#3352): this script declares its whole
# command line — it is a unittest module and accepts only unittest's
# own arguments (e.g. `-v`); it defines no custom flags, reads no
# environment, and writes nothing. Run from the repo root:
#
#   python3 -m unittest discover \
#     -s doc/2026-11-26T07-46-31Z_verification_15340-monitoring-gaps \
#     -p 'test_*.py'
#
# Oracle: the repository tree itself. Each test names one factual
# claim in the verification record and checks the file(s) the record
# cites for it. If a cited anchor moves or is removed, the matching
# test fails and the record must be re-verified, not just edited.
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOC = Path(__file__).resolve().parent / (
    "2026-11-26T07-46-31Z_verification_15340-monitoring-gaps.md"
)


def read(rel):
    return (ROOT / rel).read_text(errors="replace")


class TestVerificationRecord(unittest.TestCase):
    def test_record_states_all_three_verdicts(self):
        text = DOC.read_text()
        for needle in (
            "Gap 2",
            "CLOSED",
            "Gap 4",
            "LIVE CONFIRMATION OUTSTANDING",
            "Pilot demonstration",
            "NOT CONFIRMED",
            "Do not port any new-issue-creator lane yet",
        ):
            self.assertIn(needle, text)


class TestGap2Exporter(unittest.TestCase):
    def test_exporter_and_timer_exist(self):
        base = ROOT / "pm-stack/opencode-usage-export"
        for name in (
            "opencode-usage-export.mjs",
            "opencode-usage-export.test.mjs",
            "opencode-usage-export.service",
            "opencode-usage-export.timer",
            "install-laptop.sh",
            "README.md",
        ):
            self.assertTrue((base / name).is_file(), name)

    def test_exporter_reads_live_v2_tables(self):
        readme = read("pm-stack/opencode-usage-export/README.md")
        self.assertIn("session_v2", readme)
        self.assertIn("session_message", readme)
        # The failure documents the record cites as "never silent zeros".
        for status in ("store-missing", "store-stale", "schema-drift"):
            self.assertIn(status, readme)

    def test_gap1_collector_enriches_tokens_from_same_store(self):
        collect = read("pm-stack/scheduled-history/collect.py")
        self.assertIn("session_v2", collect)
        readme = read("pm-stack/scheduled-history/README.md")
        self.assertIn("Token/cost enrichment", readme)


class TestGap4ResidencyShape(unittest.TestCase):
    def test_openchamber_service_enabled(self):
        nix = read("home/common/default.nix")
        m = re.search(r"services\.openchamber = \{\s*enable = true;", nix)
        self.assertIsNotNone(m)

    def test_stable_entrypoint_keeps_unit_from_bouncing(self):
        nix = read("nixos/home/stable-entrypoints.nix")
        self.assertIn("%h/.local/state/openchamber/start", nix)
        self.assertIn("X-RestartIfChanged", nix)

    def test_ports_records_unit_live(self):
        ports = read("pm-stack/PORTS.md")
        self.assertIn("openchamber.service", ports)

    def test_no_openchamber_socket_unit_in_repo(self):
        offenders = []
        for path in ROOT.rglob("*.socket"):
            if ".git" in path.parts:
                continue
            if (
                "openchamber" in path.name
                or "openchamber" in path.read_text(errors="replace").lower()
            ):
                offenders.append(str(path.relative_to(ROOT)))
        self.assertEqual(offenders, [])


class TestCreatorLaneRegistryRows(unittest.TestCase):
    def test_registry_rows_match_record(self):
        registry = read("doc/fleet-schedule-registry.md")
        for lane, cadence, state in (
            ("new-issue-creator-code-a", "interval@30m", "enabled"),
            ("new-issue-creator-code-b", "interval@30m", "enabled"),
            ("new-issue-creator-opencode", "interval@1h", "enabled"),
            ("new-issue-creator-openchamber", "interval@1h", "enabled"),
            ("new-issue-creator-nix", "interval@2h", "enabled"),
            ("new-issue-creator`", "interval@30m", "disabled"),
            ("new-issue-creator-b`", "interval@30m", "disabled"),
        ):
            row = next(
                (
                    ln
                    for ln in registry.splitlines()
                    if f"`{lane}" in ln or f"| `{lane}" in ln
                ),
                "",
            )
            self.assertIn(cadence, row, lane)
            self.assertIn(state, row, lane)


if __name__ == "__main__":
    unittest.main()
