#!/bin/bash

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
KEY_ID="your-key-id-here"  # The API key ID to record usage for

# Record API key usage
echo "Recording API key usage for key ID: $KEY_ID"
curl -s -X POST -H "Content-Type: application/json" -d "{\"key_id\":\"$KEY_ID\"}" "$HOST/api/key/usage" | jq .
