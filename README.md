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
| `# @timeout 10s` | Default 30s |
| `# @no-redirect`, `# @insecure` | Don't follow redirects / skip TLS checks |

Built-ins: `{{$uuid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$randomInt 1 100}}`.
`GRAPHQL url` sends the body as a GraphQL query (variables JSON after a blank line).

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
