# if [[ "$REQUEST_METHOD" != "POST" ]]; then
#   # only allow POST to this endpoint
#   return $(status_code 405)
# fi

# echo "0"
# echo ""
# echo "<br>"

# # The following variables are available in all route handlers:

# # REQUEST_METHOD - the HTTP verb used
# echo "REQUEST_METHOD: $REQUEST_METHOD<br>"
# # REQUEST_PATH - the relative path of the request
# echo "REQUEST_PATH: $REQUEST_PATH<br>"
# # REQUEST_QUERY - the raw (unparsed) query string
# echo "REQUEST_QUERY: $REQUEST_QUERY<br>"

# # The framework will automatically parse the request and populate the following associative arrays:

# # HTTP_HEADERS - The parsed request headers
# echo "HTTP_HEADERS:<br>"
# for key in "${!HTTP_HEADERS[@]}"; do
#   echo "  $key: ${HTTP_HEADERS[$key]}<br>"
# done

# # QUERY_PARAMS - The parsed query parameter string
# echo "QUERY_PARAMS:<br>"
# for key in "${!QUERY_PARAMS[@]}"; do
#   echo "  $key: ${QUERY_PARAMS[$key]}<br>"
# done

# # FORM_DATA - The parsed form data (if applicable)
# echo "FORM_DATA:<br>"
# for key in "${!QUERY_PARAMS[@]}"; do
#   echo "  $key: ${QUERY_PARAMS[$key]}<br>"
# done

random() {
  echo $(dd if=/dev/urandom bs=1 count=64 2>/dev/null | xxd -p)
}

for key in "${!FORM_DATA[@]}"; do
  if [[ "$key" == "link" ]]; then
    echo "CHARM_DIR=$CHARM_DIR/$(random)"
    CHARM_DIR=$CHARM_DIR/$(random) ~$USER/code/source/identity/identity charm link ${FORM_DATA[$key]}
    if [[ $? -eq 0 ]]; then
      respond 200 "OK"
    else
      respond 405 "Failure."
    fi
  fi
done

# # PATH_VARS - The parsed variable names for dynamic and catch-all routes
# echo "PATH_VARS:<br>"
# for key in "${!PATH_VARS[@]}"; do
#   echo "  $key: ${PATH_VARS[$key]}<br>"
# done

# # COOKIES - The parsed cookies from the request headers
# echo "COOKIES:<br>"
# for key in "${!COOKIES[@]}"; do
#   echo "  $key: ${COOKIES[$key]}<br>"
# done

# # The following are only used if you are writing an upload handler:

# # FILE_UPLOADS - A mapping of input names -> tmp files
# echo "FILE_UPLOADS:<br>"
# for key in "${!FILE_UPLOADS[@]}"; do
#   echo "  $key: ${FILE_UPLOADS[$key]}<br>"
# done

# # FILE_UPLOAD_TYPES - A mapping of input names -> file upload types (according to the request)
# echo "FILE_UPLOAD_TYPES:<br>"
# for key in "${!FILE_UPLOAD_TYPES[@]}"; do
#   echo "  $key: ${FILE_UPLOAD_TYPES[$key]}<br>"
# done

# # FILE_UPLOAD_NAMES - A mapping of input names -> original filenames
# echo "FILE_UPLOAD_NAMES:<br>"
# for key in "${!FILE_UPLOAD_NAMES[@]}"; do
#   echo "  $key: ${FILE_UPLOAD_NAMES[$key]}<br>"
# done
