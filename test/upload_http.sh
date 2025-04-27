#!/bin/bash
# test/upload_http.sh

# Single configurable variable for the input file
INPUT_FILE="${1:-samples/divess.yaml}" # Default to samples/divess.yaml if no argument provided

# Configuration
HOST="127.0.0.1:9090" # HTTP server address
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
  echo -e "${BLUE}  Testing HTTP Upload Service${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Check for curl and jq
check_dependencies() {
  if ! command -v curl &>/dev/null; then
    echo -e "${RED}Error: curl is not installed${NC}"
    echo "Please install curl to run this test"
    exit 1
  fi

  if ! command -v jq &>/dev/null; then
    echo -e "${RED}Error: jq is not installed${NC}"
    echo "Please install jq to run this test"
    exit 1
  fi
}

# Test uploading endpoints
test_upload_endpoints() {
  echo -e "${YELLOW}Testing: Upload API Configuration via HTTP${NC}"
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

  response=$(curl -s -X POST \
    -H "Content-Type: application/json" \
    -d "$REQUEST_PAYLOAD" \
    "$HOST/api/upload")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"

  # Check if the upload was successful
  if echo "$response" | jq -e '.success == true' >/dev/null; then
    echo -e "${GREEN}Upload successful!${NC}"
    echo -e "Imported ${GREEN}$(echo "$response" | jq '.imported_count')${NC} endpoints in ${GREEN}$(echo "$response" | jq '.group_count')${NC} groups."
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
