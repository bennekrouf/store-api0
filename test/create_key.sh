#!/bin/bash

# Test script for API key management

# Configuration
HOST="127.0.0.1:9090"                     # HTTP server address
TEST_EMAIL="mohamed.bennekrouf@gmail.com" # Test email
KEY_NAME="Test API Key"                   # Name for the API key

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing API Key Management${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to test generating a new API key
test_generate_key() {
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $KEY_NAME"
  echo "-----------------"

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
  "email": "$TEST_EMAIL",
  "key_name": "$KEY_NAME"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/key")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"

  # Extract and save the API key for later tests
  API_KEY=$(echo "$response" | jq -r '.key')
  if [ "$API_KEY" != "null" ]; then
    echo -e "${GREEN}API Key: $API_KEY${NC}"
    # Save to a temp file for later tests
    echo "$API_KEY" >/tmp/api_key.txt
  else
    echo -e "${RED}Failed to extract API key from response${NC}"
  fi

  echo
}

# Main execution
print_header

# Generate a new API key
echo -e "${BLUE}Generating a new API key:${NC}"
test_generate_key

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key.txt
