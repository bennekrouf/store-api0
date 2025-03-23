#!/bin/bash

# Configuration
HOST="0.0.0.0:50055"                      # Match your server address
TEST_EMAIL="mohamed.bennekrouf@gmail.com" # The email we want to test

# Color codes for output
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing Endpoint Service - Get Endpoints by Email${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if grpcurl is installed
check_dependencies() {
  if ! command -v grpcurl &>/dev/null; then
    echo -e "${RED}Error: grpcurl is not installed${NC}"
    echo "Please install grpcurl to run this test:"
    echo "  - MacOS: brew install grpcurl"
    echo "  - Linux: Go to https://github.com/fullstorydev/grpcurl/releases"
    exit 1
  fi
}

# Function to test getting endpoints for an email with detailed output
test_get_endpoints_detailed() {
  local email="$1"

  echo -e "${CYAN}Testing Get Endpoints for:${NC} $email"
  echo -e "${YELLOW}Request Payload:${NC}"
  echo '{'
  echo "    \"email\": \"$email\""
  echo '}'
  echo

  # Create a temporary file for the response
  TEMP_FILE=$(mktemp)

  # Run the grpcurl command and capture output
  echo -e "${YELLOW}Sending request to:${NC} $HOST"
  grpcurl -plaintext \
    -d "{\"email\": \"$email\"}" \
    $HOST \
    endpoint.EndpointService/GetEndpoints >"$TEMP_FILE" 2>&1

  # Add a section to the test script to dump the raw response
  grpcurl -plaintext -d "{\"email\": \"$email\"}" $HOST endpoint.EndpointService/GetEndpoints >raw_response.txt
  echo "Raw response saved to raw_response.txt"

  local status=$?

  if [ $status -eq 0 ]; then
    echo -e "${GREEN}Request successful!${NC}"

    # Count the number of endpoints
    local endpoint_count=$(grep -c "\"id\":" "$TEMP_FILE")
    echo -e "${CYAN}Received${NC} $endpoint_count ${CYAN}endpoints${NC}"
    echo

    # Process and format output to be more readable
    echo -e "${CYAN}Endpoint Details:${NC}"

    # Extract and display each endpoint in a formatted way
    local current_id=""
    local current_text=""
    local current_desc=""
    local current_verb=""
    local current_base_url=""
    local current_path=""
    local param_count=0

    while IFS= read -r line; do
      # Extract fields
      if [[ $line =~ \"id\":\ \"([^\"]*)\" ]]; then
        if [ ! -z "$current_id" ]; then
          # Print previous endpoint details
          echo -e "${YELLOW}ID:${NC} $current_id"
          echo -e "${YELLOW}Text:${NC} $current_text"
          echo -e "${YELLOW}Description:${NC} $current_desc"
          echo -e "${YELLOW}HTTP Verb:${NC} $current_verb"
          echo -e "${YELLOW}Base URL:${NC} $current_base_url"
          echo -e "${YELLOW}Path:${NC} $current_path"
          echo -e "${YELLOW}Parameters:${NC} $param_count"
          echo -e "----------------------------------------------"
        fi
        current_id="${BASH_REMATCH[1]}"
        param_count=0
      elif [[ $line =~ \"text\":\ \"([^\"]*)\" ]]; then
        current_text="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"description\":\ \"([^\"]*)\" ]]; then
        current_desc="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"verb\":\ \"([^\"]*)\" ]]; then
        current_verb="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"base\":\ \"([^\"]*)\" ]]; then
        current_base_url="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"path\":\ \"([^\"]*)\" ]]; then
        current_path="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"name\":\ \"([^\"]*)\" ]]; then
        param_count=$((param_count + 1))
      fi
    done <"$TEMP_FILE"

    # Print the last endpoint
    if [ ! -z "$current_id" ]; then
      echo -e "${YELLOW}ID:${NC} $current_id"
      echo -e "${YELLOW}Text:${NC} $current_text"
      echo -e "${YELLOW}Description:${NC} $current_desc"
      echo -e "${YELLOW}HTTP Verb:${NC} $current_verb"
      echo -e "${YELLOW}Base URL:${NC} $current_base_url"
      echo -e "${YELLOW}Path:${NC} $current_path"
      echo -e "${YELLOW}Parameters:${NC} $param_count"
    fi

    echo
    echo -e "${GREEN}Endpoints retrieved successfully!${NC}"
  else
    echo -e "${RED}Error retrieving endpoints:${NC}"
    cat "$TEMP_FILE"
  fi

  # Clean up
  rm "$TEMP_FILE"
}

# Function to dump the raw response for debugging
dump_raw_response() {
  local email="$1"

  echo -e "${CYAN}Getting raw gRPC response for:${NC} $email"

  grpcurl -plaintext \
    -d "{\"email\": \"$email\"}" \
    $HOST \
    endpoint.EndpointService/GetEndpoints

  echo -e "${GREEN}Raw response dump complete${NC}"
  echo
}

# Main execution
print_header
check_dependencies

# Run the main detailed test
test_get_endpoints_detailed "$TEST_EMAIL"

# Uncomment to get raw response for debugging
# echo -e "${BLUE}=======================================================${NC}"
# echo -e "${BLUE}  Raw Response Dump (for debugging)${NC}"
# echo -e "${BLUE}=======================================================${NC}"
# dump_raw_response "$TEST_EMAIL"

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
