if [[ "$REQUEST_METHOD" != "POST" ]]; then
  return $(status_code 405)
fi
random() {
  dd if=/dev/urandom bs=1 count="${1:-32}" 2>/dev/null | xxd -p | tr -d '[:space:]'
}
for key in "${!FORM_DATA[@]}"; do
  if [[ "$key" == "keys" ]]; then
    CHARM_DIR=$CHARM_DIR/$(random)
    mkdir -p "$CHARM_DIR"
    LINK_CODE_PATH=$CHARM_DIR/.link
    rm -rf "$LINK_CODE_PATH"

mkdir -p "$(dirname "$LINK_CODE_PATH")"

~/code/src/identity/identity charm link -d -o "$LINK_CODE_PATH" -k "${FORM_DATA[$key]}" > /dev/null 2>&1
max_wait=60 # seconds
wait_interval=1 # seconds
elapsed_time=0

while [[ ! -f "$LINK_CODE_PATH" && $elapsed_time -lt $max_wait ]]; do
  sleep $wait_interval
  ((elapsed_time+=wait_interval))
done

if [[ -f "$LINK_CODE_PATH" ]]; then
  LINK_CODE=$(cat "$LINK_CODE_PATH")
  if [[ -z "$LINK_CODE" ]]; then
    respond 405 "Failure."
  else
    respond 200 "$LINK_CODE"
  fi
else
  respond 405 "Failure."
fi
  fi
done
