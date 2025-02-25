#!/bin/bash

HOST="0.0.0.0:50055"
EMAIL="mohamed.bennekrouf@gmail.com"
FILE_PATH="../semantic/pickbazar_endpoints.yaml"

# Test uploading endpoints
echo "Testing endpoint upload..."
echo "-----------------"

# Read file content and encode as base64
FILE_CONTENT=$(base64 -w 0 "$FILE_PATH")

REQUEST_PAYLOAD=$(cat <<EOF
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

# Test getting updated endpoints
# echo "Verifying uploaded endpoints..."
# echo "-----------------"

GET_REQUEST_PAYLOAD=$(cat <<EOF
{
    "email": "$EMAIL"
}
EOF
)

# response=$(grpcurl -plaintext \
#     -d "$GET_REQUEST_PAYLOAD" \
#     $HOST \
#     endpoint.EndpointService/GetDefaultEndpoints)
#
# echo "Updated endpoints:"
# echo "$response"
