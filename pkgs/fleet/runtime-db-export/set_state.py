#!/usr/bin/env python3
"""Record a unit's outcome in state.json AND agent.mirror_export_state.
Usage: set_state.py <table_name> <watermark> <row_count> <ok|error> [error_text]
"""
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
STATE = os.path.join(HERE, "state.json")
LOADER = os.path.join(HERE, "load_rows.py")


def main():
    table, watermark, count, status = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
    error = sys.argv[5] if len(sys.argv) > 5 else None
    now = datetime.now(timezone.utc).isoformat()
    with open(STATE) as f:
        state = json.load(f)
    state["tables"][table] = {"watermark": watermark, "last_run_at": now,
                              "last_row_count": count, "last_status": status,
                              "last_error": error}
    with open(STATE, "w") as f:
        json.dump(state, f, indent=1)
    spec = {"table": "agent.mirror_export_state", "conflict": ["table_name"],
            "mode": "update", "no_exported_at": True,
            "rows": [{"table_name": table, "watermark": watermark,
                      "last_run_at": now, "last_row_count": count,
                      "last_status": status, "last_error": error}]}
    p = subprocess.run([sys.executable, LOADER], input=json.dumps(spec), text=True)
    sys.exit(p.returncode)


if __name__ == "__main__":
    main()
