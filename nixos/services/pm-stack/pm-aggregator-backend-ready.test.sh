#!/usr/bin/env bash
# Contract: run with no arguments as `bash nixos/services/pm-stack/pm-aggregator-backend-ready.test.sh`.
# Oracle: the socket proxy must not start until the private backend accepts a
# TCP connection; the readiness helper succeeds only after that connection
# works and times out when the backend stays unavailable.
set -uo pipefail

if [[ $# -ne 0 ]]; then
  echo "usage: bash nixos/services/pm-stack/pm-aggregator-backend-ready.test.sh" >&2
  exit 2
fi

here="$(dirname -- "$(realpath -- "$0")")"
module="$here/default.nix"
helper="$here/pm-aggregator-backend-ready.sh"
failures=0

expect_present() { # expect_present <file> <literal> <description>
  if grep -qF -- "$2" "$1"; then
    echo "PASS: $3"
  else
    echo "FAIL: $3 (pattern not found in $1: $2)"
    failures=$((failures + 1))
  fi
}

expect_present "$module" 'ExecStartPost = lib.optional cfg.aggregatorSocketActivation' \
  "socket-activated aggregator service gates start-complete on backend readiness"
expect_present "$module" 'pm-aggregator-backend-ready.sh ${toString cfg.aggregatorBackendPort} 30' \
  "readiness gate probes the private backend port with a bounded timeout"
expect_present "$module" 'Requires = [ "pm-aggregator.service" ];' \
  "proxy remains ordered after the backend service"

if [[ -f "$helper" ]]; then
  if python3 - "$helper" <<'PY'
import socket
import subprocess
import sys
import threading
import time

helper = sys.argv[1]

def unused_port():
    with socket.socket() as reserved:
        reserved.bind(("127.0.0.1", 0))
        return reserved.getsockname()[1]

port = unused_port()
waiter = subprocess.Popen(["bash", helper, str(port), "4"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
time.sleep(0.25)
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", port))
listener.listen(1)
accepted = threading.Event()

def accept_once():
    connection, _ = listener.accept()
    connection.close()
    accepted.set()

thread = threading.Thread(target=accept_once, daemon=True)
thread.start()
stdout, stderr = waiter.communicate(timeout=5)
listener.close()
thread.join(timeout=1)
if waiter.returncode != 0 or not accepted.is_set():
    print(f"FAIL: helper did not wait for a listening backend (rc={waiter.returncode}, stdout={stdout!r}, stderr={stderr!r})", file=sys.stderr)
    raise SystemExit(1)

closed_port = unused_port()
timed_out = subprocess.run(["bash", helper, str(closed_port), "1"], capture_output=True, text=True, timeout=3)
if timed_out.returncode != 1:
    print(f"FAIL: helper must fail when the backend stays unavailable (rc={timed_out.returncode}, stdout={timed_out.stdout!r}, stderr={timed_out.stderr!r})", file=sys.stderr)
    raise SystemExit(1)

bad_args = subprocess.run(["bash", helper, "70000", "1"], capture_output=True, text=True, timeout=2)
if bad_args.returncode != 2:
    print(f"FAIL: helper must reject an invalid port before probing (rc={bad_args.returncode})", file=sys.stderr)
    raise SystemExit(1)
print("PASS: backend readiness helper waits for accept, times out, and rejects invalid arguments")
PY
  then
    :
  else
    failures=$((failures + 1))
  fi
else
  echo "FAIL: backend readiness helper exists"
  failures=$((failures + 1))
fi

if [[ "$failures" -gt 0 ]]; then
  echo "pm-aggregator-backend-ready.test.sh: $failures check(s) failed" >&2
  exit 1
fi
echo "pm-aggregator-backend-ready.test.sh: all checks passed"
