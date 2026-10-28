#!/usr/bin/env python3
"""power-guard: stop UPower from hybrid-sleeping this machine on a glitch.

Context (2026-10-28): amd-6 hybrid-slept 06:12:29 -> 08:53:10 because
upowerd requested its CriticalPowerAction on a reading that could not
have been real (battery was at 99% on AC minutes later; no low-battery
warnings logged). UPower acts on a single sample. This guard adds the
missing confirmation step:

- Polls the battery every POLL seconds.
- While discharging and at/below LOW_PCT (or a low time-to-empty), it
  holds a block inhibitor on sleep, so UPower's request cannot fire.
- Readings that recover (AC back, charge up, estimate sane) release
  the inhibitor and reset all timers. A glitch never survives minutes
  of continuous polling.
- Only a *sustained* critical state fires the sleep, and the guard
  issues it itself after releasing the inhibitor:
    * at/below EDGE_PCT (truly at the edge): EDGE_CONFIRM seconds
    * at/below CRITICAL_PCT otherwise:      CONFIRM seconds
- On AC the guard can never fire.

Runs as root in production (systemd unit from the NixOS flake).
Test hooks (env): POWER_GUARD_SYSFS (fake sysfs root),
POWER_GUARD_NO_INHIBIT=1 (log inhibitor actions instead of taking a
real lock), POWER_GUARD_SLEEP_CMD (default "systemctl hybrid-sleep").
Threshold/timing overrides: POWER_GUARD_POLL, POWER_GUARD_CONFIRM,
POWER_GUARD_EDGE_CONFIRM.
"""

import os
import subprocess
import sys
import time

SYSFS = os.environ.get("POWER_GUARD_SYSFS", "/sys/class/power_supply")
POLL = float(os.environ.get("POWER_GUARD_POLL", "15"))
LOW_PCT = 20  # engage the block at/below this (UPower PercentageLow)
CRITICAL_PCT = 5  # UPower PercentageCritical
EDGE_PCT = 2  # UPower PercentageAction
CRITICAL_TTE = 300  # seconds-to-empty treated as critical
CONFIRM = float(os.environ.get("POWER_GUARD_CONFIRM", "300"))
EDGE_CONFIRM = float(os.environ.get("POWER_GUARD_EDGE_CONFIRM", "60"))
NO_INHIBIT = os.environ.get("POWER_GUARD_NO_INHIBIT") == "1"
SLEEP_CMD = os.environ.get("POWER_GUARD_SLEEP_CMD", "systemctl hybrid-sleep")


def log(msg):
    print(f"[power-guard {time.strftime('%H:%M:%S')}] {msg}", flush=True)


def read(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def read_int(path):
    v = read(path)
    try:
        return int(v)
    except (TypeError, ValueError):
        return None


def sample():
    """Return (on_ac, capacity_pct, status, tte_seconds_or_None)."""
    on_ac = False
    for name in os.listdir(SYSFS):
        d = os.path.join(SYSFS, name)
        if read(os.path.join(d, "type")) == "Mains":
            if read(os.path.join(d, "online")) == "1":
                on_ac = True
    bat = None
    for name in sorted(os.listdir(SYSFS)):
        d = os.path.join(SYSFS, name)
        if read(os.path.join(d, "type")) == "Battery":
            bat = d
            break
    if bat is None:
        return on_ac, None, "NoBattery", None
    cap = read_int(os.path.join(bat, "capacity"))
    status = read(os.path.join(bat, "status")) or "Unknown"
    tte = None
    energy_now = read_int(os.path.join(bat, "energy_now"))
    power_now = read_int(os.path.join(bat, "power_now"))
    if energy_now is not None and power_now:
        tte = energy_now * 3600 // power_now
    else:
        charge_now = read_int(os.path.join(bat, "charge_now"))
        current_now = read_int(os.path.join(bat, "current_now"))
        if charge_now is not None and current_now:
            tte = charge_now * 3600 // current_now
    return on_ac, cap, status, tte


class Inhibitor:
    def __init__(self):
        self.proc = None

    def engage(self):
        if self.proc is not None:
            return
        if NO_INHIBIT:
            log("INHIBIT(block sleep) [stubbed]")
            self.proc = "stub"
            return
        self.proc = subprocess.Popen(
            [
                "systemd-inhibit",
                "--what=sleep",
                "--mode=block",
                "--who=power-guard",
                "--why=battery reading unconfirmed; power-guard is evaluating",
                "sleep",
                "infinity",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        log("INHIBIT(block sleep) engaged")

    def release(self):
        if self.proc is None:
            return
        if self.proc != "stub":
            self.proc.terminate()
        self.proc = None
        log("INHIBIT released")

    @property
    def held(self):
        return self.proc is not None


def main():
    inh = Inhibitor()
    critical_since = None
    edge_since = None
    log(
        f"started: poll={POLL}s low={LOW_PCT}% crit={CRITICAL_PCT}% "
        f"edge={EDGE_PCT}% confirm={CONFIRM}s edge_confirm={EDGE_CONFIRM}s "
        f"sysfs={SYSFS} no_inhibit={NO_INHIBIT}"
    )
    while True:
        on_ac, cap, status, tte = sample()
        now = time.monotonic()
        discharging = (not on_ac) and status == "Discharging" and cap is not None
        low = discharging and (cap <= LOW_PCT or (tte is not None and tte <= 900))
        critical = discharging and (
            cap <= CRITICAL_PCT or (tte is not None and tte <= CRITICAL_TTE)
        )
        edge = discharging and cap <= EDGE_PCT

        if not low:
            if inh.held:
                log(
                    f"recovered (ac={on_ac} cap={cap} status={status} tte={tte}); releasing"
                )
            inh.release()
            critical_since = edge_since = None
        else:
            inh.engage()
            critical_since = critical_since if critical else None
            if critical and critical_since is None:
                critical_since = now
                log(f"CRITICAL state entered (cap={cap} tte={tte}); confirming")
            edge_since = edge_since if edge else None
            if edge and edge_since is None:
                edge_since = now
                log(f"EDGE state entered (cap={cap}); confirming")
            fire = None
            if edge_since is not None and now - edge_since >= EDGE_CONFIRM:
                fire = f"edge sustained {now - edge_since:.0f}s at cap={cap}"
            elif critical_since is not None and now - critical_since >= CONFIRM:
                fire = f"critical sustained {now - critical_since:.0f}s at cap={cap} tte={tte}"
            if fire:
                log(f"CONFIRMED: {fire}; releasing inhibitor and sleeping")
                inh.release()
                subprocess.run(SLEEP_CMD, shell=True)
                log("sleep command issued; cooling down")
                critical_since = edge_since = None
                time.sleep(120)
        time.sleep(POLL)


if __name__ == "__main__":
    sys.exit(main())
