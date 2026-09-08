#!/usr/bin/env bash
# Register the Azure DevOps work-item tools for one tenant.
#
# Everything Azure-specific lives in the four request-shaping fields below —
# content_type, body_template, static_headers, forward_identity. The gateway has
# no idea what Azure DevOps is. See gateway-api0/MCP_GATEWAY.md.
#
# Before running, set the tenant's credential once (PAT as HTTP Basic with an
# empty username), through the dashboard or:
#
#   curl -X PUT "$STORE_URL/api/user/downstream-auth" -H 'Content-Type: application/json' \
#     -d '{"email":"you@example.com","auth_mode":"header_injection",
#          "custom_headers":{"Authorization":"Basic '"$(printf ':%s' "$ADO_PAT" | base64)"'"}}'
#
# Usage:
#   STORE_URL=http://localhost:8080 \
#   API0_INTERNAL_SECRET=… TENANT_ID=… ADO_ORG=acme \
#   ./samples/azure-devops-mcp-tools.sh

set -euo pipefail

STORE_URL="${STORE_URL:?set STORE_URL}"
API0_INTERNAL_SECRET="${API0_INTERNAL_SECRET:?set API0_INTERNAL_SECRET}"
TENANT_ID="${TENANT_ID:?set TENANT_ID}"
ADO_ORG="${ADO_ORG:?set ADO_ORG, your Azure DevOps organisation}"
API_VERSION="${API_VERSION:-7.1}"

register() {
  local payload="$1" name
  name=$(printf '%s' "$payload" | sed -n 's/.*"tool_name" *: *"\([^"]*\)".*/\1/p')
  printf '→ %s\n' "$name"
  curl -sS -X POST "$STORE_URL/api/mcp-tools" \
    -H 'Content-Type: application/json' \
    -H "X-Internal-Secret: $API0_INTERNAL_SECRET" \
    -d "$payload" | sed 's/^/  /'
  printf '\n'
}

# ── create_work_item ─────────────────────────────────────────────────────────
# The optional ops disappear when the caller omits the field: an array element
# that cannot render completely is dropped, and Azure DevOps rejects an op with
# no value.
register "$(cat <<JSON
{
  "tenant_id": "$TENANT_ID",
  "tool_name": "create_work_item",
  "description": "Create a work item (Task, Bug, User Story) in an Azure DevOps project.",
  "backend_url": "https://dev.azure.com/$ADO_ORG/{project}/_apis/wit/workitems/\${type}?api-version=$API_VERSION",
  "http_verb": "POST",
  "content_type": "application/json-patch+json",
  "forward_identity": false,
  "cost_credits": 1,
  "body_template": $(cat <<'TPL' | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
[
  {"op":"add","path":"/fields/System.Title","value":"{title}"},
  {"op":"add","path":"/fields/System.Description","value":"{description}"},
  {"op":"add","path":"/fields/System.AreaPath","value":"{area_path}"},
  {"op":"add","path":"/fields/System.IterationPath","value":"{iteration_path}"},
  {"op":"add","path":"/fields/System.AssignedTo","value":"{assigned_to}"},
  {"op":"add","path":"/fields/System.Tags","value":"{tags}"}
]
TPL
),
  "input_schema": $(cat <<'SCHEMA' | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
{
  "type": "object",
  "properties": {
    "project": {"type": "string", "description": "Azure DevOps project name."},
    "type": {"type": "string", "enum": ["Task", "Bug", "User Story", "Feature", "Epic"]},
    "title": {"type": "string", "description": "Work item title."},
    "description": {"type": "string", "description": "HTML body of the work item."},
    "area_path": {"type": "string"},
    "iteration_path": {"type": "string"},
    "assigned_to": {"type": "string", "description": "Display name or email."},
    "tags": {"type": "string", "description": "Semicolon-separated tags."}
  },
  "required": ["project", "type", "title"]
}
SCHEMA
)
}
JSON
)"

# ── create_user_story ────────────────────────────────────────────────────────
# Same endpoint, a shape the model can fill in without knowing Azure's field
# names — including the parent link, which drops whole when parent_id is absent.
register "$(cat <<JSON
{
  "tenant_id": "$TENANT_ID",
  "tool_name": "create_user_story",
  "description": "Create a User Story in Azure DevOps, optionally under a parent Feature or Epic.",
  "backend_url": "https://dev.azure.com/$ADO_ORG/{project}/_apis/wit/workitems/\$User%20Story?api-version=$API_VERSION",
  "http_verb": "POST",
  "content_type": "application/json-patch+json",
  "forward_identity": false,
  "cost_credits": 1,
  "body_template": $(cat <<TPL | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
[
  {"op":"add","path":"/fields/System.Title","value":"{title}"},
  {"op":"add","path":"/fields/System.Description","value":"{description}"},
  {"op":"add","path":"/fields/Microsoft.VSTS.Common.AcceptanceCriteria","value":"{acceptance_criteria}"},
  {"op":"add","path":"/fields/Microsoft.VSTS.Scheduling.StoryPoints","value":"{story_points}"},
  {"op":"add","path":"/fields/System.IterationPath","value":"{iteration_path}"},
  {"op":"add","path":"/relations/-","value":{"rel":"System.LinkTypes.Hierarchy-Reverse","url":"https://dev.azure.com/$ADO_ORG/_apis/wit/workItems/{parent_id}"}}
]
TPL
),
  "input_schema": $(cat <<'SCHEMA' | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
{
  "type": "object",
  "properties": {
    "project": {"type": "string"},
    "title": {"type": "string", "description": "As a <role>, I want <goal>, so that <benefit>."},
    "description": {"type": "string"},
    "acceptance_criteria": {"type": "string"},
    "story_points": {"type": "number"},
    "iteration_path": {"type": "string"},
    "parent_id": {"type": "integer", "description": "Work item id of the parent Feature or Epic."}
  },
  "required": ["project", "title"]
}
SCHEMA
)
}
JSON
)"

# ── get_work_item ────────────────────────────────────────────────────────────
# A GET: arguments the URL does not consume become query params, so $expand
# reaches Azure DevOps as written.
register "$(cat <<JSON
{
  "tenant_id": "$TENANT_ID",
  "tool_name": "get_work_item",
  "description": "Fetch one Azure DevOps work item by id.",
  "backend_url": "https://dev.azure.com/$ADO_ORG/_apis/wit/workitems/{id}?api-version=$API_VERSION",
  "http_verb": "GET",
  "forward_identity": false,
  "cost_credits": 1,
  "input_schema": $(cat <<'SCHEMA' | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
{
  "type": "object",
  "properties": {
    "id": {"type": "integer", "description": "Work item id."},
    "$expand": {"type": "string", "enum": ["none", "relations", "fields", "links", "all"]}
  },
  "required": ["id"]
}
SCHEMA
)
}
JSON
)"

printf 'Done. The tenant\x27s MCP clients will see these on the next tools/list.\n'
