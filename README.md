# store-api0

The api0 store: every piece of persistent state the platform has — tenants and
their members, API keys, endpoint groups, the MCP tool registry, credits,
licences, downstream credentials, messaging channels and linked identities.

It sits behind the gateway and has no user authentication of its own. Nothing
in a browser should reach it.

## Who calls it

| Caller | How | For |
|---|---|---|
| `gateway-api0` | HTTP, `X-Internal-Secret` on sensitive routes | everything the dashboard, the SDK and MCP clients do |
| `whatsapp-bridge` | HTTP, `/api/internal/*` with `X-Internal-Secret` | channel lookup, sessions, linked identities, link codes |
| `ai-uploader` | HTTP, unauthenticated read of the public AI config | which model to format specs with |

The gateway talks to the store over HTTP only (`gateway-api0/src/store`).

## What runs

`cargo run` starts two servers in one process:

- **HTTP (actix-web)** on `server.http` in `config.yaml` — `127.0.0.1:5007` by
  default. This is the real interface. Every route is registered in
  [`src/http_server.rs`](src/http_server.rs), grouped and commented by purpose;
  read it rather than a list here, which would go stale.
- **gRPC (tonic, with gRPC-Web and reflection)** on `server.grpc` —
  `0.0.0.0:50057` by default. It serves the older `EndpointService` from
  [`endpoint_service.proto`](endpoint_service.proto) (API groups, user
  preferences, payments). Nothing in this tree calls it any more.

Both share one `EndpointStore` over Postgres. On every start the store applies
[`sql/schema.sql`](sql/schema.sql) and the row-level security policies in
[`sql/rls.sql`](sql/rls.sql).

If `ENDPOINTS_CONFIG_PATH` points at a YAML file, its API groups are loaded as
default endpoints at boot.

## Internal routes fail closed

Routes that read or write a tenant's credentials call
`require_internal_secret` ([`src/middleware/internal_secret.rs`](src/middleware/internal_secret.rs)).
An unset or empty `API0_INTERNAL_SECRET` denies every such request rather than
allowing it, so a misconfigured deployment is loud, not open. The gateway and
the bridge must be given the same value.

## Service keys

A first-party service that calls the store directly (cvenom) gets a key scoped
to the routes it needs, never `API0_INTERNAL_SECRET`. It sends
`X-Service-Key: <key>`; the store keeps only the key's SHA-256:

```bash
key=$(openssl rand -hex 32)            # give this to the service
printf %s "$key" | shasum -a 256       # this goes in API0_SERVICE_KEYS
```

```
API0_SERVICE_KEYS="cvenom:<sha256 hex>:credits.read,credits.write,email.send"
```

Several services are separated by `;`. The scopes, and the routes they open:

| Scope | Route |
|---|---|
| `credits.read` | `GET /api/user/credits/{tenant or email}` |
| `credits.write` | `POST /api/user/credits` |
| `email.send` | `POST /api/internal/email/send` |

Those routes still accept `X-Internal-Secret` from the gateway. A key outside
its scopes gets 403; a malformed entry is ignored and logged. See
[`src/middleware/service_key.rs`](src/middleware/service_key.rs).

## Configuration

`config.yaml` (or `CONFIG_PATH`) sets the two listen addresses and the YAML
formatter's host and port. Everything else is environment, see
[`.env.example`](.env.example):

| Variable | Required | Purpose |
|---|---|---|
| `DATABASE_URL` | yes — exits without it | Postgres connection string |
| `LOG_PATH_API0` | yes — exits without it | log file |
| `API0_INTERNAL_SECRET` | for any internal route | shared secret with gateway and bridge |
| `API0_SERVICE_KEYS` | for first-party services | scoped keys, stored as SHA-256 hashes — see below |
| `API0_ENCRYPTION_KEY` | for stored secrets | AES-256-GCM key sealing secrets at rest: downstream credentials, bot tokens, IdP client secrets, linked keys |
| `API0_LOG_LEVEL` | no | `trace` · `debug` · `info` (default) · `warn` · `error` |
| `FIREBASE_PROJECT_ID` | for admin routes | verifies Firebase JWTs |
| `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `STRIPE_AUTOMATIC_TAX` | for payments | credits and licence checkout |
| `LICENSE_SIGNING_KEY`, `LICENSE_SITE_URL` | for licences | signs desktop licences |
| `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASSWORD`, `EMAIL_FROM` | for email | invites, broadcasts |
| `AI_UPLOADER_URL` | for uploads | the spec formatter |
| `API0_FIRST_PARTY_HOSTS` | no | hosts treated as the platform's own |
| `ENDPOINTS_CONFIG_PATH` | no | default endpoint groups to load at boot |
| `TEST_DATABASE_URL` | tests only | database the integration tests use |

## Endpoint definitions

An endpoint group, as uploaded or as `ENDPOINTS_CONFIG_PATH` holds it:

```yaml
api_groups:
  - name: "Users"
    base: "https://api.example.com"
    endpoints:
      - id: "list_users"
        text: "List users"
        description: "Returns a page of users"
        verb: "GET"
        path: "/users"
        suggested_sentence: "Show me the first ten users"
        parameters:
          - name: "limit"
            description: "Number of users per page"
            required: false
            alternatives: ["page_size"]
```

`suggested_sentence` is not decoration: it goes into the MCP `initialize`
instructions the gateway hands to clients.

## Build and test

```bash
cargo build
cargo test            # integration tests need TEST_DATABASE_URL
```

The scripts in [`test/`](test) are manual curl and grpcurl probes against a
running store.
