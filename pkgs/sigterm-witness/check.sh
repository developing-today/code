#!/usr/bin/env bash
# Behavioural check for sigterm-witness (#10386). Usage: check.sh [path-to-binary]
# Oracles: (1) every distinct caught signal in a burst reaches the child,
# even when receipts are dropped; (2) an unwritable log yields a stderr
# diagnostic and the child still gets the signal; (3) dash-led tokens
# before the command are usage errors (exit 64) with no log written;
# dash-led commands after an explicit -- are exec'd; (4) with SIGCHLD
# ignored on entry, the child's real exit code still passes through and
# the child still inherits the ignored SIGCHLD (#15781).
set -u
bin="${1:-}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
if [ -z "$bin" ]; then
  cc -O2 -Wall -o "$tmp/sigterm-witness" "$(dirname "$0")/sigterm-witness.c" || exit 1
  bin="$tmp/sigterm-witness"
fi
fail=0
ok() { echo "ok   - $1"; }
bad() {
  echo "FAIL - $1"
  fail=1
}

# (3) argument rules
"$bin" --log "$tmp/a.log" -x -- true 2>/dev/null
rc=$?
[ "$rc" = 64 ] && [ ! -e "$tmp/a.log" ] && ok "single-dash option rejected pre-fork (exit 64, no log)" || bad "single-dash option rc=$rc"
"$bin" --log "$tmp/b.log" -- -definitely-not-a-command 2>/dev/null
rc=$?
[ "$rc" = 127 ] && ok "dash-led command after -- is exec'd (127: not found)" || bad "after-- rc=$rc"
"$bin" --log "$tmp/c.log" -- sh -c 'exit 3'
rc=$?
[ "$rc" = 3 ] && ok "child exit code passes through" || bad "exit passthrough rc=$rc"

# (4) SIGCHLD ignored on entry (#15781): the wrapper must still read the
# child's status, never log a guessed code=0, and leave the child's
# inherited disposition alone. SigIgn bit 17 (0x10000) is SIGCHLD.
ilog="$tmp/ign.log"
sh -c "trap '' CHLD; exec \"\$0\" --log \"\$1\" -- sh -c 'exit 7'" "$bin" "$ilog"
rc=$?
[ "$rc" = 7 ] && ok "SIGCHLD ignored: child exit 7 passes through" || bad "SIGCHLD ignored: rc=$rc (want 7)"
grep -q "child-exit code=7 " "$ilog" 2>/dev/null && ok "SIGCHLD ignored: log records code=7" || bad "SIGCHLD ignored: log lacks code=7: $(cat "$ilog" 2>/dev/null)"
grep -q "child-exit code=0 " "$ilog" 2>/dev/null && bad "SIGCHLD ignored: log claims code=0" || ok "SIGCHLD ignored: no guessed code=0"
sigign="$(sh -c "trap '' CHLD; exec \"\$0\" --log \"\$1\" -- sh -c 'sed -n \"s/^SigIgn:[[:space:]]*//p\" /proc/\$\$/status'" "$bin" "$tmp/ign2.log")"
if [ -n "$sigign" ] && [ $((0x$sigign & 0x10000)) -ne 0 ]; then
  ok "SIGCHLD ignored: child still inherits SIG_IGN"
else
  bad "SIGCHLD ignored: child SigIgn=$sigign lacks SIGCHLD"
fi

# (2) unwritable log: diagnostic on stderr, child still runs and gets TERM
marker="$tmp/unwritable.marker"
err="$tmp/unwritable.err"
"$bin" --log "$tmp/no-such-dir/x.log" -- sh -c "trap 'echo TERM >> $marker; exit 0' TERM; while :; do sleep 0.05; done" 2>"$err" &
wp=$!
sleep 0.5
kill -TERM "$wp"
wait "$wp"
grep -q "cannot open log" "$err" && ok "unwritable log prints stderr diagnostic" || bad "no stderr diagnostic"
grep -q TERM "$marker" 2>/dev/null && ok "child still receives signal with unwritable log" || bad "child missed TERM with unwritable log"

# (1) burst of distinct signals; every distinct signal must reach the child
burst="$tmp/burst.marker"
blog="$tmp/burst.log"
"$bin" --log "$blog" -- sh -c "trap 'echo USR1 >> $burst' USR1; trap 'echo USR2 >> $burst' USR2; trap 'echo HUP >> $burst' HUP; trap 'echo TERM >> $burst; exit 0' TERM; while :; do sleep 0.05; done" &
wp=$!
sleep 0.5
for _ in $(seq 1 400); do
  kill -USR1 "$wp"
  kill -USR2 "$wp"
  kill -HUP "$wp"
done
sleep 0.5
kill -TERM "$wp"
wait "$wp"
for s in USR1 USR2 HUP TERM; do
  grep -q "^$s\$" "$burst" 2>/dev/null && ok "burst: child received $s" || bad "burst: child missed $s"
done
grep -q "signal-events-dropped" "$blog" 2>/dev/null && echo "note - receipt drop path exercised during burst" || echo "note - burst did not overflow the self-pipe on this host"

exit "$fail"
