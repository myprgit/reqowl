# Start here

This notebook is a folder of Markdown pages. Write notes the way you like;
any `http` code block is a request you can run.

Click ▶ in the margin next to a block, or put the cursor inside it and
press Ctrl+Enter. The response appears under the block. Click it to see
headers and the full body in the panel below.

```http
GET https://httpbin.org/get?hello=owl
Accept: application/json
```

## Variables

`{{base}}` comes from the environment picked at the top right. Environments
are files in `env/`, for example `env/dev.toml`. Put secrets in
`env/dev.local.toml`: it stays out of git, and its values are hidden in
history.

```http
GET {{base}}/health
```

## Chaining

Give a block a name, then use parts of its response in later blocks:

```http
# @name login
POST https://httpbin.org/anything
Content-Type: application/json

{"token": "abc123"}
```

```http
# @auth bearer {{login.response.body.$.json.token}}
GET https://httpbin.org/bearer
```

Running the second block on its own runs `login` first.
