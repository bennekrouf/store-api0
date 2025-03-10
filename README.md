# API Store

A Rust service for managing API endpoints with parameter definitions, alternatives, and HTTP verbs.

## Overview

API Store is a gRPC service written in Rust that allows you to manage and query API endpoint definitions. It supports storing default endpoints and user-specific endpoint configurations.

## Features

- Store API endpoint definitions with parameters
- Support for HTTP verbs (GET, POST, PUT, DELETE, etc.)
- Parameter definitions with optional descriptions and alternatives
- User-specific endpoint configurations
- YAML and JSON import/export

## Data Structure

Each endpoint consists of:

- `id`: Unique identifier
- `text`: Display text or short description
- `description`: Optional longer description (defaults to empty string)
- `verb`: HTTP verb (GET, POST, PUT, DELETE, etc. - defaults to "GET")
- `parameters`: List of parameters (can be empty)

Each parameter consists of:

- `name`: Parameter name
- `description`: Optional parameter description (defaults to empty string)
- `required`: Whether the parameter is required (defaults to false)
- `alternatives`: List of alternative names for the parameter (defaults to empty list)

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

## Usage

### Running the Server

```bash
cargo run
```

The server will start on port 50055 by default.

### Testing

Use the provided test scripts in the `test` directory:

```bash
cd test
./query.sh  # Fetch endpoints for a user
./upload.sh # Upload a new endpoints file
```

## API

The service exposes two gRPC endpoints:

1. `GetEndpoints`: Fetch endpoints for a user
2. `UploadEndpoints`: Upload a new endpoints configuration file

## License

MIT
