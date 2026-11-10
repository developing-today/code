#!/usr/bin/env bash
# Passthrough FUSE probe. Run from the fuse-probe directory. Two modes:
#   A: plain uid 1337 process, mount through the setuid fusermount3
#   B: inside `unshare -Urm`, mount through the kernel directly (no setuid helper)
set -u
here="$(cd "$(dirname "$0")" && pwd)"
PT="$here/pt"
W="$here/work"
rm -rf "$W"; mkdir -p "$W/root-a" "$W/mnt-a" "$W/root-b" "$W/mnt-b"

echo "== A: plain process, uid $(id -u), fusermount3 = $(command -v fusermount3)"
"$PT" "$W/root-a" "$W/mnt-a" & pid=$!
sleep 1
grep "mnt-a" /proc/mounts | sed 's/^/  mount: /'
echo hello-a > "$W/mnt-a/a.txt" && echo "  write via mount: ok"
echo "  read via mount: $(cat "$W/mnt-a/a.txt")"
echo "  underlying root-a: $(cat "$W/root-a/a.txt")"
fusermount3 -u "$W/mnt-a" && echo "  fusermount3 -u: ok"
wait "$pid" 2>/dev/null; echo "  daemon exit: $?"

echo "== B: inside unshare -Urm (user+mount namespace)"
timeout 60 unshare -Urm sh -c '
  PT="$1"; W="$2"
  "$PT" "$W/root-b" "$W/mnt-b" & pid=$!
  sleep 1
  echo "  uid inside: $(id -u) (outside: $(id -u))"
  grep "mnt-b" /proc/mounts | sed "s/^/  mount: /"
  echo hello-b > "$W/mnt-b/b.txt" && echo "  write via mount: ok"
  echo "  read via mount: $(cat "$W/mnt-b/b.txt")"
  echo "  underlying root-b: $(cat "$W/root-b/b.txt")"
  kill "$pid"; wait "$pid" 2>/dev/null
' sh "$PT" "$W"
echo "  unshare exit: $?"
