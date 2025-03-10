#!/bin/bash

# Configuration
HOST="0.0.0.0:50055" # Match your server address

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to test getting endpoints for an email
test_get_endpoints() {
  local email="$1"
  local description="$2"

  echo -e "${BLUE}Testing: $description${NC}"
  echo "Email: $email"
  echo "-----------------"

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
    "email": "$email"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD"
  echo "-----------------"

  response=$(grpcurl -plaintext \
    -d "$REQUEST_PAYLOAD" \
    $HOST \
    endpoint.EndpointService/GetEndpoints 2>&1)

  if [ $? -eq 0 ]; then
    echo -e "${GREEN}Success:${NC}"
    echo "$response"
  else
    echo -e "${RED}Error:${NC}"
    echo "$response"
  fi
  echo "-----------------"
  echo
}

# Test cases
echo "Testing endpoint service..."

# Test with different email addresses
# test_get_endpoints "user@example.com" "Get endpoints for standard user"
# test_get_endpoints "new.user@company.com" "Get endpoints for new user"
test_get_endpoints "mohamed.bennekrouf@gmail.com" "Get endpoints for admin"

# List available services (for verification)
echo "Checking available services:"
echo "-----------------"
grpcurl -plaintext $HOST list
echo

# Show service description
echo "Service description:"
echo "-----------------"
grpcurl -plaintext $HOST describe endpoint.EndpointService
echo

echo "All tests completed."
