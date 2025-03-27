# API Key Management

## Overview

API Store now includes a comprehensive API key management system that allows users to generate, manage, and track usage of API keys. These API keys can be used to authenticate API requests without requiring user login sessions.

## Features

- **API Key Generation**: Users can generate secure API keys with custom names
- **API Key Authentication**: All API endpoints can be authenticated using API keys
- **Usage Tracking**: Track API key usage, including count and last used time
- **Key Revocation**: Users can revoke their API keys at any time
- **Security**: API keys are securely hashed using SHA-256 before storage
- **Prefix Display**: Only key prefixes are stored and displayed for reference

## API Endpoints

### Get API Key Status

```
GET /api/user/key/{email}
```

Returns the current status of a user's API key, including metadata like usage count and last used time.

**Example Response:**
```json
{
  "success": true,
  "keyPreference": {
    "hasKey": true,
    "generatedAt": "2023-05-15T14:30:00Z",
    "lastUsed": "2023-05-16T09:12:43Z",
    "usageCount": 127,
    "keyName": "Production Key",
    "keyPrefix": "sk_abc123",
    "balance": 0
  }
}
```

### Generate API Key

```
POST /api/user/key
```

**Request Body:**
```json
{
  "email": "user@example.com",
  "key_name": "Production Key"
}
```

Generates a new API key for the user. If a key already exists, it will be replaced.

**Example Response:**
```json
{
  "success": true,
  "message": "API key generated successfully",
  "key": "sk_live_abcdef123456789...", // Full key, only returned once
  "keyPrefix": "sk_abc123"
}
```

### Revoke API Key

```
DELETE /api/user/key/{email}
```

Revokes a user's existing API key.

**Example Response:**
```json
{
  "success": true,
  "message": "API key revoked successfully"
}
```

### Get API Key Usage

```
GET /api/user/usage/{email}
```

Returns usage analytics for a user's API key.

**Example Response:**
```json
{
  "success": true,
  "usage": {
    "hasKey": true,
    "generatedAt": "2023-05-15T14:30:00Z",
    "lastUsed": "2023-05-16T09:12:43Z",
    "usageCount": 127,
    "keyName": "Production Key",
    "keyPrefix": "sk_abc123",
    "balance": 0
  }
}
```

## Using API Keys

To use an API key for authentication, include it in API requests using the `X-API-Key` HTTP header:

```
X-API-Key: sk_live_yourApiKeyHere
```

All API endpoints that require authentication will accept this header.

## Security Considerations

- API keys are securely hashed before storage
- Only key prefixes are stored in plain text for reference
- Full keys are only returned once upon creation
- Keys follow the format `sk_live_<random string>` for easy identification
- Rate limiting is applied to all API requests

## Implementation Details

The API key management system is implemented with the following components:

1. **Database Schema**: The `user_preferences` table is extended with additional fields for API key storage
2. **API Key Generation**: Uses cryptographically secure random generators
3. **Authentication Middleware**: All API requests are authenticated using the API key middleware
4. **Usage Tracking**: API key usage is automatically tracked on each request
5. **Security**: API keys are hashed using SHA-256 before storage

## Future Enhancements

- Enhanced analytics for API key usage
- Support for multiple API keys per user
- Role-based permissions for API keys
- Usage quotas and rate limiting per key
- Credit-based usage system
