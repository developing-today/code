#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2154
set -exuo pipefail

usage() {
  echo "usage: $0 [--platform p] [--app a] [--static] [--skip-run]"
  exit 1
}

platform=${PLATFORM:-go}
application=${APPLICATION:-hello}
static=0
skip_run=0

eval set -- "$(getopt -o "" --long platform:,app:,static,skip-run,help -- "$@")"
while true; do
  case ${1:---} in
  --platform)
    platform=$2
    shift 2
    ;;
  --app)
    application=$2
    shift 2
    ;;
  --static)
    static=1
    shift
    ;;
  --skip-run)
    skip_run=1
    shift
    ;;
  --help) usage ;;
  --)
    shift
    ;;
  # *) usage ;;
  *)
    if [ -z "$1" ]; then
      usage
    fi
    application=$1
    if [ "$#" -lt 2 ]; then
      break
    fi
    platform=$1
    break
    ;;
  esac
done
platform=${platform:-go}
application=${application:-hello}

platform_path=${PLATFORM_PATH:-platforms/${platform}}
app_path=${APPLICATION_PATH:-applications/${application}/${platform}}

[[ -d $platform_path && -d $app_path ]] ||
  {
    echo "missing dirs: $platform_path or $app_path"
    exit 1
  }
if [ ! -d "$platform_path/Lib" ]; then
  ln -s ../../lib "$platform_path/Lib"
fi
if [ ! -d "$app_path/Lib" ]; then
  ln -s ../../../lib "$app_path/Lib"
fi
app_main=$app_path/main.roc
app_lib=$app_path/libapp.so
rm -f "$app_lib" 2>/dev/null || true
host_main=$platform_path/main.roc
host_bin=$platform_path/dynhost
rm -f "$host_bin" 2>/dev/null || true

trap 'echo aborted; exit 1' INT ERR

roc build --lib "$app_main" --output "$app_lib"

# ------------ key fixes -----------------
abs_app_dir=$(realpath "$app_path")                      # compute BEFORE pushd
rel_app_dir="../applications/${application}/${platform}" # runtime rpath
# ----------------------------------------

pushd "$platform_path" >/dev/null

export CGO_LDFLAGS="-L${abs_app_dir} -Wl,-rpath,'\$ORIGIN/${rel_app_dir}'"

ldflags=()
((static)) && ldflags=(-ldflags "-extldflags=-static")

go build -buildmode=pie "${ldflags[@]}" -o "$(basename "$host_bin")"
popd >/dev/null
unset CGO_LDFLAGS

roc preprocess-host "$host_bin" "$host_main" "$app_lib"
((skip_run)) || roc "$app_main"
