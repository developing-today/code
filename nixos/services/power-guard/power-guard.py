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

import math
import os
import subprocess
import sys
import threading
import time

SYSFS = os.environ.get("POWER_GUARD_SYSFS", "/sys/class/power_supply")

# Largest interval time.sleep can represent: C PyTime_t is a signed
# 64-bit count of nanoseconds, so a finite value beyond ~292 years
# still overflows time.sleep at runtime (OverflowError) mid-loop.
MAX_TIMING_SECONDS = (2**63 - 1) / 1e9


def parse_timing(name, raw):
    """Parse one timing override (seconds).

    Invariant (the daemon's reason to exist, #1687/#6477): a timing
    override is only meaningful as a finite, strictly positive number
    of seconds no larger than MAX_TIMING_SECONDS (the largest interval
    time.sleep can represent). Zero or negative confirmation would fire
    on the first sample; a non-positive poll cannot pace the loop.
    Raises ValueError naming the variable and the offending value.
    """
    try:
        value = float(raw)
    except (TypeError, ValueError):
        raise ValueError(
            f"{name}={raw!r} is not a number; "
            "it must be a finite positive number of seconds"
        ) from None
    if not math.isfinite(value) or value <= 0:
        raise ValueError(
            f"{name}={raw!r} must be a finite positive number of seconds"
        )
    if value > MAX_TIMING_SECONDS:
        raise ValueError(
            f"{name}={raw!r} exceeds the largest interval time.sleep "
            f"can represent ({MAX_TIMING_SECONDS:.0f} seconds)"
        )
    return value


def timing_from_env(name, default):
    """Read a timing override, failing closed at startup on bad input.

    On an invalid value, emit one named error (no traceback) and exit
    non-zero before any polling, so a systemd unit shows as failed
    instead of crash-looping or silently running unguarded.
    """
    raw = os.environ.get(name, default)
    try:
        return parse_timing(name, raw)
    except ValueError as exc:
        print(f"power-guard: error: {exc}", file=sys.stderr, flush=True)
        raise SystemExit(2) from None

