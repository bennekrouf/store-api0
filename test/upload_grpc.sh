#!/bin/bash
# test/upload_grpc.sh

# Single configurable variable for the input file
INPUT_FILE="${1:-samples/divess.yaml}" # Default to samples/divess.yaml if no argument provided

# Configuration
HOST="0.0.0.0:50055" # gRPC server address
EMAIL="mohamed.bennekrouf@gmail.com"

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing gRPC Upload Service${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Check for grpcurl
check_dependencies() {
  if ! command -v grpcurl &>/dev/null; then
    echo -e "${RED}Error: grpcurl is not installed${NC}"
    echo "Please install grpcurl to run this test"
    exit 1
  fi
}

# Test uploading endpoints
test_upload_endpoints() {
  echo -e "${YELLOW}Testing: Upload API Configuration via gRPC${NC}"
  echo "Email: $EMAIL, File: $INPUT_FILE"
  echo "-----------------"

  # Read file content and encode as base64
  FILE_CONTENT=$(base64 <"$INPUT_FILE" | tr -d '\n')

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
    "email": "$EMAIL",
    "file_content": "$FILE_CONTENT",
    "file_name": "$INPUT_FILE"
}
EOF
  )

  echo "Request payload (truncated):"
  echo "{ \"email\": \"$EMAIL\", \"file_name\": \"$INPUT_FILE\", \"file_content\": \"[BASE64 CONTENT]\" }"
  echo "-----------------"

  response=$(grpcurl -plaintext \
    -d "$REQUEST_PAYLOAD" \
    $HOST \
    endpoint.EndpointService/UploadApiGroups)

  echo "Response:"
  echo "$response"
  echo "-----------------"

  # Check if the upload was successful
  if echo "$response" | grep -q '"success": true'; then
    echo -e "${GREEN}Upload successful!${NC}"
  else
    echo -e "${RED}Upload failed!${NC}"
  fi

  echo
}

# Main execution
print_header
check_dependencies

# Test uploading with the specified file
test_upload_endpoints

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
