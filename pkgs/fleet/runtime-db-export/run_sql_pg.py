#!/usr/bin/env python3
"""Run a SQL file against muse-dev Supabase via ~/tools/supabase-exec.py,
loading the egress proxy from ~/tools/secrets.d/mcpx-daemon.env into the
subprocess env when the caller has none (recorder/fleet-feed pattern).
Usage: run_sql.py <sql-file>   (exit code = supabase-exec exit code)
"""
import os
import subprocess
import sys

SUPABASE_EXEC = os.path.expanduser("~/tools/supabase-exec.py")
PROXY_ENV_FILE = os.path.expanduser("~/tools/secrets.d/mcpx-daemon.env")
PROXY_KEYS = ("HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy",
              "ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy")


def flush_env():
    env = dict(os.environ)
    if not (env.get("HTTPS_PROXY") or env.get("https_proxy")):
        try:
            with open(PROXY_ENV_FILE) as f:
                for line in f:
                    k, sep, v = line.strip().partition("=")
                    if sep and k in PROXY_KEYS and k not in env:
                        env[k] = v.strip().strip('"').strip("'")
        except OSError:
            pass
    return env


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    p = subprocess.run([sys.executable, SUPABASE_EXEC, sys.argv[1]],
                       env=flush_env(), timeout=300)
    sys.exit(p.returncode)


if __name__ == "__main__":
    main()