POLL = timing_from_env("POWER_GUARD_POLL", "15")
LOW_PCT = 20  # engage the block at/below this (UPower PercentageLow)
CRITICAL_PCT = 5  # UPower PercentageCritical
EDGE_PCT = 2  # UPower PercentageAction
CRITICAL_TTE = 300  # seconds-to-empty treated as critical
CONFIRM = timing_from_env("POWER_GUARD_CONFIRM", "300")
EDGE_CONFIRM = timing_from_env("POWER_GUARD_EDGE_CONFIRM", "60")
NO_INHIBIT = os.environ.get("POWER_GUARD_NO_INHIBIT") == "1"
SLEEP_CMD = os.environ.get("POWER_GUARD_SLEEP_CMD", "systemctl hybrid-sleep")
# #8183: how long release() may wait inline for a terminated child
# before the reap moves to a background thread. The full reap used
# to block the sampling loop for up to ~10s (wait 5s, kill, wait 5s).
REAP_GRACE = 0.5


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
        # #8183: in-flight background reap, {"proc", "event", "rc"}.
        self._pending = None

    def _reap(self, proc, timeout=5):
        """Wait for a child to exit so it does not remain a zombie.

        The child is already dead or has just been terminated; wait()
        returns immediately in the common case. On timeout, escalate
        to kill() so a wedged child cannot accumulate.
        """
        try:
            return proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            try:
                proc.kill()
            except Exception:
                pass
            try:
                return proc.wait(timeout=timeout)
            except Exception:
                return proc.poll()
        except Exception:
            try:
                return proc.poll()
            except Exception:
                return None

    def _dead_exit_code(self):
        """Return the exit code if the real child is dead, else None.

        Never raises: a poll failure is treated as unknown, not alive,
        so the caller reaps and re-engages loudly rather than
        believing a lock it cannot verify.
        """
        proc = self.proc
        if proc is None or proc == "stub":
            return None
        try:
            return proc.poll()
        except Exception as exc:
            log(f"INHIBIT child poll failed ({exc!r}); treating as dead")
            return -1

    def engage(self):
        if self._pending is not None:
            if not self._pending["event"].is_set():
                # A previous release is still reaping; the old child
                # may still hold the lock, so do not stack a second
                # inhibitor on top of it (#8183).
                return
            self._pending = None
        if self.proc is not None:
            if self.proc == "stub":
                return
            rc = self._dead_exit_code()
            if rc is None:
                return
            log(f"INHIBIT child died (exit={rc}); re-engaging")
            self._reap(self.proc)
            self.proc = None
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
            # stderr is deliberately discarded (the child is `sleep
            # infinity`, which is silent on success); the exit status
            # is logged loudly on death in engage()/release() so the
            # failure stays diagnosable from the journal.
            stderr=subprocess.DEVNULL,
        )
        log("INHIBIT(block sleep) engaged")

    def release(self):
        """Begin releasing the inhibitor; return True once settled.

        #8183: reaping a stalled child used to block the caller for
        the full wait/kill/wait sequence (~10s) inside the sampling
        loop. A live child now gets a short grace wait (REAP_GRACE)
        inline; if it has not exited by then, the kill-escalation
        reap finishes on a background thread while sampling
        continues. "Settled" means the child was reaped, or the
        escalation was exhausted and logged loudly as UNCONFIRMED --
        the release is never silently believed. The confirmed-sleep
        path in main() waits for settled before issuing sleep.
        """
        if self.proc is None:
            return self.reap_settled()
        if self.proc == "stub":
            self.proc = None
            log("INHIBIT released")
            return True
        proc = self.proc
        self.proc = None
        rc = None
        try:
            rc = proc.poll()
        except Exception:
            rc = None
        if rc is not None:
            self._reap(proc)
            log(f"INHIBIT child already dead (exit={rc}); reaped")
            log("INHIBIT released")
            return True
        try:
            proc.terminate()
        except Exception as exc:
            log(f"INHIBIT terminate failed ({exc!r}); killing")
            try:
                proc.kill()
            except Exception:
                pass
        try:
            rc = proc.wait(timeout=REAP_GRACE)
            log(f"INHIBIT released (child exit={rc})")
            return True
        except Exception:
            pass
        pending = {"proc": proc, "event": threading.Event(), "rc": None}
        self._pending = pending
        threading.Thread(
            target=self._finish_reap, args=(pending,), daemon=True
        ).start()
        log(
            "INHIBIT release pending: stalled child reap moved off "
            "the sampling loop (#8183)"
        )
        return False

    def _finish_reap(self, pending):
        """Background reap: preserve the kill escalation + bounded wait."""
        proc = pending["proc"]
        try:
            proc.kill()
        except Exception:
            pass
        rc = None
        try:
            rc = proc.wait(timeout=5)
        except Exception:
            try:
                rc = proc.poll()
            except Exception:
                rc = None
        pending["rc"] = rc
        if rc is None:
            log(
                "INHIBIT release UNCONFIRMED: child still running "
                "after kill escalation; not believed released (#8183)"
            )
        else:
            log(f"INHIBIT released (child exit={rc}, reaped in background)")
        pending["event"].set()

    def reap_settled(self):
        """True when no background reap is still in flight."""
        return self._pending is None or self._pending["event"].is_set()

    def poll_reap(self):
        """Clear a finished background reap; called once per sample."""
        if self._pending is not None and self._pending["event"].is_set():
            self._pending = None

    @property
    def held(self):
        if self._pending is not None and not self._pending["event"].is_set():
            # Reap in flight: the child may still hold the lock.
            try:
                if self._pending["proc"].poll() is None:
                    return True
            except Exception:
                return True
        if self.proc is None:
            return False
        if self.proc == "stub":
            return True
        try:
            return self.proc.poll() is None
        except Exception:
            return False


def main():
    inh = Inhibitor()
    critical_since = None
    edge_since = None
    sleep_pending = None
    log(
        f"started: poll={POLL}s low={LOW_PCT}% crit={CRITICAL_PCT}% "
        f"edge={EDGE_PCT}% confirm={CONFIRM}s edge_confirm={EDGE_CONFIRM}s "
        f"sysfs={SYSFS} no_inhibit={NO_INHIBIT}"
    )
    while True:
        on_ac, cap, status, tte = sample()
        inh.poll_reap()
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
            sleep_pending = None
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
            if fire and sleep_pending is None:
                # #8183: start the release, but do not issue the
                # sleep command until the release has settled -- a
                # stalled reap no longer blocks this loop, and the
                # samples above keep running while it drains.
                log(f"CONFIRMED: {fire}; releasing inhibitor before sleep")
                sleep_pending = fire
                inh.release()
            if sleep_pending is not None and inh.reap_settled():
                log(f"CONFIRMED: {sleep_pending}; inhibitor release settled; sleeping")
                subprocess.run(SLEEP_CMD, shell=True)
                log("sleep command issued; cooling down")
                critical_since = edge_since = None
                sleep_pending = None
                time.sleep(120)
        time.sleep(POLL)


if __name__ == "__main__":
    sys.exit(main())
