#!/bin/bash
# test/test_user_preferences.sh

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
  echo -e "${BLUE}  Testing User Preferences API${NC}"
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

# Function to test getting user preferences
test_get_preferences() {
  echo -e "${YELLOW}Testing: Get User Preferences${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/preferences/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test updating user preferences
test_update_preferences() {
  local action=$1
  local endpoint_id=$2
  
  echo -e "${YELLOW}Testing: Update User Preferences ($action)${NC}"
  echo "Email: $TEST_EMAIL, Endpoint ID: $endpoint_id"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "action": "$action",
  "endpoint_id": "$endpoint_id"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/preferences")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test resetting user preferences
test_reset_preferences() {
  echo -e "${YELLOW}Testing: Reset User Preferences${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/preferences/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test getting API groups with preferences applied
test_get_api_groups() {
  echo -e "${YELLOW}Testing: Get API Groups with Preferences Applied${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/groups/$TEST_EMAIL")
  
  echo "Response (truncated):"
  echo "$response" | jq '.success, .message, (.api_groups | length) as $len | "Number of groups: \($len)"'
  echo "-----------------"
  echo
}

# Main execution
print_header
check_dependencies

# Initial state
echo -e "${BLUE}Initial state:${NC}"
test_get_preferences

# Get default endpoints
response=$(curl -s -X GET "$HOST/api/groups/$TEST_EMAIL")
default_endpoint_id=$(echo "$response" | jq -r '.api_groups[0].endpoints[0].id')

if [ -z "$default_endpoint_id" ] || [ "$default_endpoint_id" == "null" ]; then
  echo -e "${RED}No endpoints found for testing!${NC}"
  exit 1
fi

echo -e "${GREEN}Found endpoint ID for testing: $default_endpoint_id${NC}"
echo

# Hide a default endpoint
echo -e "${BLUE}Hiding a default endpoint:${NC}"
test_update_preferences "hide_default" "$default_endpoint_id"

# Get user preferences to verify
echo -e "${BLUE}Verifying preferences after hiding:${NC}"
test_get_preferences

# Test get API groups to see if hiding worked
echo -e "${BLUE}Verifying API groups after hiding:${NC}"
test_get_api_groups

# Show the default endpoint again
echo -e "${BLUE}Showing the default endpoint again:${NC}"
test_update_preferences "show_default" "$default_endpoint_id"

# Get user preferences to verify
echo -e "${BLUE}Verifying preferences after showing:${NC}"
test_get_preferences

# Reset preferences
echo -e "${BLUE}Resetting all preferences:${NC}"
test_reset_preferences

# Final state
echo -e "${BLUE}Final state:${NC}"
test_get_preferences

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
