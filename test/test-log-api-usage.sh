#!/bin/bash
# test/test_api_usage_logging.sh

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
TEST_EMAIL="test@example.com"  # Test email
KEY_NAME="Test API Key for Logging"  # Name for the API key

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing API Usage Logging${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if dependencies are installed
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
    echo "$API_KEY" > /tmp/api_key_logging.txt
    echo "$KEY_ID" > /tmp/key_id_logging.txt
    echo "$TEST_EMAIL" > /tmp/email_logging.txt
  else
    echo -e "${RED}Failed to extract API key and key ID from response${NC}"
    exit 1
  fi
  
  echo
}

# Function to test logging API usage
test_log_api_usage() {
  KEY_ID=$(cat /tmp/key_id_logging.txt)
  EMAIL=$(cat /tmp/email_logging.txt)
  
  echo -e "${YELLOW}Testing: Log API Usage${NC}"
  echo "Email: $EMAIL, Key ID: $KEY_ID"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "key_id": "$KEY_ID",
  "email": "$EMAIL",
  "endpoint_path": "/api/test/endpoint",
  "method": "GET",
  "status_code": 200,
  "response_time_ms": 132,
  "request_size_bytes": 1024,
  "response_size_bytes": 8192,
  "ip_address": "127.0.0.1",
  "user_agent": "Test Script/1.0"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/usage/log")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Extract log ID if available
  LOG_ID=$(echo "$response" | jq -r '.log_id')
  
  if [ "$LOG_ID" != "null" ]; then
    echo -e "${GREEN}Successfully logged API usage with ID: $LOG_ID${NC}"
  else
    echo -e "${RED}Failed to log API usage${NC}"
  fi
  
  echo
}

# Function to test getting API usage logs
test_get_api_usage_logs() {
  KEY_ID=$(cat /tmp/key_id_logging.txt)
  EMAIL=$(cat /tmp/email_logging.txt)
  
  echo -e "${YELLOW}Testing: Get API Usage Logs${NC}"
  echo "Email: $EMAIL, Key ID: $KEY_ID"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/usage/logs/$EMAIL/$KEY_ID?limit=10")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Check if logs were successfully retrieved
  SUCCESS=$(echo "$response" | jq -r '.success')
  LOG_COUNT=$(echo "$response" | jq -r '.count')
  
  if [ "$SUCCESS" = "true" ]; then
    echo -e "${GREEN}Successfully retrieved $LOG_COUNT API usage logs${NC}"
  else
    echo -e "${RED}Failed to retrieve API usage logs${NC}"
  fi
  
  echo
}

# Main execution
print_header
check_dependencies

# Generate a new API key
echo -e "${BLUE}Generating a new API key:${NC}"
test_generate_key

# Log API usage
echo -e "${BLUE}Logging API usage:${NC}"
test_log_api_usage

# Log multiple API usages for better testing
for i in {1..5}; do
  KEY_ID=$(cat /tmp/key_id_logging.txt)
  EMAIL=$(cat /tmp/email_logging.txt)
  
  # Create a varied payload
  REQUEST_PAYLOAD=$(cat <<EOF
{
  "key_id": "$KEY_ID",
  "email": "$EMAIL",
  "endpoint_path": "/api/test/endpoint$i",
  "method": "$([ $i % 2 -eq 0 ] && echo 'GET' || echo 'POST')",
  "status_code": $([ $i % 3 -eq 0 ] && echo '404' || echo '200'),
  "response_time_ms": $((50 + $i * 25)),
  "request_size_bytes": $((512 * $i)),
  "response_size_bytes": $((1024 * $i)),
  "ip_address": "127.0.0.$i",
  "user_agent": "Test Script/$i.0"
}
EOF
  )

  # Skip output for cleaner test
  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/usage/log")
  echo -e "${GREEN}Logged additional API usage ${i}/5${NC}"
done

# Get API usage logs
echo -e "${BLUE}Getting API usage logs:${NC}"
test_get_api_usage_logs

# Test cleanup
echo -e "${BLUE}Cleaning up test data...${NC}"
KEY_ID=$(cat /tmp/key_id_logging.txt)
EMAIL=$(cat /tmp/email_logging.txt)
curl -s -X DELETE "$HOST/api/user/keys/$EMAIL/$KEY_ID" > /dev/null
echo -e "${GREEN}Test completed and resources cleaned up${NC}"

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key_logging.txt /tmp/key_id_logging.txt /tmp/email_logging.txt
