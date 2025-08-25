# API Store

A Rust service for managing API endpoints with parameter definitions, alternatives, and HTTP verbs.

## Overview

API Store is a gRPC service written in Rust that allows you to manage and query API endpoint definitions. It supports storing default endpoints and user-specific endpoint configurations.

## Features

- Store API endpoint definitions with parameters
- Support for HTTP verbs (GET, POST, PUT, DELETE, etc.)
- Parameter definitions with optional descriptions and alternatives
- User-specific endpoint configurations
- Default endpoints management with user preferences
- YAML and JSON import/export

## Data Structure

Each endpoint consists of:

- `id`: Unique identifier
- `text`: Display text or short description
- `description`: Optional longer description (defaults to empty string)
- `verb`: HTTP verb (GET, POST, PUT, DELETE, etc. - defaults to "GET")
- `is_default`: Whether this is a default endpoint (cannot be modified/deleted)
- `parameters`: List of parameters (can be empty)

Each parameter consists of:

- `name`: Parameter name
- `description`: Optional parameter description (defaults to empty string)
- `required`: Whether the parameter is required (defaults to false)
- `alternatives`: List of alternative names for the parameter (defaults to empty list)

User preferences consist of:

- `email`: User's email address
- `hidden_defaults`: List of default endpoint IDs that the user has chosen to hide

## Configuration

Endpoints can be defined in YAML or JSON. Here's an example:

```yaml
endpoints:
  - id: "get_users"
    text: "Get users"
    # Description is optional
    verb: "GET"  # Optional, defaults to GET
    parameters:
      - name: "page"
        # No required field - defaults to false
        # No description - defaults to empty string
      - name: "limit"
        description: "Number of users per page"
      - name: "sort_by"
        description: "Field to sort by"
        alternatives:
          - "order_by"
          - "sort"

  - id: "create_user"
    text: "Create a new user"
    description: "Creates a new user in the system"
    verb: "POST"
    parameters:
      - name: "username"
        description: "User's username"
        required: true
        alternatives:
          - "user_name"
          - "login"
```

## Default Endpoints Management

API Store includes support for default endpoints that cannot be modified or deleted by users. Users can, however, choose to hide specific default endpoints from their view.

### User Preferences API

The following API endpoints are available for managing user preferences:

#### Get User Preferences

```
GET /api/user/preferences/:email
```

Returns user preferences including hidden default endpoints.

Response:
```json
{
  "success": true,
  "preferences": {
    "email": "user@example.com",
    "hidden_defaults": ["endpoint_id_1", "endpoint_id_2"]
  }
}
```

#### Update User Preferences

```
POST /api/user/preferences
```

Request body:
```json
{
  "email": "user@example.com",
  "action": "hide_default",  // or "show_default"
  "endpoint_id": "endpoint_id_1"
}
```

Response:
```json
{
  "success": true,
  "message": "User preferences successfully updated"
}
```

#### Reset User Preferences

```
DELETE /api/user/preferences/:email
```

Resets all user preferences to default.

Response:
```json
{
  "success": true,
  "message": "User preferences successfully reset"
}
```

## Usage

### Running the Server

```bash
cargo run
```

The server will start on port 50055 (gRPC) and 9090 (HTTP) by default.

### Testing

Use the provided test scripts in the `test` directory:

```bash
cd test
./query.sh                   # Fetch endpoints for a user
./upload.sh                  # Upload a new endpoints file
./test_user_preferences.sh   # Test user preferences functionality
```

## API

The service exposes both gRPC and HTTP endpoints:

### gRPC Endpoints:

1. `GetApiGroups`: Fetch endpoints for a user
2. `UploadApiGroups`: Upload a new endpoints configuration file
3. `GetUserPreferences`: Get user preferences
4. `UpdateUserPreferences`: Update user preferences
5. `ResetUserPreferences`: Reset user preferences

### HTTP Endpoints:

1. `GET /api/groups/:email`: Get API groups for a user
2. `POST /api/upload`: Upload a new endpoints configuration file
3. `POST /api/group`: Add a new API group
4. `PUT /api/group`: Update an API group
5. `DELETE /api/groups/:email/:group_id`: Delete an API group
6. `GET /api/user/preferences/:email`: Get user preferences
7. `POST /api/user/preferences`: Update user preferences
8. `DELETE /api/user/preferences/:email`: Reset user preferences


Convenient curl to add credit for test 

curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"email":"mohamed.bennekrouf@gmail.com","amount":500}' \
  http://127.0.0.1:9090/api/user/credits

## License

MIT
