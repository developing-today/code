set +x
if [[ $REQUEST_METHOD != "POST" ]]; then
  echo "Invalid request method = $REQUEST_METHOD" >&2
  return $(status_code 405)
else
  set -ex
  echo "OPEN" >&2
  if [[ -z $INIT_URL ]]; then
    echo "Init URL not found" >&2
    return $(status_code 405)
  fi
  for key in "${!FORM_DATA[@]}"; do
    if [[ $key == "keys" ]]; then
      KEYS="${FORM_DATA[$key]}"
      break
    fi
  done
  if [[ -z $KEYS ]]; then
    echo "Keys not found" >&2
    return $(status_code 405)
  fi
  random() {
    dd if=/dev/urandom bs=1 count="${1:-16}" 2>/dev/null | xxd -p | tr -d '[:space:]'
  }
  RANDOM_ID=$(random)
  REPO_ROOT=$(git rev-parse --show-toplevel)
  IDENTITY_DIR="$REPO_ROOT/sources/identity"
  CHARM_DATA_DIR="$IDENTITY_DIR/data/charm/provider/$RANDOM_ID"
  mkdir -p "$CHARM_DATA_DIR"
  LINK_PATH="$CHARM_DATA_DIR/.link.$RANDOM_ID.$(date +%s)"
  logged() {
    LINK_CODE_PATH="$LINK_PATH.$(date +%s).code"
    rm -rf "$LINK_CODE_PATH"
    mkdir -p "$(dirname "$LINK_CODE_PATH")"
    INIT_SCRIPT_NAME_PREFIX=".init.$RANDOM_ID.$(date +%s)"
    INIT_SCRIPT_NAME="$INIT_SCRIPT_NAME_PREFIX.sh"
    INIT_PATH="$CHARM_DATA_DIR/$INIT_SCRIPT_NAME"
    cat <<EOF >"$INIT_PATH"
#!/usr/bin/env bash
set -ex
output() {
REPO_ROOT=\$(git rev-parse --show-toplevel)
# CHARM_DATA_DIR="\$REPO_ROOT/sources/identity/data/charm/consumer" ./identity charm kv list @$RANDOM_ID
CHARM_DATA_DIR="\$REPO_ROOT/sources/identity/data/charm/consumer" ./identity charm kv reset @$RANDOM_ID # ?? needed?
# CHARM_DATA_DIR="\$REPO_ROOT/sources/identity/data/charm/consumer" ./identity charm kv list @$RANDOM_ID
TURSO_HOST=\$(CHARM_DATA_DIR="\$REPO_ROOT/sources/identity/data/charm/consumer" ./identity charm kv get dt.identity.secret.TURSO_HOST@$RANDOM_ID)
export TURSO_HOST
if [ -z "\$TURSO_HOST" ]; then
  echo "TURSO_HOST not set"
  exit 1
fi
TURSO_AUTH_TOKEN=\$(CHARM_DATA_DIR="\$REPO_ROOT/sources/identity/data/charm/consumer" ./identity charm kv get dt.identity.secret.TURSO_AUTH_TOKEN@$RANDOM_ID)
export TURSO_AUTH_TOKEN
if [ -z "\$TURSO_AUTH_TOKEN" ]; then
  echo "TURSO_AUTH_TOKEN not set"
  exit 1
fi
./provider.sh "data/charm" "$INIT_URL" "$PORT" &
./start-server-all.ps1
}
output
# 2>&1 | tee -a "$INIT_SCRIPT_NAME_PREFIX.\$(date +%s).log"
EOF
    cat "$INIT_PATH" >&2
    chmod +x "$INIT_PATH"
    echo "\n\n\n=====================\n\n\n" >&2
    CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm id >&2
    # CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list >&2
    # CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list @$RANDOM_ID >&2
    # CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm fs tree "/" >&2 # breaks things???
    echo "\n\n\n=====================\n\n\n" >&2
    BACKGROUND_PATH=$CHARM_DATA_DIR/.background.$RANDOM_ID.$(date +%s)
    BACKGROUND_SCRIPT_PATH="$BACKGROUND_PATH.sh"
    cat <<EOF >"$BACKGROUND_SCRIPT_PATH"
#!/usr/bin/env bash
BACKGROUND_LOG="$BACKGROUND_PATH.\$(date +%s).log"
echo "Logging to \$BACKGROUND_LOG" | tee -a "\$BACKGROUND_LOG"
echo "+ \$0" | tee -a "\$BACKGROUND_LOG"
echo "CHARM_DATA_DIR = $CHARM_DATA_DIR" | tee -a "\$BACKGROUND_LOG"
echo "IDENTITY_DIR = $IDENTITY_DIR" | tee -a "\$BACKGROUND_LOG"
echo "LINK_CODE_PATH = $LINK_CODE_PATH" | tee -a "\$BACKGROUND_LOG"
echo "KEYS = $KEYS" | tee -a "\$BACKGROUND_LOG"
echo "INIT_SCRIPT_NAME = $INIT_SCRIPT_NAME" | tee -a "\$BACKGROUND_LOG"
echo "INIT_PATH = $INIT_PATH" | tee -a "\$BACKGROUND_LOG"
echo "INIT_URL = $INIT_URL" | tee -a "\$BACKGROUND_LOG"
echo "PORT = $PORT" | tee -a "\$BACKGROUND_LOG"
echo "TURSO_HOST = $TURSO_HOST" | tee -a "\$BACKGROUND_LOG"
echo "charm id = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm id)" | tee -a "\$BACKGROUND_LOG"
# echo "charm fs tree = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm fs tree "/")" | tee -a "\$BACKGROUND_LOG"
# echo "charm kv list = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list )" | tee -a "\$BACKGROUND_LOG"
# echo "charm kv list = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list @$RANDOM_ID)" | tee -a "\$BACKGROUND_LOG"
CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm link -d -o "$LINK_CODE_PATH" -k "$KEYS" | tee -a "\$BACKGROUND_LOG"
echo "charm link exit code = \$?" | tee -a "\$BACKGROUND_LOG"
echo "charm id = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm id)" | tee -a "\$BACKGROUND_LOG"
# echo "charm fs tree = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm fs tree "/")" | tee -a "\$BACKGROUND_LOG"
# echo "charm kv list = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list)" | tee -a "\$BACKGROUND_LOG"
# echo "charm kv list = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list @$RANDOM_ID)" | tee -a "\$BACKGROUND_LOG"
echo "charm kv set dt.identity.secret.TURSO_HOST = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv set "dt.identity.secret.TURSO_HOST@$RANDOM_ID" "$TURSO_HOST")" | tee -a "\$BACKGROUND_LOG"
echo "charm kv set dt.identity.secret.TURSO_AUTH_TOKEN = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv set "dt.identity.secret.TURSO_AUTH_TOKEN@$RANDOM_ID" "$TURSO_AUTH_TOKEN")" | tee -a "\$BACKGROUND_LOG"
# echo "charm kv list = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list)" | tee -a "\$BACKGROUND_LOG"
# echo "charm kv list = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm kv list @$RANDOM_ID)" | tee -a "\$BACKGROUND_LOG"
# echo "charm fs tree = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm fs tree "/")" | tee -a "\$BACKGROUND_LOG"
echo "charm fs cp = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm fs cp "$INIT_PATH" "charm:dt/identity/init/init")" | tee -a "\$BACKGROUND_LOG"
# echo "charm fs tree = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm fs tree "/")" | tee -a "\$BACKGROUND_LOG"
echo "charm id = \$(CHARM_DATA_DIR="$CHARM_DATA_DIR" $IDENTITY_DIR/identity charm id)" | tee -a "\$BACKGROUND_LOG"
echo "CLOSE" | tee -a "\$BACKGROUND_LOG"
EOF
    cat "$BACKGROUND_SCRIPT_PATH" >&2
    chmod +x "$BACKGROUND_SCRIPT_PATH"
    </dev/null "$BACKGROUND_SCRIPT_PATH" >/dev/null 2>&1 &
    max_wait=60
    wait_interval=1
    elapsed_time=0
    set +x
    while [[ ! -f $LINK_CODE_PATH && $elapsed_time -lt $max_wait ]]; do
      sleep $wait_interval
      ((elapsed_time += wait_interval))
      echo "Waiting for link code = $LINK_CODE_PATH - Elapsed time = $elapsed_time" >&2
    done
    set -x
    if [[ $elapsed_time -ge $max_wait ]]; then
      echo "Link code not found = $LINK_CODE_PATH" >&2
      return $(status_code 405)
    fi
    if [[ ! -f $LINK_CODE_PATH ]]; then
      echo "Link code not found = $LINK_CODE_PATH" >&2
      return $(status_code 405)
    fi
    LINK_CODE=$(cat "$LINK_CODE_PATH")
    if [[ -z $LINK_CODE ]]; then
      echo "Link code not found = $LINK_CODE_PATH" >&2
      return $(status_code 405)
    else
      echo "Obtained charm link code = $LINK_CODE" >&2
      respond 200 "$LINK_CODE"
    fi
    set +x
    echo "CLOSE" >&2
  }
  logged
fi
