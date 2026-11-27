#!/usr/bin/env python3
"""Hermetic tests for laptop-db-backup leg wiring (mesh class E, #13459/#13474).

Oracle: the repo tree itself. Every `systemd/run-<leg>-import.sh`
wrapper must have a matching `<leg>.service`/`<leg>.timer` pair, be
enrolled in `systemd/install-units.sh`, and appear in both the
README's leg table and `doc/fleet-schedule-registry.md`. A leg that
ships a wrapper but no timer (or no registry row) never runs — this
test fails the PR instead of the Sunday morning.

Run: python3 -I tests/test_leg_wiring.py
"""

import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LDB = os.path.join(HERE, "..")
SYSTEMD = os.path.join(LDB, "systemd")
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))


def _legs():
    legs = []
    for name in sorted(os.listdir(SYSTEMD)):
        m = re.fullmatch(r"run-(.+)-import\.sh", name)
        if m:
            legs.append(m.group(1))
    return legs


def test_every_wrapper_has_units_and_install():
    legs = _legs()
    assert legs, "no run-*-import.sh wrappers found"
    install = open(os.path.join(SYSTEMD, "install-units.sh")).read()
    for leg in legs:
        for ext in ("service", "timer"):
            path = os.path.join(SYSTEMD, f"laptop-db-backup-{leg}.{ext}")
            assert os.path.isfile(path), f"leg {leg}: missing {path}"
            assert f"laptop-db-backup-{leg}.{ext}" in install, (
                f"leg {leg}: install-units.sh does not enroll .{ext}"
            )


def test_every_timer_is_enabled_by_install():
    install = open(os.path.join(SYSTEMD, "install-units.sh")).read()
    enable_lines = [
        line for line in install.splitlines() if "systemctl --user enable --now" in line
    ]
    assert enable_lines, "install-units.sh does not enable any backup timer"
    for leg in _legs():
        timer = f"laptop-db-backup-{leg}.timer"
        assert any(timer in line for line in enable_lines), (
            f"leg {leg}: install-units.sh installs but never enables {timer}"
        )


def test_every_wrapper_names_its_home_and_source():
    for leg in _legs():
        text = open(os.path.join(SYSTEMD, f"run-{leg}-import.sh")).read()
        assert "TURSO_HOME_TOKEN" in text, f"leg {leg}: wrapper never exports token"
        assert "laptop-backup-" in text, f"leg {leg}: wrapper names no home"
        assert "--source" in text and "--home-url" in text, (
            f"leg {leg}: wrapper does not invoke the import with source+home"
        )


def test_every_leg_in_readme_and_registry():
    readme = open(os.path.join(LDB, "README.md")).read()
    registry = open(os.path.join(REPO, "doc", "fleet-schedule-registry.md")).read()
    for leg in _legs():
        assert f"laptop-db-backup-{leg}.timer" in readme, (
            f"leg {leg}: README leg table omits the timer"
        )
        assert f"laptop-db-backup-{leg}.timer" in registry, (
            f"leg {leg}: fleet-schedule-registry.md omits the timer"
        )


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print(f"ok {t.__name__}")
    print(f"{len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
