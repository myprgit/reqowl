# reqowl

A notebook for APIs. Pages are Markdown files in a folder tree; any
` ```http ` block in a page is a request you can run, and its response
shows up right under it. Notes and requests live together, in plain files
that diff cleanly in git.

> Status: early development. The core and the CLI work; the desktop app
> (Tauri, Linux/macOS/Windows) is next.

## A page

````markdown
# Login flow

```http
# @name login
POST {{base}}/login
Content-Type: application/json

{"user": "{{user}}", "password": "{{password}}"}
```

```http
# @auth bearer {{login.response.body.$.token}}
GET {{base}}/orders?limit=2
```
````

Request syntax follows VS Code REST Client / JetBrains HTTP Client:
`METHOD URL`, headers, a blank line, then the body.

| Directive | Meaning |
|---|---|
| `# @name login` | Later blocks can use `{{login.response.body.$.path}}`, `.status`, `.headers.Name` |
| `# @auth bearer T` / `basic U P` / `apikey Header V` | Sets the auth header |
| `# @auth oauth2 api` | Gets a token for the OAuth2 profile `api` (see below) |
| `# @timeout 10s` | Default 30s |
| `# @no-redirect`, `# @insecure` | Don't follow redirects / skip TLS checks |

Built-ins: `{{$uuid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$randomInt 1 100}}`.
`GRAPHQL url` sends the body as a GraphQL query (variables JSON after a blank line).

### WebSocket

````markdown
```http
WEBSOCKET wss://{{host}}/socket
Authorization: Bearer {{token}}

{"subscribe": "orders"}
===
{"subscribe": "prices"}
```
````

Headers go on the handshake (`@auth` works as usual). The body holds the
messages to send once the connection opens, separated by lines that are
exactly `===`. `@timeout` limits the handshake only; `@insecure` accepts any
certificate. The CLI prints what it sends (`→`) and receives (`←`) for
`--ws-wait` seconds (default 3), then closes.

### OAuth2

`# @auth oauth2 api` takes its settings from variables named `api.<setting>`,
so they can differ per environment and secrets can live in the local file.
Quote the keys: unquoted dotted keys are TOML tables.

```toml
# env/dev.toml
"api.grant" = "client_credentials"     # or authorization_code, password
"api.token_url" = "https://auth.example.com/oauth/token"
"api.client_id" = "reqowl"
"api.scope" = "orders:read"

# env/dev.local.toml
"api.client_secret" = "..."
```

| Setting | |
|---|---|
| `grant` | `client_credentials`, `authorization_code` or `password` |
| `token_url`, `client_id` | Required |
| `client_secret`, `scope`, `audience` | Optional; sent in the form body |
| `auth_url` | Required for `authorization_code` |
| `username`, `password` | Required for `password` |
| `redirect_port` | `authorization_code` only; default 0 (any free port) |

`authorization_code` uses PKCE and opens the sign-in page in your browser
(the CLI prints the URL too); the redirect URI is
`http://127.0.0.1:<port>/callback`, so register that with the provider, with
a fixed `redirect_port` if it needs an exact match. Tokens are kept in memory
until shortly before they expire, then renewed with the refresh token if
there is one.

## Workspace

```
my-api/
  reqowl.toml          # name, default_env, [vars] defaults
  env/dev.toml         # environment variables
  env/dev.local.toml   # secrets; git-ignored
  Auth.md
  Auth/Login.md
```

Variable lookup order: page front matter, `env/<name>.local.toml`,
`env/<name>.toml`, `[vars]` in `reqowl.toml`.

## CLI

```sh
python3 scripts/demo-server.py &          # demo API on :8787
cargo run -p reqowl-cli -- run examples/demo/Auth/Login.md
cargo run -p reqowl-cli -- run examples/demo/Auth/Login.md --block login -i
cargo run -p reqowl-cli -- blocks examples/demo/Auth/Login.md
cargo run -p reqowl-cli -- envs examples/demo
```

Exit status: 0 if every block ran and got a response below 400; 1 if a block
failed or got 4xx/5xx; 2 if the page or workspace could not be loaded
(missing file, unknown environment).

## License

Apache-2.0
