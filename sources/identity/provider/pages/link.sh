if [[ "$REQUEST_METHOD" != "POST" ]]; then
  return $(status_code 405)
else
for key in "${!FORM_DATA[@]}"; do
  if [[ "$key" == "keys" ]]; then
    KEYS="${FORM_DATA[$key]}"
    break
  fi
done
if [[ -z "$KEYS" ]]; then
  return $(status_code 405)
fi
random() {
  dd if=/dev/urandom bs=1 count="${1:-16}" 2>/dev/null | xxd -p | tr -d '[:space:]'
}
RANDOM_ID=$(random)
IDENTITY_DIR="$(realpath ~)/code/src/identity"
CHARM_DIR="$IDENTITY_DIR/data/charm/link/$RANDOM_ID"
mkdir -p "$CHARM_DIR"
LINK_CODE_PATH=$CHARM_DIR/.link
rm -rf "$LINK_CODE_PATH"
mkdir -p "$(dirname "$LINK_CODE_PATH")"
if [[ -z "$BACKGROUND_JOB_DIR" ]]; || [[ ! -d "$BACKGROUND_JOB_DIR" ]]; then
  echo "Background job directory not found = $BACKGROUND_JOB_DIR" >&2
  return $(status_code 405)
fi
BACKGROUND_JOB_PATH="$BACKGROUND_JOB_DIR/$RANDOM_ID.sh"
echo "background job path = $BACKGROUND_JOB_PATH ; link code path = $LINK_CODE_PATH" >&2
cat << EOF > "$BACKGROUND_JOB_PATH"
#!/usr/bin/env bash
CHARM_DIR="$CHARM_DIR" $IDENTITY_DIR/identity charm link -d -o "$LINK_CODE_PATH" -k "$KEYS"
EOF
max_wait=60
wait_interval=1
elapsed_time=0
while [[ ! -f "$LINK_CODE_PATH" && $elapsed_time -lt $max_wait ]]; do
  sleep $wait_interval
  ((elapsed_time+=wait_interval))
done
if [[ "$elapsed_time" -ge $max_wait ]]; then
  echo "Link code not found = $LINK_CODE_PATH" >&2
  return $(status_code 405)
fi
if [[ ! -f "$LINK_CODE_PATH" ]]; then
  echo "Link code not found = $LINK_CODE_PATH" >&2
  return $(status_code 405)
fi
LINK_CODE=$(cat "$LINK_CODE_PATH")
if [[ -z "$LINK_CODE" ]]; then
  echo "Link code not found = $LINK_CODE_PATH" >&2
  return $(status_code 405)
fi
echo "Obtained charm link code = $LINK_CODE" >&2
respond 200 "$LINK_CODE"
fi