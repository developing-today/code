#!/usr/bin/env bash
set -Eeuo pipefail

repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/bin" "$tmp_dir/run/logs/first/bad" "$tmp_dir/run/logs/first/good"
printf 'keep me\n' >"$tmp_dir/run/logs/first/bad/source.txt"
printf 'archive me\n' >"$tmp_dir/run/logs/first/good/source.txt"

cat >"$tmp_dir/bin/tar" <<'FAKE_TAR'
#!/usr/bin/env bash
set -euo pipefail
archive="$2"
folder="${!#}"
printf 'partial\n' > "$archive"
if [[ "$folder" == bad ]]; then
  exit 1
fi
FAKE_TAR
chmod +x "$tmp_dir/bin/tar"

cp "$repo_dir/compress_logs.sh" "$tmp_dir/run/compress_logs.sh"
mkdir -p "$tmp_dir/run/logs"

PATH="$tmp_dir/bin:$PATH" bash "$tmp_dir/run/compress_logs.sh" >"$tmp_dir/output.log" 2>&1

test -d "$tmp_dir/run/logs/first/bad"
test -f "$tmp_dir/run/logs/first/bad/source.txt"
test ! -e "$tmp_dir/run/logs/first/bad.tar.gz"
test ! -d "$tmp_dir/run/logs/first/good"
test -f "$tmp_dir/run/logs/first/good.tar.gz"
test ! -e "$tmp_dir/run/.lock"
grep -q 'Failed to compress first/bad' "$tmp_dir/output.log"
grep -q 'Successfully compressed first/good' "$tmp_dir/output.log"

echo 'compress_logs failure-path test passed'
