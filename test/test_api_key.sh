#!/bin/bash

# Test script for API key management

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
TEST_EMAIL="test@example.com"  # Test email
KEY_NAME="Test API Key"  # Name for the API key

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

# Function to test getting API key status
test_get_key_status() {
  echo -e "${YELLOW}Testing: Get API Key Status${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/key/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test generating a new API key
test_generate_key() {
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $KEY_NAME"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
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
    echo "$API_KEY" > /tmp/api_key.txt
  else
    echo -e "${RED}Failed to extract API key from response${NC}"
  fi
  
  echo
}

# Function to test an endpoint with API key
test_endpoint_with_key() {
  API_KEY=$(cat /tmp/api_key.txt)
  
  echo -e "${YELLOW}Testing: Access API with Key${NC}"
  echo "API Key: ${API_KEY:0:10}..."
  echo "-----------------"

  response=$(curl -s -X GET -H "X-API-Key: $API_KEY" "$HOST/api/groups/$TEST_EMAIL")
  
  echo "Response (truncated):"
  echo "$response" | jq '.success, .message, (.api_groups | length) as $len | "Number of groups: \($len)"'
  echo "-----------------"
  echo
}

# Function to test getting API key usage
test_get_key_usage() {
  echo -e "${YELLOW}Testing: Get API Key Usage${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/usage/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test revoking an API key
test_revoke_key() {
  echo -e "${YELLOW}Testing: Revoke API Key${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/key/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Main execution
print_header

# Initial state
echo -e "${BLUE}Initial API key status:${NC}"
test_get_key_status

# Generate a new API key
echo -e "${BLUE}Generating a new API key:${NC}"
test_generate_key

# Verify key status after generation
echo -e "${BLUE}API key status after generation:${NC}"
test_get_key_status

# Test an endpoint with the API key
echo -e "${BLUE}Testing an endpoint with the API key:${NC}"
test_endpoint_with_key

# Check key usage
echo -e "${BLUE}Checking API key usage:${NC}"
test_get_key_usage

# Revoke the API key
echo -e "${BLUE}Revoking the API key:${NC}"
test_revoke_key

# Final state
echo -e "${BLUE}Final API key status:${NC}"
test_get_key_status

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key.txt
