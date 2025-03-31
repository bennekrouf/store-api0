#!/bin/bash
# test/test_multi_key.sh

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
TEST_EMAIL="test@example.com"  # Test email

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing Multiple API Key Management${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if curl is installed
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
  }
}

# Function to test getting API keys status
test_get_keys_status() {
  echo -e "${YELLOW}Testing: Get API Keys Status${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/keys/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test generating a new API key
test_generate_key() {
  local key_name=$1
  
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $key_name"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "key_name": "$key_name"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/keys")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Extract and save the API key and key_id for later tests
  API_KEY=$(echo "$response" | jq -r '.key')
  KEY_ID=$(echo "$response" | jq -r '.keyId')
  
  if [ "$API_KEY" != "null" ] && [ "$KEY_ID" != "null" ]; then
    echo -e "${GREEN}API Key: $API_KEY${NC}"
    echo -e "${GREEN}Key ID: $KEY_ID${NC}"
    
    # Save to a temp file for later tests
    echo "$API_KEY" > /tmp/api_key_${key_name}.txt
    echo "$KEY_ID" > /tmp/key_id_${key_name}.txt
  else
    echo -e "${RED}Failed to extract API key and key ID from response${NC}"
  fi
  
  echo
}

# Function to test validating an API key
test_validate_key() {
  local key_name=$1
  API_KEY=$(cat /tmp/api_key_${key_name}.txt)
  
  echo -e "${YELLOW}Testing: Validate API Key ($key_name)${NC}"
  echo "API Key: ${API_KEY:0:10}..."
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "api_key": "$API_KEY"
}
EOF
  )
  
  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/key/validate")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test getting credit balance
test_get_credit_balance() {
  echo -e "${YELLOW}Testing: Get Credit Balance${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/credits/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test updating credit balance
test_update_credit_balance() {
  local amount=$1
  
  echo -e "${YELLOW}Testing: Update Credit Balance${NC}"
  echo "Email: $TEST_EMAIL, Amount: $amount"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "amount": $amount
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/credits")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test revoking a specific API key
test_revoke_key() {
  local key_name=$1
  KEY_ID=$(cat /tmp/key_id_${key_name}.txt)
  
  echo -e "${YELLOW}Testing: Revoke Specific API Key ($key_name)${NC}"
  echo "Email: $TEST_EMAIL, Key ID: $KEY_ID"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/keys/$TEST_EMAIL/$KEY_ID")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test revoking all API keys
test_revoke_all_keys() {
  echo -e "${YELLOW}Testing: Revoke All API Keys${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/keys/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Main execution
print_header
check_dependencies

# Initial state
echo -e "${BLUE}Initial API key status:${NC}"
test_get_keys_status

# Add initial credit
echo -e "${BLUE}Adding initial credit:${NC}"
test_update_credit_balance 100

# Generate multiple API keys
echo -e "${BLUE}Generating first API key:${NC}"
test_generate_key "Production Key"

echo -e "${BLUE}Generating second API key:${NC}"
test_generate_key "Development Key"

echo -e "${BLUE}Generating third API key:${NC}"
test_generate_key "Testing Key"

# Verify key status after generation
echo -e "${BLUE}API keys status after generation:${NC}"
test_get_keys_status

# Test validation with each key
echo -e "${BLUE}Testing validation with first key:${NC}"
test_validate_key "Production Key"

echo -e "${BLUE}Testing validation with second key:${NC}"
test_validate_key "Development Key"

echo -e "${BLUE}Testing validation with third key:${NC}"
test_validate_key "Testing Key"

# Check credit balance
echo -e "${BLUE}Checking credit balance:${NC}"
test_get_credit_balance

# Revoke one specific key
echo -e "${BLUE}Revoking the second key:${NC}"
test_revoke_key "Development Key"

# Check keys status after revoking one
echo -e "${BLUE}API keys status after revoking one key:${NC}"
test_get_keys_status

# Check that credit balance is preserved
echo -e "${BLUE}Verifying credit balance is preserved after revoking key:${NC}"
test_get_credit_balance

# Revoke all remaining keys
echo -e "${BLUE}Revoking all remaining keys:${NC}"
test_revoke_all_keys

# Final state
echo -e "${BLUE}Final API key status:${NC}"
test_get_keys_status

# Final credit balance
echo -e "${BLUE}Final credit balance (should be preserved):${NC}"
test_get_credit_balance

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key_*.txt /tmp/key_id_*.txt
