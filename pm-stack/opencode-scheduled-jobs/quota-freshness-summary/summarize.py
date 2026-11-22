#!/usr/bin/env python3
"""Create a credential-free status view of quota-current.json for an LLM job."""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

MAX_AGE_SECONDS = 30 * 60


def parse_time(value: Any) -> datetime:
    if not isinstance(value, str):
        raise ValueError("timestamp is missing")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("timestamp has no timezone")
    return parsed.astimezone(timezone.utc)


def _valid_heartbeat(value: Any, now: datetime) -> dict[str, Any] | None:
    if not isinstance(value, dict) or not isinstance(value.get("ok"), bool):
        return None
    try:
        tick_at = parse_time(value.get("tickAt"))
        duration = value.get("durationMs")
        if (
            (now - tick_at).total_seconds() < -300
            or isinstance(duration, bool)
            or not isinstance(duration, (int, float))
            or duration < 0
        ):
            return None
        if not isinstance(value.get("perProvider"), dict):
            return None
        return {**value, "_parsedTickAt": tick_at}
    except (TypeError, ValueError):
        return None


def build_report(
    source: dict[str, Any], now: datetime, standalone_heartbeat: Any = None
) -> dict[str, Any]:
    generated_at = parse_time(source.get("generatedAt"))
    age_seconds = int((now - generated_at).total_seconds())
    embedded = _valid_heartbeat(source.get("heartbeat"), now)
    standalone = _valid_heartbeat(standalone_heartbeat, now)
    heartbeat = (
        standalone
        if standalone is not None
        and (
            embedded is None or standalone["_parsedTickAt"] > embedded["_parsedTickAt"]
        )
        else embedded
    ) or {}
    heartbeat_at = heartbeat.get("_parsedTickAt")
    heartbeat_age = int((now - heartbeat_at).total_seconds()) if heartbeat_at else None

    providers = source.get("providers")
    if not isinstance(providers, list):
        raise ValueError("providers is not an array")

    rows = []
    for provider in providers:
        if not isinstance(provider, dict):
            continue
        measured_at = provider.get("measuredAt")
        try:
            provider_age = int((now - parse_time(measured_at)).total_seconds())
            provider_fresh = 0 <= provider_age <= MAX_AGE_SECONDS
        except (TypeError, ValueError):
            provider_age = None
            provider_fresh = False
        windows = provider.get("windows")
        safe_windows = []
        if isinstance(windows, list):
            for window in windows:
                if isinstance(window, dict):
                    safe_windows.append(
                        {
                            "name": window.get("name")
                            if isinstance(window.get("name"), str)
                            else "unknown",
                            "state": window.get("state")
                            if window.get("state") in ("ok", "error")
                            else "unknown",
                        }
                    )
        rows.append(
            {
                "provider": provider.get("providerName")
                if isinstance(provider.get("providerName"), str)
                else "unknown",
                "state": provider.get("state")
                if provider.get("state") in ("ok", "error", "lapsed", "unconfigured")
                else "unknown",
                "measuredAt": measured_at,
                "ageSeconds": provider_age,
                "fresh": provider_fresh,
                "stale": provider_age is not None and not provider_fresh,
                "retained": bool(provider.get("retainedFromFailedTick")),
                "windows": safe_windows,
            }
        )

    states = [row["state"] for row in rows]
    return {
        "schemaVersion": 1,
        "observedAt": now.isoformat().replace("+00:00", "Z"),
        "snapshotGeneratedAt": generated_at.isoformat().replace("+00:00", "Z"),
        "snapshotAgeSeconds": age_seconds,
        "snapshotFresh": 0 <= age_seconds <= MAX_AGE_SECONDS,
        "gathererHeartbeatAt": heartbeat.get("tickAt"),
        "gathererHeartbeatAgeSeconds": heartbeat_age,
        "gathererHeartbeatFresh": heartbeat_age is not None
        and 0 <= heartbeat_age <= 20 * 60,
        "gathererHealthy": heartbeat.get("ok") is True and not heartbeat.get("error"),
        "gathererError": bool(heartbeat.get("error")),
        "providerCounts": {
            "total": len(rows),
            "ok": states.count("ok"),
            "error": states.count("error"),
            "lapsed": states.count("lapsed"),
            "unconfigured": states.count("unconfigured"),
            "unknown": states.count("unknown"),
        },
        "providers": rows,
    }


def main() -> int:
    path = Path(
        os.environ.get(
            "QUOTA_CURRENT_PATH",
            str(Path.home() / ".local/state/pm-aggregator/quota-current.json"),
        )
    )
    heartbeat_path = Path(
        os.environ.get(
            "QUOTA_HEARTBEAT_PATH",
            str(path.parent / "quota-gatherer-heartbeat.json"),
        )
    )
    try:
        source = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(source, dict) or source.get("version") != 1:
            raise ValueError("unsupported quota-current document")
        try:
            standalone_heartbeat = json.loads(
                heartbeat_path.read_text(encoding="utf-8")
            )
        except (OSError, json.JSONDecodeError):
            standalone_heartbeat = None
        report = build_report(source, datetime.now(timezone.utc), standalone_heartbeat)
    except (OSError, json.JSONDecodeError, TypeError, ValueError) as error:
        print(f"quota summary input unavailable or invalid: {error}", file=sys.stderr)
        return 2
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
