#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2154
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
echo "$0 $*"
usage() {
  echo "usage: $0 [--platform p] [--app a] [--static] [--skip-run]"
  exit 1
}
trap 'echo aborted; exit 1' INT ERR
DEFAULT_PLATFORM="go-basic-cli"
platform="${PLATFORM:-$DEFAULT_PLATFORM}"
DEFAULT_APPLICATION="hello"
application="${APPLICATION:-$DEFAULT_APPLICATION}"
static=0
skip_run=0
eval set -- "$(getopt -o "" --long platform:,app:,static,skip-run,help -- "$@")"
if [[ $# -gt 1 ]]; then
  while true; do
    case ${1:""} in
    --platform)
      platform="$2"
      shift 2
      ;;
    --application)
      application="$2"
      shift 2
      ;;
    --app)
      application="$2"
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
      if [[ -z "$1" ]]; then
        usage
      fi
      application="$1"
      shift
      if [[ "$#" -lt 1 ]]; then
        break
      fi
      platform="$1"
      shift
      break
      ;;
    esac
  done
fi
platform="${platform:-$DEFAULT_PLATFORM}"
application="${application:-$DEFAULT_APPLICATION}"
platform_path="${PLATFORM_PATH:-./platforms/${platform}}"
app_path="${APPLICATION_PATH:-./applications/${application}/${platform}}"
if [[ ! -d $app_path ]]; then
  echo "missing dir: $app_path"
  exit 1
fi
platform_roc_path="$platform_path"
if [[ -d "$platform_path/platform" ]]; then
  platform_roc_path="$platform_path/platform"
fi
if [[ -d "$platform_path" ]] && [[ ! -d "$app_path/Platform" ]]; then
  if [[ -L "$app_path/Platform" ]]; then
    unlink "$app_path/Platform"
  fi
  ln -s "../../../$platform_roc_path" "$app_path/Platform"
fi
if [[ ! -d "$app_path/Lib" ]]; then
  if [[ -L "$app_path/Lib" ]]; then
    unlink "$app_path/Lib"
  fi
  ln -s ../../../lib "$app_path/Lib"
fi
app_lib="$app_path/libapp.so"
rm -f "$app_lib" 2>/dev/null || true
app_main="$app_path/main.roc"
# --linker=legacy
roc build --lib "$app_main" --output "$app_lib"
if [[ -d "$platform_path" ]]; then
  if [[ ! -d "$platform_roc_path/Lib" ]]; then
    if [[ -L "$platform_roc_path/Lib" ]]; then
      unlink "$platform_roc_path/Lib"
    fi
    ln -s ../../../lib "$platform_roc_path/Lib"
  fi
  abs_app_dir="$(realpath "$app_path")"
  pushd "$platform_path" >/dev/null
  jump_start_file="./jump-start.sh"
  if [[ -e "$jump_start_file" ]]; then
    "$jump_start_file"
  fi
  roc_build_file="./build.roc"
  host_bin="$platform_path/dynhost"
  if [[ -e "$roc_build_file" ]]; then
    # nix_file="./flake.nix"
    # if [[ -f "$nix_file" ]] && command -v nix && eval "nix eval --json .#devShell.x86_64-linux >/dev/null 2>&1"; then
    # --linker=legacy
    #   nix develop --command "roc \"$roc_build_file\""
    # else
    # --linker=legacy
    roc "$roc_build_file"
    # fi
    if [[ -d "target/release" ]]; then
      host_bin="$platform_path/target/release/host"
    fi
  elif [[ $platform_path == *go-* ]] || [[ $platform_path == *-go ]]; then
    # TODO: build.roc?
    rm -f "$host_bin" 2>/dev/null || true
    rel_app_dir="../applications/${application}/${platform}"
    export CGO_LDFLAGS="-L${abs_app_dir} -Wl,-rpath,'\$ORIGIN/${rel_app_dir}'"
    ldflags=()
    ((static)) && ldflags=(-ldflags "-extldflags=-static")
    go build -buildmode=pie "${ldflags[@]}" -o "$(basename "$host_bin")"
    unset CGO_LDFLAGS
  fi
  popd >/dev/null
  host_main="$platform_roc_path/main.roc"
  # --linker=legacy
  roc preprocess-host "$host_bin" "$host_main" "$app_lib"
fi
# --linker=legacy
((skip_run)) || roc "$app_main"
