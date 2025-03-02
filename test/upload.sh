#!/bin/bash

HOST="0.0.0.0:50055"
EMAIL="mohamed.bennekrouf@gmail.com"
FILE_PATH="../semantic/samples/divess.yaml"

# Test uploading endpoints
echo "Testing endpoint upload..."
echo "-----------------"

# Read file content and encode as base64
# FILE_CONTENT=$(base64 -w 0 "$FILE_PATH")

FILE_CONTENT=$(base64 <"$FILE_PATH" | tr -d '\n')
REQUEST_PAYLOAD=$(
  cat <<EOF
{
    "email": "$EMAIL",
    "file_content": "$FILE_CONTENT",
    "file_name": "$FILE_PATH"
}
EOF
)

echo "Request payload:"
echo "$REQUEST_PAYLOAD"
echo "-----------------"

response=$(grpcurl -plaintext \
  -d "$REQUEST_PAYLOAD" \
  $HOST \
  endpoint.EndpointService/UploadEndpoints)

echo "Response:"
echo "$response"
echo "-----------------"

GET_REQUEST_PAYLOAD=$(
  cat <<EOF
{
    "email": "$EMAIL"
}
EOF
)
