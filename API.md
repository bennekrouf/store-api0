# API Endpoints Documentation

## Overview

This document provides a comprehensive list of all endpoints exposed by the api0 Store API service. The API contains multiple endpoint categories for managing API groups, user preferences, API keys, and credit balances.

## Base URLs

- **HTTP Server**: `http://127.0.0.1:9090`
- **gRPC Server**: `127.0.0.1:50055`

## Authentication

Most endpoints require authentication using an API key. Include the API key in the `X-API-Key` header:

```
X-API-Key: sk_live_yourApiKeyHere
```

## API Groups Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/groups/{email}` | GET | Get all API groups for a user with preferences applied |
| `/api/upload` | POST | Upload API groups configuration file (YAML/JSON) |
| `/api/group` | POST | Add a new API group |
| `/api/group` | PUT | Update an existing API group |
| `/api/groups/{email}/{group_id}` | DELETE | Delete an API group |

## User Preferences Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/user/preferences/{email}` | GET | Get user preferences |
| `/api/user/preferences` | POST | Update user preferences |
| `/api/user/preferences/{email}` | DELETE | Reset user preferences |

## API Key Management Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/user/keys/{email}` | GET | Get status of all API keys for a user |
| `/api/user/keys` | POST | Generate a new API key for a user |
| `/api/user/keys/{email}/{key_id}` | DELETE | Revoke a specific API key |
| `/api/user/keys/{email}` | DELETE | Revoke all API keys for a user |
| `/api/key/usage/{email}/{key_id}` | GET | Get usage statistics for a specific API key |
| `/api/key/validate` | POST | Validate an API key |
| `/api/key/usage` | POST | Record API key usage |

## Credit Balance Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/user/credits/{email}` | GET | Get credit balance for a user |
| `/api/user/credits` | POST | Update credit balance for a user |

## System Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/health` | GET | Health check endpoint |

## External Services

The backend connects to a YAML formatter service:
- **Formatter Service**: `http://localhost:6001/format-yaml`

## gRPC Services

The backend also exposes the following gRPC services:

| Service | Method | Description |
|---------|--------|-------------|
| `endpoint.EndpointService` | `GetApiGroups` | Stream API groups for a user |
| `endpoint.EndpointService` | `UploadApiGroups` | Upload API groups configuration |
| `endpoint.EndpointService` | `GetUserPreferences` | Get user preferences |
| `endpoint.EndpointService` | `UpdateUserPreferences` | Update user preferences |
| `endpoint.EndpointService` | `ResetUserPreferences` | Reset user preferences |

## Detailed Endpoint Specifications

### API Groups Endpoints

#### GET `/api/groups/{email}`
- **Purpose**: Retrieves all API groups for a user with preferences applied
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: JSON containing API groups and their endpoints

#### POST `/api/upload`
- **Purpose**: Upload an API groups configuration file
- **Request Body**: 
  - `email`: User's email address 
  - `file_content`: Base64-encoded file content
  - `file_name`: Name of the file
- **Returns**: Success status and count of imported groups/endpoints

#### POST `/api/group`
- **Purpose**: Add a new API group
- **Request Body**: 
  - `email`: User's email address
  - `api_group`: Group object with endpoints
- **Returns**: Success status and group ID

#### PUT `/api/group`
- **Purpose**: Update an existing API group
- **Request Body**: 
  - `email`: User's email address
  - `group_id`: ID of the group to update
  - `api_group`: Updated group object with endpoints
- **Returns**: Success status and group ID

#### DELETE `/api/groups/{email}/{group_id}`
- **Purpose**: Delete an API group
- **URL Parameters**: 
  - `email`: User's email address
  - `group_id`: ID of the group to delete
- **Returns**: Success status

### User Preferences Endpoints

#### GET `/api/user/preferences/{email}`
- **Purpose**: Get user preferences
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: User preferences including hidden default endpoints

#### POST `/api/user/preferences`
- **Purpose**: Update user preferences
- **Request Body**: 
  - `email`: User's email address
  - `action`: Action to perform ("hide_default" or "show_default")
  - `endpoint_id`: ID of the endpoint to hide/show
- **Returns**: Success status

#### DELETE `/api/user/preferences/{email}`
- **Purpose**: Reset user preferences
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: Success status

### API Key Management Endpoints

#### GET `/api/user/keys/{email}`
- **Purpose**: Get status of all API keys for a user
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: List of API keys with usage statistics

#### POST `/api/user/keys`
- **Purpose**: Generate a new API key for a user
- **Request Body**: 
  - `email`: User's email address
  - `key_name`: Name for the API key
- **Returns**: The generated API key (shown only once)

#### DELETE `/api/user/keys/{email}/{key_id}`
- **Purpose**: Revoke a specific API key
- **URL Parameters**: 
  - `email`: User's email address
  - `key_id`: ID of the key to revoke
- **Returns**: Success status

#### DELETE `/api/user/keys/{email}`
- **Purpose**: Revoke all API keys for a user
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: Success status and count of revoked keys

#### GET `/api/key/usage/{email}/{key_id}`
- **Purpose**: Get usage statistics for a specific API key
- **URL Parameters**: 
  - `email`: User's email address
  - `key_id`: ID of the key
- **Returns**: Key usage statistics

#### POST `/api/key/validate`
- **Purpose**: Validate an API key
- **Request Body**: 
  - `api_key`: API key to validate
- **Returns**: Validation status and key owner email

#### POST `/api/key/usage`
- **Purpose**: Record API key usage
- **Request Body**: 
  - `key_id`: ID of the key to record usage for
- **Returns**: Success status

### Credit Balance Endpoints

#### GET `/api/user/credits/{email}`
- **Purpose**: Get credit balance for a user
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: Credit balance

#### POST `/api/user/credits`
- **Purpose**: Update credit balance for a user
- **Request Body**: 
  - `email`: User's email address
  - `amount`: Amount to add to balance (can be negative)
- **Returns**: New balance

### System Endpoints

#### GET `/api/health`
- **Purpose**: Health check endpoint
- **Returns**: Service status information
