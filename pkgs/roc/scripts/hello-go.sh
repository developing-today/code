#!/usr/bin/env bash
set -euo pipefail

default_platform=go
default_application=hello

application="${1:-${APPLICATION:-$default_application}}"
platform="${2:-${PLATFORM:-$default_platform}}"

platform_path="platforms/${platform}-platform"
application_path="${platform_path}/applications/${application}-${platform}"

[[ -d "$platform_path" ]] || {
  echo "missing $platform_path"
  exit 1
}
[[ -d "$application_path" ]] || {
  echo "missing $application_path"
  exit 1
}

application_main="${application_path}/main.roc"
application_output="${application_path}/libapp.so"
platform_main="${platform_path}/main.roc"
platform_output="${platform_path}/dynhost"

roc build --lib "$application_main" --output "$application_output"

pushd "$platform_path" >/dev/null
export CGO_LDFLAGS="-L./applications/${application}-${platform} \
  -Wl,-rpath,'\$ORIGIN/applications/${application}-${platform}'"
go build -buildmode=pie -o "$(basename "$platform_output")"
popd >/dev/null
unset CGO_LDFLAGS

roc preprocess-host "$platform_output" "$platform_main" "$application_output"
roc "$application_main"
# #!/usr/bin/env bash
# set -ex
# DEFAULT_PLATFORM=go
# DEFAULT_APPLICATION=hello
# if [ -z "${PLATFORM}" ]; then
#   PLATFORM="${2:-${DEFAULT_PLATFORM}}"
# fi
# if [ -z "${PLATFORM}" ]; then
#   echo "No platform specified. Usage: $0 <application> [platform]"
#   exit 1
# fi
# if [ -z "${PLATFORM_PATH}" ]; then
#   PLATFORM_PATH="platforms/${PLATFORM}-platform"
# fi
# if [ ! -d "${PLATFORM_PATH}" ]; then
#   echo "Platform directory ${PLATFORM_PATH} does not exist."
#   exit 1
# fi
# if [ -z "${APPLICATION}" ]; then
#   APPLICATION="${1:-${DEFAULT_APPLICATION}}"
# fi
# if [ -z "${APPLICATION}" ]; then
#   echo "No application specified. Usage: $0 [application] [platform]"
#   exit 1
# fi
# if [ -z "${APPLICATION_PATH}" ]; then
#   APPLICATION_PATH="${PLATFORM_PATH}/applications/${APPLICATION}-${PLATFORM}"
# fi
# if [ ! -d "${APPLICATION_PATH}" ]; then
#   echo "Application directory ${APPLICATION_PATH} does not exist."
#   exit 1
# fi
# APPLICATION_MAIN="${APPLICATION_PATH}/main.roc"
# APPLICATION_OUTPUT="${APPLICATION_PATH}/libapp.so"
# PLATFORM_MAIN="${PLATFORM_PATH}/main.roc"
# PLATFORM_OUTPUT="${PLATFORM_PATH}/dynhost"
# roc build \
#   --lib "${APPLICATION_MAIN}" \
#   --output "${APPLICATION_OUTPUT}"
# go build \
#   -C "${PLATFORM_PATH}" \
#   -buildmode=pie \
#   -o "${PLATFORM_OUTPUT}"
# roc preprocess-host \
#   "${PLATFORM_OUTPUT}" \
#   "${PLATFORM_MAIN}" \
#   "${APPLICATION_OUTPUT}"
# roc "${APPLICATION_MAIN}"
