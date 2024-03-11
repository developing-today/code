if [[ "$REQUEST_METHOD" != "POST" ]]; then
  return $(status_code 405)
fi
random() {
  echo $(dd if=/dev/urandom bs=1 count=64 2>/dev/null | xxd -p)
}
for key in "${!FORM_DATA[@]}"; do
  if [[ "$key" == "link" ]]; then
    echo "CHARM_DIR=$CHARM_DIR/$(random)"
    CHARM_DIR=$CHARM_DIR/$(random) ~$USER/code/source/identity/identity charm link ${FORM_DATA[$key]}
    if [[ $? -eq 0 ]]; then
      respond 200 "${FORM_DATA[$key]}"
    else
      respond 405 "Failure."
    fi
  fi
done
