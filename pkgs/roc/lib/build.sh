#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2154
set -exuo pipefail
echo "$0 $*"
usage() {
  echo "usage: $0 [--platform p] [--app a] [--static] [--skip-run]"
  exit 1
}
trap 'echo aborted; exit 1' INT ERR
platform=${PLATFORM:-go}
application=${APPLICATION:-hello}
static=0
skip_run=0
eval set -- "$(getopt -o "" --long platform:,app:,static,skip-run,help -- "$@")"
if [[ $# -gt 1 ]]; then
  while true; do
    case ${1:""} in
    --platform)
      platform=$2
      shift 2
      ;;
    --application)
      application=$2
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
      # break
      ;;
    # *) usage ;;
    *)
      if [ -z "$1" ]; then
        usage
      fi
      application=$1
      shift
      if [ "$#" -lt 1 ]; then
        break
      fi
      platform=$1
      shift
      break
      ;;
    esac
  done
fi
platform=${platform:-go}
application=${application:-hello}
platform_path=${PLATFORM_PATH:-platforms/${platform}}
app_path=${APPLICATION_PATH:-applications/${application}/${platform}}
if [[ ! -d $app_path ]]; then
  echo "missing dir: $app_path"
  exit 1
fi
if [ ! -d "$app_path/Lib" ]; then
  ln -s ../../../lib "$app_path/Lib"
fi
app_main=$app_path/main.roc
app_lib=$app_path/libapp.so
rm -f "$app_lib" 2>/dev/null || true
roc build --lib "$app_main" --output "$app_lib"
abs_app_dir=$(realpath "$app_path")
rel_app_dir="../applications/${application}/${platform}"
if [[ -d "$platform_path" ]]; then
  if [ ! -d "$platform_path/Lib" ]; then
    ln -s ../../lib "$platform_path/Lib"
  fi
  host_main=$platform_path/main.roc
  host_bin=$platform_path/dynhost
  rm -f "$host_bin" 2>/dev/null || true
  pushd "$platform_path" >/dev/null
  export CGO_LDFLAGS="-L${abs_app_dir} -Wl,-rpath,'\$ORIGIN/${rel_app_dir}'"
  ldflags=()
  ((static)) && ldflags=(-ldflags "-extldflags=-static")
  go build -buildmode=pie "${ldflags[@]}" -o "$(basename "$host_bin")"
  popd >/dev/null
  unset CGO_LDFLAGS
  roc preprocess-host "$host_bin" "$host_main" "$app_lib"
fi
((skip_run)) || roc "$app_main"
