if [[ "$REQUEST_METHOD" != "POST" ]]; then
  return $(status_code 405)
else
random() {
  dd if=/dev/urandom bs=1 count="${1:-32}" 2>/dev/null | xxd -p | tr -d '[:space:]'
}
for key in "${!FORM_DATA[@]}"; do
  if [[ "$key" == "keys" ]]; then
    KEYS="${FORM_DATA[$key]}"
    break
  fi
done
if [[ -z "$KEYS" ]]; then
  return $(status_code 405)
fi
CHARM_DIR=$(realpath $CHARM_DIR)"/$(random)"
mkdir -p "$CHARM_DIR"
LINK_CODE_PATH=$CHARM_DIR/.link
rm -rf "$LINK_CODE_PATH"
mkdir -p "$(dirname "$LINK_CODE_PATH")"
cat << EOF > "$BACKGROUND_JOB_DIR/$(basename "$(dirname "$CHARM_DIR")").sh"
#!/usr/bin/env bash
CHARM_DIR="$CHARM_DIR" ~/code/src/identity/identity charm link -d -o "$LINK_CODE_PATH" -k "${FORM_DATA[$key]}"
EOF
echo "Created background job: $BACKGROUND_JOB_DIR/$(basename "$(dirname "$CHARM_DIR")").sh" >&2
max_wait=60 # seconds
wait_interval=1 # seconds
elapsed_time=0
while [[ ! -f "$LINK_CODE_PATH" && $elapsed_time -lt $max_wait ]]; do
  sleep $wait_interval
  ((elapsed_time+=wait_interval))
  echo "Waiting for link code: $elapsed_time seconds elapsed" >&2
done
echo "Elapsed time: $elapsed_time" >&2
if [[ "$elapsed_time" -ge $max_wait ]]; then
  return $(status_code 405)
fi
if [[ ! -f "$LINK_CODE_PATH" ]]; then
  return $(status_code 405)
fi
LINK_CODE=$(cat "$LINK_CODE_PATH")
if [[ -z "$LINK_CODE" ]]; then
  return $(status_code 405)
fi
echo "Obtained charm link code: $LINK_CODE" >&2
respond 200 "$LINK_CODE"
fi
echo "Done" >&2
